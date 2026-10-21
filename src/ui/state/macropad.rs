//! Grammar of the Macropad faces: DIRECT BINDING, no cursor. A hardware
//! control with a binding presses its key (`Event::Macro` act 1 + `global::
//! hold(.., Owner::Macro{..})`; wheel bindings send act 2 and flash), touch
//! presses any present key on Down (`TargetKind::Press`). Never emits `key`.
//!
//! Rules (docs/agent-command-deck.md, "Macropad"):
//! - LEFT / RIGHT / ENTER, with or without SHIFT, press the present key whose
//!   `Binding` matches; the key shows pressed until global releases the owner
//!   (physical release or teardown). A chord without a key is swallowed.
//! - CW / CCW step their key (`act:2`) and flash it for `FLASH_MS`.
//! - Plain CANCEL goes home; CANCEL is never bindable.
//! - A touch Down on a present key presses it; the recogniser's release
//!   (Up, leave, Cancel) goes through global, which emits the act 0.
//! - The `down` mask belongs to one instance (`View.face`); pressing a key of
//!   another instance starts a fresh mask.
//! - A `macro` push that removes a held key tears the hold down (act 0 now,
//!   the physical release is orphaned).
//! - A key two sources press at once (finger + binding, two bindings) is one
//!   press: act 1 once, act 0 and the un-press with the last release.
//!
//! Editing on the deck (the key editor, `Overlay::MacroEdit`):
//! - Into the editor: a tap on a "+ assign" slot, SHIFT + a tap on any
//!   slot, or SHIFT+CANCEL for a cursor over the grid (`View::cursor`:
//!   SCROLL / LEFT / RIGHT move it, ENTER opens the slot, CANCEL leaves;
//!   bindings do not fire while the cursor is up).
//! - Inside: rows Label, Modifiers, Key, Save, and Remove key for a slot
//!   that had one. SCROLL selects a row. PTT dictates the label (a `name`
//!   session, like naming a face; the host's `name` text lands in the
//!   draft). ENTER on Modifiers or Key adjusts it with SCROLL (the modifier
//!   combos of `MOD_COMBOS`, the keys of `msg::KEYS`; LEFT/RIGHT jump ten
//!   keys; hard ends; ENTER or CANCEL ends), on Save commits, on Remove
//!   clears the slot. CANCEL discards. A hardware binding the host set is
//!   kept as it was.
//! - A commit updates this deck's set at once and sends `Event::MacroKey`;
//!   the host persists it and re-sends the whole `macro`, which is the
//!   same keys again.
use super::{global, global::Overlay, Owner, Source, Target, TouchAct, UiState, Update};
use crate::control::{Button, Control, Edge, Wheel};
use crate::deck::model::Effects;
use crate::deck::msg::{Binding, Event, HostMsg, Keystroke, MacroKey, KEYS, MOD_CMD, MOD_CTRL, MOD_OPT, MOD_SHIFT};
use crate::deck::text::Text;
use crate::ui::layout;

/// How long a wheel-stepped key shows pressed.
pub const FLASH_MS: u32 = 150;

/// Touch target ids: 0..9 press a key, `OPEN_ID` + n opens the editor on
/// slot n, `ROW_ID` + n is an editor row.
const OPEN_ID: u16 = 16;
const ROW_ID: u16 = 32;

/// The editor's rows, in order.
pub const ROW_LABEL: u8 = 0;
pub const ROW_MODS: u8 = 1;
pub const ROW_KEY: u8 = 2;
pub const ROW_SAVE: u8 = 3;
pub const ROW_REMOVE: u8 = 4;

/// The modifier combinations SCROLL steps through on the Modifiers row,
/// the common ones first.
pub const MOD_COMBOS: [u8; 16] = [
    0,
    MOD_CMD,
    MOD_CMD | MOD_SHIFT,
    MOD_CMD | MOD_OPT,
    MOD_CMD | MOD_CTRL,
    MOD_SHIFT,
    MOD_OPT,
    MOD_CTRL,
    MOD_OPT | MOD_SHIFT,
    MOD_CTRL | MOD_SHIFT,
    MOD_CTRL | MOD_OPT,
    MOD_CMD | MOD_OPT | MOD_SHIFT,
    MOD_CMD | MOD_CTRL | MOD_SHIFT,
    MOD_CMD | MOD_CTRL | MOD_OPT,
    MOD_CTRL | MOD_OPT | MOD_SHIFT,
    MOD_CMD | MOD_CTRL | MOD_OPT | MOD_SHIFT,
];
/// LEFT/RIGHT move this many keys while the Key row is adjusting.
pub const KEY_JUMP: u8 = 10;

/// One slot of one instance, drafted until Save or Remove sends it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Edit {
    pub face: u8,
    pub index: u8,
    /// `ROW_LABEL` .. `ROW_REMOVE`.
    pub row: u8,
    /// SCROLL is changing the selected row's value, not the row.
    pub adjusting: bool,
    pub label: Text<12>,
    /// The keystroke being composed; `key` 0 saves as none.
    pub keys: Keystroke,
    /// A hardware binding the host set, carried through unchanged.
    pub binding: Binding,
    /// The slot had a key when the editor opened: Remove is offered.
    pub present: bool,
    /// A touched row, for its pressed look.
    pub pressed: Option<u8>,
    /// Save was pressed without a label: the Label row says so.
    pub needs_label: bool,
}

impl Edit {
    pub const fn new() -> Self {
        Self {
            face: 0,
            index: 0,
            row: ROW_LABEL,
            adjusting: false,
            label: Text::new(),
            keys: Keystroke::NONE,
            binding: Binding::None,
            present: false,
            pressed: None,
            needs_label: false,
        }
    }
}

/// Rows the editor shows: Remove only for a slot that had a key.
pub const fn edit_rows(edit: &Edit) -> u8 {
    if edit.present {
        5
    } else {
        4
    }
}

#[derive(Clone, Copy)]
pub struct View {
    /// Face id the `down` mask belongs to.
    pub face: u8,
    /// Keys shown pressed (held or flashing), bit n = key n.
    pub down: u16,
    /// The grid cursor (SHIFT+CANCEL): the instance it is on and the slot.
    pub cursor: Option<(u8, u8)>,
    /// The key editor's draft; meaningful while `Overlay::MacroEdit` is up.
    pub edit: Edit,
    /// Keys held by a button or the finger.
    held: u16,
    /// Keys flashing after a wheel step, and when each flash ends.
    flashing: u16,
    flash_until_ms: [u32; 9],
}

impl View {
    pub const fn new() -> Self {
        Self {
            face: 0,
            down: 0,
            cursor: None,
            edit: Edit::new(),
            held: 0,
            flashing: 0,
            flash_until_ms: [0; 9],
        }
    }
}

/// Make the mask about `face`, forgetting another instance's keys (which
/// may still be sliding out of view).
fn select_face(ui: &mut UiState, face: u8, out: &mut Update) {
    if ui.macropad.face != face {
        out.dirty |= ui.macropad.down != 0;
        ui.macropad.face = face;
        ui.macropad.held = 0;
        ui.macropad.flashing = 0;
        ui.macropad.down = 0;
    }
}

fn refresh_down(ui: &mut UiState, out: &mut Update) {
    let down = ui.macropad.held | ui.macropad.flashing;
    out.dirty |= down != ui.macropad.down;
    ui.macropad.down = down;
}

/// The present key of `instance` bound to `binding`.
fn key_for(ui: &UiState, instance: u8, binding: Binding) -> Option<u8> {
    ui.deck
        .macro_set(instance)?
        .keys
        .iter()
        .position(|key| key.present && key.binding == binding)
        .map(|index| index as u8)
}

fn key_present(ui: &UiState, instance: u8, index: u8) -> bool {
    ui.deck
        .macro_set(instance)
        .and_then(|set| set.keys.get(index as usize))
        .is_some_and(|key| key.present)
}

/// Press key `index` of `instance` on behalf of `source`. A key another
/// source already holds is pressed once: the newcomer joins the hold (no
/// second act 1) and the key stays down until the last of them is up.
fn press(ui: &mut UiState, instance: u8, index: u8, source: Source, out: &mut Update) {
    select_face(ui, instance, out);
    let owner = Owner::Macro {
        face: instance,
        index,
    };
    if !global::holds(ui, owner) {
        out.push_deck(Event::Macro {
            face: instance,
            index,
            act: 1,
        });
    }
    global::hold(ui, source, owner);
    ui.macropad.held |= 1 << (index & 15);
    refresh_down(ui, out);
}

/// Some source still holds key `index` of `instance`.
fn owned(ui: &UiState, instance: u8, index: u8) -> bool {
    global::holds(
        ui,
        Owner::Macro {
            face: instance,
            index,
        },
    )
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
    let shift = ui.global.shift;
    if let Some((face, slot)) = ui.macropad.cursor.filter(|(face, _)| *face == instance) {
        // The cursor is up: the controls move it, nothing presses a key.
        let step = match control {
            Control::Wheel(wheel) => super::selection_step(wheel),
            Control::Button(Button::Right, Edge::Press) => 1,
            Control::Button(Button::Left, Edge::Press) => -1,
            Control::Button(Button::Enter, Edge::Press) if !shift => {
                open_editor(ui, face, slot, at_ms, out);
                return true;
            }
            Control::Button(Button::Cancel, Edge::Press) if !shift => {
                ui.macropad.cursor = None;
                out.dirty = true;
                return true;
            }
            _ => return true,
        };
        let next = (slot as i32 + step).clamp(0, 8) as u8;
        if next != slot {
            ui.macropad.cursor = Some((face, next));
            out.dirty = true;
        }
        return true;
    }
    match control {
        Control::Button(Button::Cancel, Edge::Press) if shift => {
            ui.macropad.cursor = Some((instance, 4));
            out.dirty = true;
        }
        Control::Wheel(wheel) => {
            let binding = match wheel {
                Wheel::Cw => Binding::Cw,
                Wheel::Ccw => Binding::Ccw,
            };
            if let Some(index) = key_for(ui, instance, binding) {
                select_face(ui, instance, out);
                out.push_deck(Event::Macro {
                    face: instance,
                    index,
                    act: 2,
                });
                ui.macropad.flashing |= 1 << (index & 15);
                ui.macropad.flash_until_ms[(index % 9) as usize] = at_ms.wrapping_add(FLASH_MS);
                refresh_down(ui, out);
            }
        }
        Control::Button(Button::Cancel, Edge::Press) if !shift => {
            global::go_home(ui, at_ms, out);
        }
        Control::Button(button, Edge::Press) => {
            let binding = match (button, shift) {
                (Button::Left, false) => Binding::Left,
                (Button::Right, false) => Binding::Right,
                (Button::Enter, false) => Binding::Enter,
                (Button::Left, true) => Binding::ShiftLeft,
                (Button::Right, true) => Binding::ShiftRight,
                (Button::Enter, true) => Binding::ShiftEnter,
                _ => return true,
            };
            if let Some(index) = key_for(ui, instance, binding) {
                press(ui, instance, index, Source::Button(button), out);
            }
        }
        Control::Button(_, Edge::Release) | Control::Touch(_) => {}
    }
    true
}

/// The touch target under (x, y), if any. Pure. A present key is pressed;
/// a "+ assign" slot, any slot under SHIFT, and any slot while the cursor
/// is up open the editor instead.
pub(super) fn hit(ui: &UiState, instance: u8, x: i32, y: i32) -> Option<Target> {
    let index = layout::macro_key_at(x, y)?;
    let edits = ui.global.shift
        || ui.macropad.cursor.is_some_and(|(face, _)| face == instance)
        || !key_present(ui, instance, index);
    Some(if edits {
        Target::tap(layout::macro_key(index), OPEN_ID + index as u16)
    } else {
        Target::press(layout::macro_key(index), index as u16)
    })
}

/// The editor's rows.
pub(super) fn edit_hit(ui: &UiState, x: i32, y: i32) -> Option<Target> {
    let row = layout::macro_edit_row_at(x, y)?;
    (row < edit_rows(&ui.macropad.edit)).then(|| Target::tap(layout::macro_edit_row(row), ROW_ID + row as u16))
}

/// One act of a touch that latched a target of this face: pressed on Down;
/// the release arrives through `on_owner_released` before the Cancel.
pub(super) fn on_touch(
    ui: &mut UiState,
    instance: u8,
    target: Target,
    act: TouchAct,
    at_ms: u32,
    out: &mut Update,
) {
    if target.id >= ROW_ID {
        let row = (target.id - ROW_ID).min(ROW_REMOVE as u16) as u8;
        match act {
            TouchAct::Down => ui.macropad.edit.pressed = Some(row),
            TouchAct::Activate => {
                ui.macropad.edit.pressed = None;
                if ui.global.overlay == Overlay::MacroEdit && row < edit_rows(&ui.macropad.edit) {
                    stop_adjusting(ui, out);
                    ui.macropad.edit.row = row;
                    activate_row(ui, at_ms, out);
                }
            }
            TouchAct::Cancel | TouchAct::Swipe { .. } => ui.macropad.edit.pressed = None,
            _ => return,
        }
        out.dirty = true;
        return;
    }
    if target.id >= OPEN_ID {
        if act == TouchAct::Activate {
            open_editor(ui, instance, (target.id - OPEN_ID).min(8) as u8, at_ms, out);
        }
        return;
    }
    let index = target.id.min(8) as u8;
    match act {
        TouchAct::Down if key_present(ui, instance, index) => {
            press(ui, instance, index, Source::Touch, out);
        }
        TouchAct::Cancel | TouchAct::Activate | TouchAct::Swipe { .. } => {
            // Global has released the owner already; the finger's hold is over
            // even if it never went through global (a Down on an absent key).
            // A button still holding the key keeps it down.
            if global::held(ui, Source::Touch).is_none()
                && ui.macropad.face == instance
                && !owned(ui, instance, index)
            {
                ui.macropad.held &= !(1 << index);
                refresh_down(ui, out);
            }
        }
        _ => {}
    }
}

/// After `Deck::apply`: a `macro` push that took away a held key ends its hold.
pub(super) fn on_host(
    ui: &mut UiState,
    msg: &HostMsg,
    _fx: &Effects,
    _at_ms: u32,
    out: &mut Update,
) {
    if let HostMsg::Name { text } = msg {
        if ui.global.overlay == Overlay::MacroEdit {
            ui.macropad.edit.label.set(text.as_str());
            ui.macropad.edit.needs_label = false;
            out.dirty = true;
        }
        return;
    }
    let HostMsg::Macro { face, .. } = msg else {
        return;
    };
    if *face != ui.macropad.face {
        return;
    }
    let gone = (0..9u8)
        .filter(|index| ui.macropad.held & (1 << index) != 0)
        .any(|index| !key_present(ui, *face, index));
    if gone {
        global::release_owned(ui, out);
        ui.macropad.held = 0;
        refresh_down(ui, out);
    }
}

// ---- the key editor ----------------------------------------------------------------

/// Open the editor on slot `index` of `face`, drafting the key it holds.
fn open_editor(ui: &mut UiState, face: u8, index: u8, at_ms: u32, out: &mut Update) {
    let key = ui
        .deck
        .macro_set(face)
        .and_then(|set| set.keys.get(index as usize).copied())
        .filter(|key| key.present)
        .unwrap_or(MacroKey::EMPTY);
    ui.macropad.edit = Edit {
        face,
        index,
        label: key.label,
        keys: key.keys,
        binding: key.binding,
        present: key.present,
        ..Edit::new()
    };
    global::set_overlay(ui, Overlay::MacroEdit, at_ms, out);
    out.dirty = true;
}

fn stop_adjusting(ui: &mut UiState, out: &mut Update) {
    out.dirty |= ui.macropad.edit.adjusting;
    ui.macropad.edit.adjusting = false;
}

/// ENTER (or a tap) on the selected row.
fn activate_row(ui: &mut UiState, at_ms: u32, out: &mut Update) {
    match ui.macropad.edit.row {
        ROW_MODS | ROW_KEY => {
            ui.macropad.edit.adjusting = true;
            out.dirty = true;
        }
        ROW_SAVE => {
            if ui.macropad.edit.label.is_empty() {
                ui.macropad.edit.needs_label = true;
                out.dirty = true;
            } else {
                commit(ui, true, at_ms, out);
            }
        }
        ROW_REMOVE => commit(ui, false, at_ms, out),
        _ => {}
    }
}

/// Save or Remove: this deck's set changes now, the host is told, and the
/// editor closes.
fn commit(ui: &mut UiState, present: bool, at_ms: u32, out: &mut Update) {
    let edit = ui.macropad.edit;
    let key = if present {
        MacroKey {
            label: edit.label,
            binding: edit.binding,
            // Modifiers alone are no keystroke.
            keys: if edit.keys.is_some() { edit.keys } else { Keystroke::NONE },
            present: true,
        }
    } else {
        MacroKey::EMPTY
    };
    if let Some(set) = ui.deck.macro_set_mut(edit.face) {
        if let Some(slot) = set.keys.get_mut(edit.index as usize) {
            *slot = key;
        }
    }
    out.push_deck(Event::MacroKey { face: edit.face, index: edit.index });
    global::set_overlay(ui, Overlay::None, at_ms, out);
    out.dirty = true;
}

/// Every wheel detent and button press while the editor is open.
pub(super) fn on_edit_control(ui: &mut UiState, control: Control, at_ms: u32, out: &mut Update) -> bool {
    if ui.global.shift {
        return true;
    }
    let edit = ui.macropad.edit;
    match control {
        Control::Wheel(wheel) if edit.adjusting => {
            adjust(ui, super::selection_step(wheel), out);
        }
        Control::Button(button @ (Button::Left | Button::Right), Edge::Press) if edit.adjusting => {
            if edit.row == ROW_KEY {
                adjust(ui, if button == Button::Right { KEY_JUMP as i32 } else { -(KEY_JUMP as i32) }, out);
            }
        }
        Control::Wheel(wheel) => {
            let last = edit_rows(&edit) - 1;
            let next = if super::selection_step(wheel) > 0 { (edit.row + 1).min(last) } else { edit.row.saturating_sub(1) };
            if next != edit.row {
                ui.macropad.edit.row = next;
                out.dirty = true;
            }
        }
        Control::Button(Button::Cancel | Button::Enter, Edge::Press) if edit.adjusting => {
            stop_adjusting(ui, out);
        }
        Control::Button(Button::Enter, Edge::Press) => activate_row(ui, at_ms, out),
        Control::Button(Button::Cancel, Edge::Press) => {
            global::set_overlay(ui, Overlay::None, at_ms, out);
            out.dirty = true;
        }
        _ => {}
    }
    true
}

/// Step the adjusting row's value by `by` (hard ends): a modifier combo of
/// `MOD_COMBOS` on the Modifiers row, a key of `KEYS` (0 = none) on Key.
fn adjust(ui: &mut UiState, by: i32, out: &mut Update) {
    let edit = ui.macropad.edit;
    match edit.row {
        ROW_MODS => {
            let at = MOD_COMBOS.iter().position(|m| *m == edit.keys.mods).unwrap_or(0) as i32;
            let next = (at + by).clamp(0, MOD_COMBOS.len() as i32 - 1) as usize;
            out.dirty |= MOD_COMBOS[next] != edit.keys.mods;
            ui.macropad.edit.keys.mods = MOD_COMBOS[next];
        }
        ROW_KEY => {
            let next = (edit.keys.key as i32 + by).clamp(0, KEYS.len() as i32) as u8;
            out.dirty |= next != edit.keys.key;
            ui.macropad.edit.keys.key = next;
        }
        _ => {}
    }
}

/// Expire wheel flashes; drop a cursor whose instance is no longer current.
pub(super) fn on_tick(ui: &mut UiState, now_ms: u32, out: &mut Update) {
    if let Some((face, _)) = ui.macropad.cursor {
        let current = ui.current_face();
        if current.template != crate::deck::msg::Template::Macropad || current.id != face {
            ui.macropad.cursor = None;
            out.dirty = true;
        }
    }
    if ui.macropad.flashing == 0 {
        return;
    }
    for index in 0..9 {
        let bit = 1 << index;
        if ui.macropad.flashing & bit != 0
            && now_ms.wrapping_sub(ui.macropad.flash_until_ms[index]) < u32::MAX / 2
        {
            ui.macropad.flashing &= !bit;
        }
    }
    refresh_down(ui, out);
}

pub(super) fn animating(ui: &UiState) -> bool {
    ui.macropad.flashing != 0
}

/// Global released a `Macro` key (act 0 is already emitted): un-press it,
/// unless another source still holds it.
pub(super) fn on_owner_released(ui: &mut UiState, face: u8, index: u8, out: &mut Update) {
    if ui.macropad.face == face && !owned(ui, face, index) {
        ui.macropad.held &= !(1 << (index & 15));
        refresh_down(ui, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::demo::MACRO_FACE;
    use crate::deck::msg::Template;

    fn on_macropad() -> std::boxed::Box<UiState> {
        let mut ui = std::boxed::Box::new(UiState::new());
        ui.reset();
        ui.load_demo();
        ui.on_face(Template::Macropad);
        ui
    }

    #[test]
    fn a_touch_press_on_a_key_is_released_through_global() {
        let mut ui = on_macropad();
        let mut out = Update::new();
        let (x, y) = layout::macro_key(4).center();
        let key = hit(&ui, MACRO_FACE, x, y).unwrap();
        assert_eq!(key.kind, super::super::TargetKind::Press);
        on_touch(&mut ui, MACRO_FACE, key, TouchAct::Down, 0, &mut out);
        assert_eq!(ui.macropad.down, 1 << 4);
        assert_eq!(
            out.deck_events().next(),
            Some(Event::Macro { face: MACRO_FACE, index: 4, act: 1 })
        );
        assert_eq!(
            global::held(&ui, Source::Touch),
            Some(Owner::Macro { face: MACRO_FACE, index: 4 })
        );
        let mut out = Update::new();
        global::release_source(&mut ui, Source::Touch, &mut out);
        on_touch(&mut ui, MACRO_FACE, key, TouchAct::Cancel, 5, &mut out);
        assert_eq!(ui.macropad.down, 0);
        assert_eq!(
            out.deck_events().collect::<std::vec::Vec<_>>(),
            [Event::Macro { face: MACRO_FACE, index: 4, act: 0 }]
        );
        // The "+ assign" placeholder opens the editor instead of pressing.
        let (x, y) = layout::macro_key(8).center();
        assert_eq!(hit(&ui, MACRO_FACE, x, y).map(|t| (t.kind, t.id)), Some((super::super::TargetKind::Tap, OPEN_ID + 8)));
    }

    fn apply(ui: &mut UiState, control: Control, at: u32) -> Update {
        let mut out = Update::new();
        ui.apply(control, at, &mut out);
        out
    }
    fn tap(ui: &mut UiState, button: Button, at: u32) -> Update {
        let out = apply(ui, Control::Button(button, Edge::Press), at);
        apply(ui, Control::Button(button, Edge::Release), at + 20);
        out
    }
    fn name(text: &str) -> HostMsg {
        let mut t = Text::new();
        t.set(text);
        HostMsg::Name { text: t }
    }

    #[test]
    fn a_tap_on_an_empty_slot_edits_it_and_save_sends_the_key() {
        let mut ui = on_macropad();
        let (x, y) = layout::macro_key(8).center();
        let (layer, target) = super::super::touch_hit(&ui, x, y).unwrap();
        let mut out = Update::new();
        super::super::touch_deliver(&mut ui, layer, target, TouchAct::Activate, 10, &mut out);
        assert_eq!(ui.global.overlay, Overlay::MacroEdit);
        assert_eq!((ui.macropad.edit.face, ui.macropad.edit.index, ui.macropad.edit.present), (MACRO_FACE, 8, false));
        assert_eq!(edit_rows(&ui.macropad.edit), 4, "no Remove for an empty slot");
        // Save without a label only complains.
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 20);
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 30);
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 31);
        assert_eq!(ui.macropad.edit.row, ROW_SAVE);
        let out = tap(&mut ui, Button::Enter, 40);
        assert!(ui.macropad.edit.needs_label && out.deck_events().count() == 0);
        assert_eq!(ui.global.overlay, Overlay::MacroEdit);
        // PTT dictates: a `name` session, and the host's text is the label.
        let out = apply(&mut ui, Control::Button(Button::PushToTalk, Edge::Press), 50);
        assert!(out.deck_events().any(|e| matches!(e, Event::Ptt { act: 1, ctx: crate::deck::msg::PttCtx::Name, .. })));
        apply(&mut ui, Control::Button(Button::PushToTalk, Edge::Release), 60);
        let mut out = Update::new();
        ui.host(&name("screenshot"), 70, &mut out);
        assert_eq!(ui.macropad.edit.label.as_str(), "screenshot");
        assert!(!ui.macropad.edit.needs_label);
        // Modifiers: ENTER adjusts, SCROLL steps the combos with hard ends,
        // ENTER ends. Key: the same through `KEYS`, LEFT/RIGHT by ten.
        apply(&mut ui, Control::Wheel(Wheel::Cw), 80);
        apply(&mut ui, Control::Wheel(Wheel::Cw), 81);
        assert_eq!(ui.macropad.edit.row, ROW_MODS);
        tap(&mut ui, Button::Enter, 90);
        assert!(ui.macropad.edit.adjusting);
        apply(&mut ui, Control::Wheel(Wheel::Cw), 100);
        assert_eq!(ui.macropad.edit.keys.mods, 0, "hard end");
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 110);
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 111);
        assert_eq!(ui.macropad.edit.keys.mods, MOD_CMD | MOD_SHIFT);
        tap(&mut ui, Button::Enter, 120);
        assert!(!ui.macropad.edit.adjusting);
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 130);
        assert_eq!(ui.macropad.edit.row, ROW_KEY);
        tap(&mut ui, Button::Enter, 131);
        for _ in 0..3 {
            tap(&mut ui, Button::Right, 132);
        }
        assert_eq!(ui.macropad.edit.keys.key, 30, "three jumps of ten");
        for _ in 0..3 {
            apply(&mut ui, Control::Wheel(Wheel::Cw), 133);
        }
        assert_eq!(ui.macropad.edit.keys.name(), "0", "a..z then 0: index 27");
        tap(&mut ui, Button::Left, 134);
        tap(&mut ui, Button::Left, 134);
        tap(&mut ui, Button::Left, 134);
        assert_eq!(ui.macropad.edit.keys.key, 0, "hard end at none");
        for _ in 0..31 {
            apply(&mut ui, Control::Wheel(Wheel::Ccw), 135);
        }
        assert_eq!(ui.macropad.edit.keys.name(), "4");
        tap(&mut ui, Button::Enter, 136);
        apply(&mut ui, Control::Wheel(Wheel::Ccw), 137);
        assert_eq!(ui.macropad.edit.row, ROW_SAVE);
        let out = tap(&mut ui, Button::Enter, 140);
        assert_eq!(ui.global.overlay, Overlay::None);
        let key = MacroKey {
            label: Text::from_str("screenshot"),
            binding: Binding::None,
            keys: Keystroke::parse("cmd+shift+4").unwrap(),
            present: true,
        };
        assert_eq!(out.deck_events().collect::<std::vec::Vec<_>>(), [Event::MacroKey { face: MACRO_FACE, index: 8 }]);
        assert_eq!(ui.deck.macro_set(MACRO_FACE).unwrap().keys[8], key, "the face shows it at once");
        // Modifiers without a key save as no keystroke (SHIFT + tap opens
        // a present key's editor).
        apply(&mut ui, Control::Button(Button::Shift, Edge::Press), 149);
        let (x, y) = layout::macro_key(7).center();
        let (layer, target) = super::super::touch_hit(&ui, x, y).unwrap();
        let mut out = Update::new();
        super::super::touch_deliver(&mut ui, layer, target, TouchAct::Activate, 150, &mut out);
        apply(&mut ui, Control::Button(Button::Shift, Edge::Release), 150);
        assert_eq!(ui.macropad.edit.index, 7);
        ui.host(&name("mods"), 151, &mut Update::new());
        ui.macropad.edit.keys = Keystroke { mods: MOD_CMD, key: 0 };
        ui.macropad.edit.row = ROW_SAVE;
        tap(&mut ui, Button::Enter, 152);
        assert_eq!(ui.deck.macro_set(MACRO_FACE).unwrap().keys[7].keys, Keystroke::NONE);
    }

    #[test]
    fn shift_tap_edits_a_present_key_and_remove_clears_it() {
        let mut ui = on_macropad();
        apply(&mut ui, Control::Button(Button::Shift, Edge::Press), 0);
        let (x, y) = layout::macro_key(0).center();
        let (layer, target) = super::super::touch_hit(&ui, x, y).unwrap();
        assert_eq!(target.kind, super::super::TargetKind::Tap);
        let mut out = Update::new();
        super::super::touch_deliver(&mut ui, layer, target, TouchAct::Activate, 10, &mut out);
        apply(&mut ui, Control::Button(Button::Shift, Edge::Release), 20);
        assert_eq!(ui.global.overlay, Overlay::MacroEdit);
        assert_eq!((ui.macropad.edit.label.as_str(), ui.macropad.edit.present), ("build", true));
        assert_eq!((ui.macropad.edit.binding, edit_rows(&ui.macropad.edit)), (Binding::Left, 5));
        // CANCEL discards.
        tap(&mut ui, Button::Cancel, 30);
        assert_eq!(ui.global.overlay, Overlay::None);
        assert_eq!(ui.deck.macro_set(MACRO_FACE).unwrap().keys[0].label.as_str(), "build");
        // Remove by touch on the row.
        apply(&mut ui, Control::Button(Button::Shift, Edge::Press), 40);
        let (layer, target) = super::super::touch_hit(&ui, x, y).unwrap();
        super::super::touch_deliver(&mut ui, layer, target, TouchAct::Activate, 50, &mut out);
        apply(&mut ui, Control::Button(Button::Shift, Edge::Release), 60);
        let (rx, ry) = layout::macro_edit_row(ROW_REMOVE).center();
        let (layer, row) = super::super::touch_hit(&ui, rx, ry).unwrap();
        assert_eq!(row.id, ROW_ID + ROW_REMOVE as u16);
        let mut out = Update::new();
        super::super::touch_deliver(&mut ui, layer, row, TouchAct::Activate, 70, &mut out);
        assert_eq!(ui.global.overlay, Overlay::None);
        assert!(!ui.deck.macro_set(MACRO_FACE).unwrap().keys[0].present);
        assert_eq!(out.deck_events().collect::<std::vec::Vec<_>>(), [Event::MacroKey { face: MACRO_FACE, index: 0 }]);
    }

    #[test]
    fn shift_cancel_puts_a_cursor_on_the_grid_that_enter_opens_and_cancel_drops() {
        let mut ui = on_macropad();
        apply(&mut ui, Control::Button(Button::Shift, Edge::Press), 0);
        tap(&mut ui, Button::Cancel, 10);
        apply(&mut ui, Control::Button(Button::Shift, Edge::Release), 30);
        assert_eq!(ui.macropad.cursor, Some((MACRO_FACE, 4)));
        assert_eq!(ui.current_face().template, Template::Macropad, "SHIFT+CANCEL is not home");
        // The wheel moves the cursor without triggering a bound key.
        let out = apply(&mut ui, Control::Wheel(Wheel::Ccw), 40);
        assert_eq!((ui.macropad.cursor, out.deck_events().count()), (Some((MACRO_FACE, 5)), 0));
        for _ in 0..6 {
            tap(&mut ui, Button::Right, 50);
        }
        assert_eq!(ui.macropad.cursor, Some((MACRO_FACE, 8)), "hard end");
        tap(&mut ui, Button::Left, 60);
        assert_eq!(ui.macropad.cursor, Some((MACRO_FACE, 7)));
        tap(&mut ui, Button::Enter, 70);
        assert_eq!((ui.global.overlay, ui.macropad.edit.index), (Overlay::MacroEdit, 7));
        tap(&mut ui, Button::Cancel, 80);
        assert_eq!(ui.global.overlay, Overlay::None);
        assert!(ui.macropad.cursor.is_some(), "back to the cursor");
        tap(&mut ui, Button::Cancel, 90);
        assert_eq!(ui.macropad.cursor, None);
        assert_eq!(ui.current_face().template, Template::Macropad);
        // Sliding to another face drops a cursor.
        ui.macropad.cursor = Some((MACRO_FACE, 1));
        ui.on_face(Template::Usage);
        let mut out = Update::new();
        ui.tick(100, 0, true, &mut out);
        assert_eq!(ui.macropad.cursor, None);
    }

    #[test]
    fn macro_key_pressed_by_touch_and_binding_emits_one_press() {
        use crate::control::{Touch, TouchPhase};
        let events = |out: &Update| out.deck_events().collect::<std::vec::Vec<_>>();
        let apply = |ui: &mut UiState, control: Control, at: u32| {
            let mut out = Update::new();
            ui.apply(control, at, &mut out);
            out
        };
        let button = |edge| Control::Button(Button::Enter, edge);
        let (x, y) = layout::macro_key(2).center();
        let touch = |phase| Control::Touch(Touch { phase, x: x as u16, y: y as u16 });
        let (down, up) = (Event::Macro { face: MACRO_FACE, index: 2, act: 1 }, Event::Macro {
            face: MACRO_FACE,
            index: 2,
            act: 0,
        });
        // Finger first, then the ENTER binding of the same key: one press;
        // the first release keeps the key down, the last one lets it go.
        let mut ui = on_macropad();
        assert_eq!(events(&apply(&mut ui, touch(TouchPhase::Down), 0)), [down]);
        assert_eq!(events(&apply(&mut ui, button(Edge::Press), 100)), []);
        assert_eq!(ui.macropad.down, 1 << 2);
        assert_eq!(events(&apply(&mut ui, button(Edge::Release), 200)), []);
        assert_eq!(ui.macropad.down, 1 << 2, "the finger still holds it");
        assert_eq!(events(&apply(&mut ui, touch(TouchPhase::Up), 300)), [up]);
        assert_eq!(ui.macropad.down, 0);
        // Binding first, finger second, finger up first.
        let mut ui = on_macropad();
        assert_eq!(events(&apply(&mut ui, button(Edge::Press), 0)), [down]);
        assert_eq!(events(&apply(&mut ui, touch(TouchPhase::Down), 100)), []);
        assert_eq!(events(&apply(&mut ui, touch(TouchPhase::Up), 200)), []);
        assert_eq!(ui.macropad.down, 1 << 2, "ENTER still holds it");
        assert_eq!(events(&apply(&mut ui, button(Edge::Release), 300)), [up]);
        assert_eq!(ui.macropad.down, 0);
        assert!(!global::anything_held(&ui));
    }
}
