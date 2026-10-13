//! Raw CST9220 frames through the contact tracker into a face's key.
use crate::{
    control::Control,
    cst9220::Report,
    deck::msg::Template,
    orientation,
    touch_state::Contact,
    ui::{layout, UiState, Update},
};

/// A probe point and its raw frame on the half-turned panel: (186, 294)
/// reports at raw (293, 185).
const PROBE: (u16, u16) = (186, 294);
const TURNED_PROBE: [u8; 15] = [0x06, 18, 11, 0x59, 0x20, 1, 0xab, 0, 0, 0, 0, 0, 0, 0, 0];

/// One-contact CST9220 frame whose raw position lands on logical (x, y)
/// for this build's mounting; either mounting is its own inverse.
fn frame((x, y): (u16, u16)) -> [u8; 15] {
    let (x, y) = orientation::point(x, y).unwrap();
    let mut bytes = [0; 15];
    bytes[..7].copy_from_slice(&[
        0x06,
        (x >> 4) as u8,
        (y >> 4) as u8,
        ((x & 15) << 4 | (y & 15)) as u8,
        0x20,
        1,
        0xab,
    ]);
    bytes
}

/// Power-on state with the Macropad face on screen, and the centre of its
/// first key.
fn macropad_face() -> (Box<UiState>, (u16, u16)) {
    let mut ui = Box::new(UiState::new());
    ui.reset();
    ui.load_demo();
    ui.on_face(Template::Macropad);
    let (x, y) = layout::macro_key(0).center();
    (ui, (x as u16, y as u16))
}

fn deliver(contact: &mut Contact, ui: &mut UiState, bytes: [u8; 15]) {
    if let Some(event) = contact.sample(Report::decode(bytes)) {
        ui.apply(Control::Touch(event), 0, &mut Update::new());
    }
}

#[test]
fn the_raw_frame_follows_the_panel_mounting() {
    assert_eq!(
        frame(PROBE),
        if orientation::ROTATE_180 {
            TURNED_PROBE
        } else {
            [0x06, 11, 18, 0xa6, 0x20, 1, 0xab, 0, 0, 0, 0, 0, 0, 0, 0]
        }
    );
}

#[test]
fn rapid_key_taps_release_with_stale_coordinates_in_empty_frames() {
    let mut contact = Contact::new();
    let (mut ui, key) = macropad_face();
    // The empty release frame retains the same raw coordinate slot.
    let down = frame(key);
    let mut up = down;
    up[5] = 0;
    for _ in 0..1000 {
        deliver(&mut contact, &mut ui, down);
        assert_ne!(ui.macropad.down, 0);
        deliver(&mut contact, &mut ui, up);
        assert_eq!(ui.macropad.down, 0);
        assert!(!contact.active());
    }
}

#[test]
fn malformed_frame_clears_the_key_without_activating_it() {
    let mut contact = Contact::new();
    let (mut ui, key) = macropad_face();
    deliver(&mut contact, &mut ui, frame(key));
    assert_ne!(ui.macropad.down, 0);
    deliver(&mut contact, &mut ui, [0; 15]);
    assert_eq!(ui.macropad.down, 0);
}
