//! Input and output end to end through the C entry points: DMA frames and
//! the splash, raw touch reports, the keypad through the shared keymap, and
//! the system requests on channel 2.
use super::*;
use crate::deck::msg::Template;
use std::{string::String, vec::Vec};

#[test]
fn firmware_dma_windows_and_splash_match_the_mounted_scene() {
    let _lock = lock(&CORE_LOCK);
    reset();
    unsafe { (*core::ptr::addr_of_mut!(CORE)).ui.on_face(Template::Settings) };
    // Independent oracle: place every logical pixel at its mounted
    // destination without using the transfer rectangle/rotation helpers.
    fn expected(ui: Option<&UiState>) -> Vec<u8> {
        let mut screen = vec![0; 480 * 480 * 2];
        let mut band = display::Band::new();
        for index in 0..display::BANDS {
            band.set_row(index);
            if let Some(ui) = ui {
                scene::draw(&mut band, ui);
            } else {
                splash::draw(&mut band);
            }
            for (pixel, rgb) in band.data().chunks_exact(2).enumerate() {
                let logical = index as usize * 480 * 48 + pixel;
                let physical = if orientation::ROTATE_180 {
                    480 * 480 - 1 - logical // Half-turn.
                } else {
                    logical
                };
                screen[physical * 2..physical * 2 + 2].copy_from_slice(rgb);
            }
        }
        screen
    }
    unsafe {
        let mut screen = vec![0; 480 * 480 * 2];
        for index in 0..display::BANDS {
            let pixels = owt_core_draw_splash_band(index);
            assert!(!pixels.is_null());
            let start = index as usize * display::BAND_BYTES;
            screen[start..start + display::BAND_BYTES]
                .copy_from_slice(core::slice::from_raw_parts(pixels, display::BAND_BYTES));
        }
        assert_eq!(screen, expected(None));
        assert!(owt_core_draw_splash_band(display::BANDS).is_null());
        for frame in 0..5 {
            let ui = &mut (*core::ptr::addr_of_mut!(CORE)).ui;
            match frame {
                1 => {
                    ui.apply(Control::Button(Button::Enter, Edge::Press), 0, &mut Update::new());
                }
                2 => {
                    ui.apply(Control::Button(Button::Enter, Edge::Release), 100, &mut Update::new());
                    ui.apply(Control::Button(Button::Enter, Edge::Press), 200, &mut Update::new());
                    ui.tick(400, 0, true, &mut Update::new());
                }
                3 => {
                    ui.settings.panel = 3;
                    ui.invalidate();
                }
                _ => {}
            }
            // Initial and wake refreshes must replace all GRAM, even if
            // the visible state has not changed since the previous frame.
            let force = frame == 0 || frame == 4;
            assert!(owt_core_begin_frame(force));
            let mut rect = damage::Region {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            };
            let mut slot = 0;
            let mut end_y = 0;
            let mut transferred = 0;
            loop {
                let pixels = owt_core_draw_next(slot, &mut rect);
                if pixels.is_null() {
                    break;
                }
                assert_eq!(pixels as usize & 3, 0);
                assert!(rect.y0 >= end_y);
                end_y = rect.y1;
                let row_bytes = (rect.x1 - rect.x0) as usize * 2;
                for row in 0..(rect.y1 - rect.y0) as usize {
                    let source = core::slice::from_raw_parts(pixels.add(row * 480 * 2), row_bytes);
                    let start = ((rect.y0 as usize + row) * 480 + rect.x0 as usize) * 2;
                    screen[start..start + row_bytes].copy_from_slice(source);
                    transferred += row_bytes / 2;
                }
                slot ^= 1;
            }
            assert_eq!(screen, expected(Some(&(*core::ptr::addr_of!(CORE)).ui)));
            if force {
                assert_eq!(transferred, 480 * 480);
            } else {
                assert!(transferred > 0 && transferred < 480 * 480);
            }
        }
    }
}

#[test]
fn boot_log_rotation_matches_the_board_feature() {
    assert_eq!(
        owt_core_rotation(),
        if cfg!(feature = "hw-a") { 180 } else { 0 }
    );
}

#[test]
fn dma_buffers_are_distinct_and_input_changes_wait_for_the_next_snapshot() {
    let _lock = lock(&CORE_LOCK);
    reset();
    unsafe {
        (*core::ptr::addr_of_mut!(CORE)).ui.on_face(Template::Settings);
        assert!(owt_core_begin_frame(false));
        let mut region = damage::Region {
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
        };
        let first = owt_core_draw_next(0, &mut region);
        assert!(!first.is_null());
        assert_eq!(first as usize & 3, 0);
        let saved = core::slice::from_raw_parts(first, display::BAND_BYTES).to_vec();
        let second = owt_core_draw_next(1, &mut region);
        assert!(!second.is_null());
        assert_eq!(second as usize & 3, 0);
        assert!(first.addr().abs_diff(second.addr()) >= display::BAND_BYTES);
        assert_eq!(
            core::slice::from_raw_parts(first, display::BAND_BYTES),
            saved
        );
        let row = (*core::ptr::addr_of!(CORE)).ui.settings.row;
        (*core::ptr::addr_of_mut!(CORE))
            .ui
            .apply(Control::Wheel(Wheel::Ccw), 0, &mut Update::new());
        assert_ne!((*core::ptr::addr_of!(CORE)).ui.settings.row, row);
        assert_eq!((*core::ptr::addr_of!(FRAME)).settings.row, row);
        let mut slot = 0;
        while !owt_core_draw_next(slot, &mut region).is_null() {
            slot ^= 1;
        }
        assert!(owt_core_begin_frame(false));
        assert_ne!((*core::ptr::addr_of!(FRAME)).settings.row, row);
        let mut pixels = 0;
        while !owt_core_draw_next(slot, &mut region).is_null() {
            pixels += (region.x1 - region.x0) as u32 * (region.y1 - region.y0) as u32;
            slot ^= 1;
        }
        assert!(pixels > 0 && pixels < 480 * 480 / 2, "{pixels}");
        assert!(!owt_core_begin_frame(false));
        assert!(owt_core_begin_frame(true)); // Wake must restore all panel RAM.
        assert!(!owt_core_draw_next(0, &mut region).is_null());
        assert_eq!(
            region,
            damage::Region {
                x0: 0,
                y0: 0,
                x1: 480,
                y1: 48
            }
        );
    }
}

/// Power-on core with the demo on the Macropad face, and the centre of its
/// first key.
fn macropad(core: &mut Core) -> (u16, u16) {
    core.ui.load_demo();
    core.ui.on_face(Template::Macropad);
    let (x, y) = crate::ui::layout::macro_key(0).center();
    (x as u16, y as u16)
}

#[test]
fn missed_irq_recovers_raw_touch_reports_and_their_edges() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        let key = macropad(core);
        core.contact = touch_state::Contact::new();
        core.ui.set_connected(true, &mut Update::new());
        core.devices = 4;
        core.touch_poll_at = 0;
        core.battery_at = 0;
        drained();
        // A raw contact on the key, then its release.
        lock(&TOUCH_REPORTS).extend([touch_frame(key), release_frame(key)]);
        owt_core_poll(50); // Idle fallback, with the mock IRQ deasserted.
        assert_ne!((*core::ptr::addr_of!(CORE)).ui.macropad.down, 0);
        owt_core_poll(70);
        assert_eq!((*core::ptr::addr_of!(CORE)).ui.macropad.down, 0);
        // The demo answers the key itself: nothing reaches the wire.
        assert_eq!(drained(), (Vec::new(), Vec::new()));
    }
}

#[test]
fn stopped_touch_reports_cancel_the_key_across_tick_wrap() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        let key = macropad(core);
        core.devices = 4;
        core.touch_at = u32::MAX - 60;
        let (x, y) = raw_point(key);
        let touch = core
            .contact
            .sample(cst9220::Report::Contact { x, y })
            .unwrap();
        apply(core, Control::Touch(touch));
        assert_ne!(core.ui.macropad.down, 0);
        owt_core_poll(50); // 111 ms since the last real report.
        assert_ne!((*core::ptr::addr_of!(CORE)).ui.macropad.down, 0);
        owt_core_poll(59); // 120 ms: release without activating.
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        assert_eq!(core.ui.macropad.down, 0);
        assert!(!core.contact.active());
    }
}

fn line(json: &str) -> String {
    String::from(json)
}

fn key_line(k: &str, act: u8) -> String {
    std::format!(r#"{{"m":"key","k":"{k}","act":{act},"shift":0,"f":2}}"#)
}

#[test]
fn keypad_keys_reach_the_host_through_the_shared_keymap() {
    let _guard = lock(&CORE_LOCK);
    let core = hosted(Template::Omarchy);
    core.devices = 2;
    let mut now = 100;
    let cases: [(u8, Vec<String>, Vec<String>); 10] = [
        (31, std::vec![key_line("enter", 1)], std::vec![key_line("enter", 0)]),
        (32, std::vec![key_line("left", 1)], std::vec![key_line("left", 0)]),
        (33, std::vec![key_line("right", 1)], std::vec![key_line("right", 0)]),
        // SHIFT never acts alone.
        (43, std::vec![], std::vec![]),
        (21, std::vec![key_line("ccw", 2)], std::vec![]),
        (41, std::vec![key_line("cw", 2)], std::vec![]),
        (
            44,
            std::vec![line(r#"{"m":"ptt","act":1,"agent":1,"ctx":"talk","p":1}"#)],
            std::vec![line(r#"{"m":"ptt","act":0,"agent":1,"ctx":"talk","p":1}"#)],
        ),
        // The old Decline key and an unbound key do nothing.
        (11, std::vec![], std::vec![]),
        (34, std::vec![], std::vec![]),
        // CANCEL at the face's top level goes home.
        (42, std::vec![line(r#"{"m":"face","f":1}"#)], std::vec![]),
    ];
    for (number, press, release) in cases {
        now += 200;
        key(number, true, now);
        assert_eq!(deck_lines(), press, "key {number} press");
        key(number, false, now + 5);
        assert_eq!(deck_lines(), release, "key {number} release");
        if number == 44 {
            host_line(br#"{"m":"transcript","agent":1,"text":"done","final":1,"p":1}"#);
        }
        assert!(!core.power.held());
        assert!(core.repeat.is_none());
    }
}

#[test]
fn held_wheel_key_repeats_in_the_direction_of_its_first_detent() {
    let _guard = lock(&CORE_LOCK);
    let core = hosted(Template::Omarchy);
    core.devices = 2;
    for (number, name) in [(21, "ccw"), (41, "cw")] {
        let start = u32::MAX - 200; // Hold across the clock wrap.
        heartbeat(core, start);
        key(number, true, start);
        assert_eq!(core.repeat.map(|held| held.0), Some(keymap_wheel(number)));
        unsafe {
            assert_eq!(owt_core_wait_ms(), power::ACTIVE_POLL_MS);
            owt_core_poll(start.wrapping_add(keymap::REPEAT_DELAY_MS - 1));
            assert_eq!(deck_lines(), [key_line(name, 2)]); // First detent only.
            owt_core_poll(start.wrapping_add(keymap::REPEAT_DELAY_MS));
            owt_core_poll(start.wrapping_add(keymap::REPEAT_DELAY_MS + 1));
            owt_core_poll(start.wrapping_add(keymap::REPEAT_DELAY_MS + keymap::REPEAT_PERIOD_MS));
        }
        assert_eq!(deck_lines(), [key_line(name, 2), key_line(name, 2)]);
        // The other wheel key's release must not stop this repeat.
        key(62 - number, false, start.wrapping_add(keymap::REPEAT_DELAY_MS + 100));
        assert!(core.repeat.is_some());
        key(number, false, start.wrapping_add(keymap::REPEAT_DELAY_MS + 110));
        assert!(core.repeat.is_none());
        assert!(deck_lines().is_empty());
    }
    // A lost release stops at the cap instead of spinning forever.
    key(21, true, 1000);
    unsafe { owt_core_poll(1000 + keymap::REPEAT_MAX_MS) };
    assert!(core.repeat.is_some());
    unsafe { owt_core_poll(1001 + keymap::REPEAT_MAX_MS) };
    assert!(core.repeat.is_none());
}

fn keymap_wheel(number: u8) -> Wheel {
    match keymap::map(number) {
        keymap::Mapped::Wheel(wheel) => wheel,
        other => panic!("key {number} maps to {other:?}"),
    }
}

#[test]
fn status_requests_accept_ble_bodies_and_macos_report_ids() {
    let _guard = lock(&CORE_LOCK);
    reset();
    let mut report = [0; wire::REPORT_LEN];
    wire::frame(br#"{"method":"device.status","id":98701}"#, 0, &mut report);
    for bytes in [&report[1..], &report[..]] {
        unsafe {
            owt_core_hid_connected(true);
            owt_core_hid_report(bytes.as_ptr(), bytes.len());
            let (rpc, deck) = drained();
            assert!(deck.is_empty());
            assert_eq!(rpc.len(), 1);
            assert!(rpc[0].starts_with(r#"{"id":98701,"result":{"version":"#), "{}", rpc[0]);
            owt_core_hid_connected(false);
        }
    }
}

#[test]
fn sys_version_names_the_native_image_and_unknown_methods_are_refused() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe { owt_core_hid_connected(true) };
    rpc_request(br#"{"method":"sys.version","id":1}"#);
    rpc_request(br#"{"method":"v.oai.thstatus","params":[],"id":2}"#);
    assert_eq!(
        drained().0,
        [
            std::format!(
                r#"{{"id":1,"result":{{"version":"0.1.0-openwalkietalkie-hfp","board":"{}"}}}}"#,
                orientation::BOARD
            ),
            String::from(r#"{"id":2,"error":{"code":-32601,"message":"Method not found"}}"#),
        ]
    );
    unsafe { owt_core_hid_connected(false) };
}
