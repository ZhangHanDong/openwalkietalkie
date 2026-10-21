//! Grammar of the Usage face: SCROLL scrolls the provider list, ENTER emits
//! `Event::Refresh` and shows "updating…" for `UPDATING_MS`, CANCEL goes home.
//! `bar_tone` is the one bar-colour rule the renderer draws with.
use super::{global, Target, TouchAct, UiState, Update};
use crate::control::{Button, Control, Edge};
use crate::deck::model::Effects;
use crate::deck::msg::{Event, HostMsg, Tone};
use crate::ui::kit::list::Viewport;
use crate::ui::layout;

/// How long the "updating…" tag replaces the period after ENTER.
pub const UPDATING_MS: u32 = 1500;

#[derive(Clone, Copy)]
pub struct View {
    pub top_row: u8,
    /// When ENTER last asked for a refresh; 0 = never.
    pub refreshed_at_ms: u32,
    /// The "updating…" tag is showing (set on ENTER, cleared by `on_tick`).
    pub updating: bool,
}

impl View {
    pub const fn new() -> Self {
        Self {
            top_row: 0,
            refreshed_at_ms: 0,
            updating: false,
        }
    }
}

/// Bar colour: the host's `c` if it sent one, else by level.
pub const fn bar_tone(percent: u8, host: Option<Tone>) -> Tone {
    match host {
        Some(tone) => tone,
        None if percent < 50 => Tone::Blue,
        None if percent < 80 => Tone::Green,
        None if percent < 95 => Tone::Amber,
        None => Tone::Red,
    }
}

fn viewport(ui: &UiState) -> Viewport {
    Viewport::new(ui.usage.top_row, layout::USAGE_VISIBLE, ui.deck.usage.count.min(6))
}

fn scroll(ui: &mut UiState, rows: i32, out: &mut Update) {
    let mut view = viewport(ui);
    view.scroll(rows);
    out.dirty |= ui.usage.top_row != view.top;
    ui.usage.top_row = view.top;
}

/// Wheel detents and button presses while this face is current and no overlay
/// is open. `true` = consumed (a locally handled face swallows everything).
pub(super) fn on_control(
    ui: &mut UiState,
    _instance: u8,
    control: Control,
    at_ms: u32,
    out: &mut Update,
) -> bool {
    if ui.global.shift {
        return true; // No chords on this face; never fall through.
    }
    match control {
        Control::Wheel(wheel) => scroll(ui, super::selection_step(wheel), out),
        Control::Button(Button::Cancel, Edge::Press) => global::go_home(ui, at_ms, out),
        Control::Button(Button::Enter, Edge::Press) => {
            out.push_deck(Event::Refresh);
            ui.usage.refreshed_at_ms = at_ms;
            ui.usage.updating = true;
            out.dirty = true;
        }
        _ => {}
    }
    true
}

/// The touch target under (x, y), if any. Pure. Rows only scroll.
pub(super) fn hit(ui: &UiState, _instance: u8, x: i32, y: i32) -> Option<Target> {
    let slot = layout::usage_row_at(x, y)?;
    let item = viewport(ui).index_at(slot)?;
    Some(Target::list_row(layout::usage_row(slot), item as u16, layout::USAGE_PITCH))
}

/// One act of a touch that latched a target of this face.
pub(super) fn on_touch(
    ui: &mut UiState,
    _instance: u8,
    _target: Target,
    act: TouchAct,
    _at_ms: u32,
    out: &mut Update,
) {
    if let TouchAct::Scroll(rows) = act {
        scroll(ui, rows as i32, out);
    }
}

/// After `Deck::apply`: a shorter list keeps the viewport in range.
pub(super) fn on_host(
    ui: &mut UiState,
    msg: &HostMsg,
    _fx: &Effects,
    _at_ms: u32,
    _out: &mut Update,
) {
    if let HostMsg::Usage { .. } = msg {
        ui.usage.top_row = viewport(ui).top;
    }
}

pub(super) fn on_tick(ui: &mut UiState, now_ms: u32, out: &mut Update) {
    if ui.usage.updating
        && now_ms.wrapping_sub(ui.usage.refreshed_at_ms) >= UPDATING_MS
    {
        ui.usage.updating = false;
        out.dirty = true;
    }
}

/// The tag has to come down on time even without other input.
pub(super) fn animating(ui: &UiState) -> bool {
    ui.usage.updating
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_tone_follows_the_level_unless_the_host_says_otherwise() {
        assert_eq!(bar_tone(0, None), Tone::Blue);
        assert_eq!(bar_tone(49, None), Tone::Blue);
        assert_eq!(bar_tone(50, None), Tone::Green);
        assert_eq!(bar_tone(79, None), Tone::Green);
        assert_eq!(bar_tone(80, None), Tone::Amber);
        assert_eq!(bar_tone(94, None), Tone::Amber);
        assert_eq!(bar_tone(95, None), Tone::Red);
        assert_eq!(bar_tone(255, None), Tone::Red);
        assert_eq!(bar_tone(12, Some(Tone::Gray)), Tone::Gray);
    }
}
