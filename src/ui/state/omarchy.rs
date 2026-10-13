//! Grammar of the Omarchy Remote face: keys are forwarded as `Event::Key`
//! (press with `global::hold(.., Owner::HostKey{..})`, global emits the
//! release), SCROLL is act 2 plus an optimistic `selected_window`, touches
//! send `WsGo` / `WsWin`, plain CANCEL goes home.
//!
//! Rules (docs/agent-command-deck.md, "Omarchy"):
//! - LEFT / RIGHT / ENTER press -> `key act:1` with the SHIFT state and the
//!   instance id latched in the held owner; the release repeats them.
//!   SHIFT+CANCEL is forwarded the same way; plain CANCEL goes home.
//! - A wheel detent -> `key cw/ccw act:2` (no hold) and moves the local
//!   highlight one row, clamped, until the next `ws` push resets it to the
//!   host's selection.
//! - A tap on workspace chip n -> `WsGo(n + 1)`; a tap on a window row ->
//!   `WsWin(row)`. Rows scroll by drag (`ListRow`).
use super::{global, Owner, Source, Target, TouchAct, UiState, Update};
use crate::control::{Button, Control, Edge, Wheel};
use crate::deck::model::Effects;
use crate::deck::msg::{Event, HostMsg, KeyId};
use crate::ui::kit::list::Viewport;
use crate::ui::layout;

#[derive(Clone, Copy)]
pub struct View {
    /// Optimistic local highlight until the next `ws` push.
    pub selected_window: u8,
    pub top_row: u8,
    /// Workspace chip shown pressed by a latched touch.
    pub pressed_chip: Option<u8>,
    /// Window row shown pressed by a latched touch.
    pub pressed_row: Option<u8>,
}

impl View {
    pub const fn new() -> Self {
        Self {
            selected_window: 0,
            top_row: 0,
            pressed_chip: None,
            pressed_row: None,
        }
    }
}

fn window_count(ui: &UiState) -> u8 {
    ui.deck.workspaces.window_count.min(6)
}

fn viewport(ui: &UiState) -> Viewport {
    Viewport::new(ui.omarchy.top_row, layout::OMARCHY_VISIBLE, window_count(ui))
}

/// Move the highlight by `by` rows; the viewport follows. True when either
/// moved (a poked highlight is clamped here, which is a visible change).
fn move_selection(ui: &mut UiState, by: i32) -> bool {
    let before = (ui.omarchy.selected_window, ui.omarchy.top_row);
    let count = window_count(ui) as i32;
    let next = (ui.omarchy.selected_window as i32 + by).clamp(0, (count - 1).max(0));
    ui.omarchy.selected_window = next as u8;
    let mut view = viewport(ui);
    view.follow(next as u8);
    ui.omarchy.top_row = view.top;
    before != (ui.omarchy.selected_window, ui.omarchy.top_row)
}

fn forward(ui: &mut UiState, instance: u8, button: Button, key: KeyId, out: &mut Update) {
    let shift = ui.global.shift;
    out.push_deck(Event::Key {
        key,
        act: 1,
        shift,
        face: instance,
    });
    global::hold(
        ui,
        Source::Button(button),
        Owner::HostKey {
            face: instance,
            key,
            shift,
        },
    );
}

/// Wheel detents and button presses while this face is current and no overlay
/// is open. `true` = consumed (a locally handled face swallows everything).
pub(super) fn on_control(
    ui: &mut UiState,
    instance: u8,
    control: Control,
    at_ms: u32,
    out: &mut Update,
) -> bool {
    match control {
        Control::Wheel(wheel) => {
            let (key, by) = match wheel {
                Wheel::Cw => (KeyId::Cw, 1),
                Wheel::Ccw => (KeyId::Ccw, -1),
            };
            out.push_deck(Event::Key {
                key,
                act: 2,
                shift: false,
                face: instance,
            });
            out.dirty |= move_selection(ui, by);
        }
        Control::Button(Button::Cancel, Edge::Press) if !ui.global.shift => {
            global::go_home(ui, at_ms, out);
        }
        Control::Button(button, Edge::Press) => {
            let key = match button {
                Button::Enter => KeyId::Enter,
                Button::Cancel => KeyId::Cancel,
                Button::Left => KeyId::Left,
                Button::Right => KeyId::Right,
                // Global owns SHIFT and PTT; they never get here.
                Button::Shift | Button::PushToTalk => return true,
            };
            forward(ui, instance, button, key, out);
        }
        Control::Button(_, Edge::Release) | Control::Touch(_) => {}
    }
    true
}

/// The touch target under (x, y), if any. Pure. Chips are `Tap` targets whose
/// id is the workspace number; rows are `ListRow` targets whose id is the
/// window row.
pub(super) fn hit(ui: &UiState, _instance: u8, x: i32, y: i32) -> Option<Target> {
    if let Some(chip) = layout::omarchy_chip_at(x, y) {
        let count = ui.deck.workspaces.count.min(layout::OMARCHY_CHIPS);
        return (chip < count).then(|| Target::tap(layout::omarchy_chip(chip), chip as u16 + 1));
    }
    let slot = layout::omarchy_window_at(x, y)?;
    let row = viewport(ui).index_at(slot)?;
    Some(Target::list_row(
        layout::omarchy_window(slot),
        row as u16,
        layout::OMARCHY_PITCH,
    ))
}

/// One act of a touch that latched a target of this face.
pub(super) fn on_touch(
    ui: &mut UiState,
    _instance: u8,
    target: Target,
    act: TouchAct,
    _at_ms: u32,
    out: &mut Update,
) {
    let chip = target.kind == super::TargetKind::Tap;
    let id = target.id.min(255) as u8;
    match act {
        TouchAct::Down => {
            if chip {
                ui.omarchy.pressed_chip = Some(id.saturating_sub(1));
            } else {
                ui.omarchy.pressed_row = Some(id);
            }
        }
        TouchAct::Activate => {
            ui.omarchy.pressed_chip = None;
            ui.omarchy.pressed_row = None;
            if chip {
                out.push_deck(Event::WsGo(id));
            } else if id < window_count(ui) {
                out.push_deck(Event::WsWin(id));
                ui.omarchy.selected_window = id;
            }
        }
        TouchAct::Scroll(rows) => {
            let mut view = viewport(ui);
            if !view.scroll(rows as i32) {
                return;
            }
            ui.omarchy.top_row = view.top;
            ui.omarchy.pressed_row = None;
        }
        TouchAct::Cancel | TouchAct::Swipe { .. } => {
            ui.omarchy.pressed_chip = None;
            ui.omarchy.pressed_row = None;
        }
        TouchAct::Move { .. } | TouchAct::LongPress => return,
    }
    out.dirty = true;
}

/// After `Deck::apply`: a `ws` push ends the optimistic highlight.
pub(super) fn on_host(
    ui: &mut UiState,
    msg: &HostMsg,
    _fx: &Effects,
    _at_ms: u32,
    out: &mut Update,
) {
    if let HostMsg::Workspaces { .. } = msg {
        let count = window_count(ui);
        ui.omarchy.selected_window = ui.deck.workspaces.selected.min(count.saturating_sub(1));
        let mut view = viewport(ui);
        view.follow(ui.omarchy.selected_window);
        ui.omarchy.top_row = view.top;
        // A row that vanished under the finger cannot be tapped.
        if ui.omarchy.pressed_row.is_some_and(|row| row >= count) {
            ui.omarchy.pressed_row = None;
        }
        out.dirty = true;
    }
}

pub(super) fn on_tick(_ui: &mut UiState, _now_ms: u32, _out: &mut Update) {}

pub(super) fn animating(_ui: &UiState) -> bool {
    false
}

/// Global released a `HostKey` this face held (act 0 is already emitted).
/// Nothing on this face shows a held key, so there is nothing to drop.
pub(super) fn on_owner_released(
    _ui: &mut UiState,
    _face: u8,
    _key: crate::deck::msg::KeyId,
    _out: &mut Update,
) {
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::msg::Template;
    use crate::ui::state::TargetKind;

    fn on_omarchy() -> std::boxed::Box<UiState> {
        let mut ui = std::boxed::Box::new(UiState::new());
        ui.reset();
        ui.load_demo();
        ui.on_face(Template::Omarchy);
        ui
    }

    #[test]
    fn chips_and_rows_are_distinct_targets_bounded_by_the_host_data() {
        let ui = on_omarchy();
        let (x, y) = layout::omarchy_chip(1).center();
        let chip = hit(&ui, 2, x, y).unwrap();
        assert_eq!((chip.kind, chip.id), (TargetKind::Tap, 2));
        // The demo has four workspaces and three windows.
        let (x, y) = layout::omarchy_chip(4).center();
        assert_eq!(hit(&ui, 2, x, y), None);
        let (x, y) = layout::omarchy_window(2).center();
        let row = hit(&ui, 2, x, y).unwrap();
        assert_eq!((row.kind, row.id, row.pitch), (TargetKind::ListRow, 2, layout::OMARCHY_PITCH));
        let (x, y) = layout::omarchy_window(3).center();
        assert_eq!(hit(&ui, 2, x, y), None);
    }

    #[test]
    fn a_touch_scroll_moves_the_viewport_and_drops_the_pressed_look() {
        let mut ui = on_omarchy();
        ui.deck.workspaces.window_count = 6;
        let mut out = Update::new();
        let row = hit(&ui, 2, 240, layout::omarchy_window(0).center().1).unwrap();
        on_touch(&mut ui, 2, row, TouchAct::Down, 0, &mut out);
        assert_eq!(ui.omarchy.pressed_row, Some(0));
        on_touch(&mut ui, 2, row, TouchAct::Scroll(1), 0, &mut out);
        assert_eq!((ui.omarchy.top_row, ui.omarchy.pressed_row), (1, None));
        on_touch(&mut ui, 2, row, TouchAct::Scroll(9), 0, &mut out);
        assert_eq!(ui.omarchy.top_row, 2);
        on_touch(&mut ui, 2, row, TouchAct::Cancel, 0, &mut out);
        assert_eq!(out.deck_events().count(), 0);
    }
}
