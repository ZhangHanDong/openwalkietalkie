//! Power policy, wake paths and battery service through the C entry points.
//!
//! Every local input wakes the locked screen without acting. Only four
//! counterclockwise wheel detents unlock it; the fourth detent is consumed.
//! Tests that watch what reaches the deck host run on `hosted` (idle-home
//! off) and send a `heartbeat` across each clock jump, so the host is still
//! present when the wake press acts.
use super::*;
use crate::deck::msg::Template;
use std::{string::String, vec::Vec};

const PTT_DOWN: &str = r#"{"m":"ptt","act":1,"agent":1,"ctx":"talk","p":1}"#;
const PTT_UP: &str = r#"{"m":"ptt","act":0,"agent":1,"ctx":"talk","p":1}"#;

fn nothing() -> (Vec<String>, Vec<String>) {
    (Vec::new(), Vec::new())
}

#[test]
fn brief_charge_done_gets_fresh_voltage_without_delaying_watchdog() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        (*core::ptr::addr_of_mut!(CORE)).devices = 1;
        BATTERY_MV.store(4190, Ordering::Relaxed);
        CHARGER_STATUS.store(0x12, Ordering::Relaxed);
        owt_core_poll(0);
        assert_eq!(owt_core_battery_percent(), 95);
        assert_eq!(BATTERY_READS.load(Ordering::Relaxed), 1);
        CHARGER_STATUS.store(0x1a, Ordering::Relaxed);
        owt_core_poll(1000); // Completion between normal 15s samples.
        assert_eq!(BATTERY_READS.load(Ordering::Relaxed), 2);
        assert_eq!((*core::ptr::addr_of!(CORE)).battery_at, 0);
        CHARGER_STATUS.store(0x12, Ordering::Relaxed);
        for now in (2000..=30_000).step_by(1000) {
            owt_core_poll(now);
            assert_eq!(owt_core_battery_percent(), 95);
        }
        assert_eq!(BATTERY_READS.load(Ordering::Relaxed), 4);
        assert_eq!(WATCHDOG_FEEDS.load(Ordering::Relaxed), 3);
        let mut diagnostic = [0; 9];
        owt_core_battery_diagnostic(diagnostic.as_mut_ptr());
        assert_eq!(&diagnostic[..2], &[95, 3]); // Real status stays Charging.
        assert_eq!(diagnostic[4], 1);
        assert_eq!(
            u32::from_le_bytes(diagnostic[5..9].try_into().unwrap()),
            29_000
        );
        for now in (31_000..=45_000).step_by(1000) {
            owt_core_poll(now);
        }
        assert_eq!(owt_core_battery_percent(), 100);
        assert_eq!(BATTERY_READS.load(Ordering::Relaxed), 5);
        assert_eq!(WATCHDOG_FEEDS.load(Ordering::Relaxed), 4);
        owt_core_battery_diagnostic(diagnostic.as_mut_ptr());
        assert_eq!(&diagnostic[..2], &[100, 3]);
        assert_eq!(diagnostic[4], 0);
    }
}

#[test]
fn repeated_done_edges_cannot_starve_watchdog_or_use_cached_voltage() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        (*core::ptr::addr_of_mut!(CORE)).devices = 1;
        BATTERY_MV.store(4190, Ordering::Relaxed);
        CHARGER_STATUS.store(0x12, Ordering::Relaxed);
        owt_core_poll(0);
        for now in (1000..=30_000).step_by(1000) {
            let done = now / 1000 % 2 == 1;
            CHARGER_STATUS.store(if done { 0x1a } else { 0x12 }, Ordering::Relaxed);
            // Every reported Done coincides with a low actual voltage,
            // despite the previous UI snapshot showing near-full voltage.
            BATTERY_MV.store(if done { 3729 } else { 4190 }, Ordering::Relaxed);
            owt_core_poll(now);
            assert_eq!(owt_core_battery_percent(), 95);
            assert!((*core::ptr::addr_of!(CORE))
                .battery_estimator
                .completion_since()
                .is_none());
        }
        assert_eq!(WATCHDOG_FEEDS.load(Ordering::Relaxed), 3);
        assert_eq!((*core::ptr::addr_of!(CORE)).battery_at, 30_000);
    }
}

#[test]
fn battery_poll_keeps_ui_and_host_estimate_stable_across_usb_insertion() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        (*core::ptr::addr_of_mut!(CORE)).devices = 1;
        BATTERY_MV.store(3750, Ordering::Relaxed);
        owt_core_poll(0);
        assert_eq!(owt_core_battery_percent(), 30);
        BATTERY_MV.store(3875, Ordering::Relaxed);
        CHARGER_STATUS.store(0x12, Ordering::Relaxed);
        for now in (15_000..=135_000).step_by(15_000) {
            owt_core_poll(now);
            let reported = (*core::ptr::addr_of!(CORE)).ui.battery;
            assert_eq!(reported.charge, battery::ChargeState::Charging);
            assert_eq!(reported.voltage_percentage(), Some(55));
            assert_eq!(reported.percentage(), Some(30));
            assert_eq!(owt_core_battery_percent(), 30);
        }
        for now in (150_000..=315_000).step_by(15_000) {
            owt_core_poll(now);
        }
        assert_eq!(owt_core_battery_percent(), 35);
        assert_eq!(
            (*core::ptr::addr_of!(CORE)).ui.battery.percentage(),
            Some(35)
        );
    }
}

#[test]
fn dark_polling_backs_off_but_touch_irq_wakes_immediately() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        let at = power::OFF_AFTER_MS;
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        core.devices = 6;
        core.power.tick(at);
        core.touch_poll_at = at;
        core.keypad_poll_at = at;
        for elapsed in [1, 50, 100, 499] {
            owt_core_poll(at + elapsed);
            assert_eq!(owt_core_wait_ms(), power::DARK_POLL_MS);
        }
        assert_eq!(INPUT_READS.load(Ordering::Relaxed), 0);
        owt_core_poll(at + 500);
        assert_eq!(INPUT_READS.load(Ordering::Relaxed), 2);
        lock(&TOUCH_REPORTS).extend([touch_frame(PROBE), release_frame(PROBE)]);
        TOUCH_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(at + 501); // Do not wait for the next recovery poll.
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert_eq!(owt_core_wait_ms(), power::ACTIVE_POLL_MS);
        owt_core_poll(at + 521); // Release uses the fast active fallback.
        assert!(!(*core::ptr::addr_of!(CORE)).contact.active());
        assert_eq!(drained(), nothing()); // Entire wake touch consumed.
    }
}

#[test]
fn short_wake_tap_with_only_a_release_report_does_not_click() {
    let _guard = lock(&CORE_LOCK);
    reset();
    unsafe {
        let at = 5 * 60 * 60 * 1000;
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        core.devices = 4;
        core.power.tick(at);
        owt_core_panel_slept();
        let released = release_frame(PROBE);
        lock(&TOUCH_REPORTS).push_back(released);
        owt_core_poll(at + 500); // An idle release alone must not wake.
        assert_eq!(owt_core_screen(), power::Screen::Off as u8);
        lock(&TOUCH_REPORTS).push_back(released);
        TOUCH_IRQ.store(true, Ordering::Relaxed);
        assert!(owt_core_poll(at + 501));
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert!(!(*core::ptr::addr_of!(CORE)).power.held());
        assert_eq!(drained(), nothing());
        // A second tap during panel reset is also consumed.
        lock(&TOUCH_REPORTS).push_back(touch_frame(PROBE));
        TOUCH_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(at + 502);
        assert_eq!(drained(), nothing());
    }
}

#[test]
fn unmapped_matrix_key_and_debug_wake_emit_nothing() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let at = power::OFF_AFTER_MS;
        let core = hosted(Template::Omarchy);
        core.devices = 2;
        core.power.tick(at);
        heartbeat(core, at);
        // '4' was never bound; the cross (11) was Decline in the old map.
        lock(&KEY_EVENTS).extend([22 | 0x80, 22, 11 | 0x80, 11]);
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        assert!(owt_core_poll(at + 1));
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert_eq!(drained(), nothing());
        owt_core_poll(2 * at + 1);
        assert_eq!(owt_core_screen(), power::Screen::Off as u8);
        owt_core_wake(2 * at + 2);
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert_eq!(drained(), nothing());
    }
}

#[test]
fn dark_keypad_irq_blocks_ptt_until_four_counterclockwise_ticks() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let at = u32::MAX - 10;
        let core = hosted(Template::Omarchy);
        core.devices = 2;
        core.power.tick(at);
        heartbeat(core, at);
        core.keypad_poll_at = at;
        lock(&KEY_EVENTS).extend([44 | 0x80, 44]);
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(2); // IRQ still wins across clock wrap.
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert_eq!(drained(), nothing());
        assert_eq!(core.ui.screen_lock, Some(0));
        assert!(core.ui.global.ptt.is_none());
        owt_core_frame_ready();
        for tick in 1..=4 {
            lock(&KEY_EVENTS).extend([41 | 0x80, 41]);
            KEYPAD_IRQ.store(true, Ordering::Relaxed);
            assert!(owt_core_poll(2 + tick * 20)); // Progress must redraw.
            assert_eq!(drained(), nothing());
            assert_eq!(core.ui.screen_lock, (tick < 4).then_some(tick as u8));
            assert!(core.repeat.is_none());
        }
        lock(&KEY_EVENTS).extend([41 | 0x80, 41]);
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(120);
        assert_eq!(deck_lines(), [r#"{"m":"key","k":"cw","act":2,"shift":0,"f":2}"#]);
        lock(&KEY_EVENTS).extend([44 | 0x80, 44]);
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(140);
        assert_eq!(deck_lines(), [PTT_DOWN, PTT_UP]);
    }
}

#[test]
fn keypad_event_during_acknowledgement_gets_another_immediate_wake() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let at = power::OFF_AFTER_MS;
        let core = hosted(Template::Omarchy);
        core.devices = 2;
        core.power.tick(at);
        heartbeat(core, at);
        core.keypad_poll_at = at;
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        KEY_ON_ACK.store(true, Ordering::Relaxed);
        owt_core_poll(at + 1); // Empty FIFO; short PTT arrives during INT clear.
        assert!(KEYPAD_IRQ.load(Ordering::Relaxed));
        assert_eq!(drained(), nothing());
        owt_core_poll(at + 2); // Scheduled wake, before the recovery deadline.
        assert!(!KEYPAD_IRQ.load(Ordering::Relaxed));
        assert_eq!(owt_core_screen(), power::Screen::Active as u8);
        assert_eq!(drained(), nothing());
        assert_eq!((*core::ptr::addr_of!(CORE)).ui.screen_lock, Some(0));
        assert!(!(*core::ptr::addr_of!(CORE)).power.held());
    }
}

#[test]
fn bus_error_releases_all_six_buttons_and_policy_not_held() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let core = hosted(Template::Omarchy);
        core.devices = 2;
        // Hold every button and the wheel key, then lose the bus. SHIFT goes
        // down after the wheel key (SHIFT+SCROLL would slide off the face)
        // and PTT last: its LISTENING session releases what the face held.
        lock(&KEY_EVENTS).extend([31, 32, 33, 21, 43, 42, 44].map(|number| number | 0x80));
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(100);
        assert!(core.power.held());
        assert!(core.repeat.is_some());
        assert!(core.ui.global.ptt.is_some());
        drained();
        KEYPAD_BUS_FAILS.store(true, Ordering::Relaxed);
        KEYPAD_IRQ.store(true, Ordering::Relaxed);
        owt_core_poll(105);
        assert!(!core.power.held());
        assert!(core.repeat.is_none());
        assert!(core.ui.global.ptt.is_none());
        assert_eq!(deck_lines(), [PTT_UP]);
        // Further failed polls stay quiet and let the screen sleep.
        for now in [110, 160, 5000] {
            KEYPAD_IRQ.store(true, Ordering::Relaxed);
            owt_core_poll(now);
        }
        assert_eq!(drained(), nothing());
        owt_core_poll(105 + power::OFF_AFTER_MS);
        assert_eq!(owt_core_screen(), power::Screen::Off as u8);
    }
}

#[test]
fn sleeping_host_status_stays_live_and_wake_touch_emits_nothing() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let core = hosted(Template::Omarchy);
        owt_core_poll(power::OFF_AFTER_MS);
        heartbeat(core, power::OFF_AFTER_MS);
        rpc_request(br#"{"method":"device.status","id":19}"#);
        assert_eq!(drained().0.len(), 1);
        assert!(!owt_core_poll(power::OFF_AFTER_MS + 10));
        assert_eq!(owt_core_screen(), power::Screen::Off as u8);
        let core = &mut *core::ptr::addr_of_mut!(CORE);
        for phase in [
            control::TouchPhase::Down,
            control::TouchPhase::Move,
            control::TouchPhase::Up,
        ] {
            apply(
                core,
                Control::Touch(control::Touch {
                    phase,
                    x: 186,
                    y: 294,
                }),
            );
        }
        assert_eq!(drained(), nothing());
        assert_eq!(core.power.screen(), power::Screen::Active);
        core.power.frame_ready(core.now);
        for edge in [Edge::Press, Edge::Release] {
            apply(core, Control::Button(Button::PushToTalk, edge));
        }
        assert_eq!(drained(), nothing());
        for _ in 0..power::UNLOCK_TICKS {
            apply(core, Control::Wheel(Wheel::Ccw));
        }
        assert_eq!(drained(), nothing());
        for edge in [Edge::Press, Edge::Release] {
            apply(core, Control::Button(Button::PushToTalk, edge));
        }
        assert_eq!(deck_lines(), [PTT_DOWN, PTT_UP]);
    }
}

#[test]
fn gpio_wheel_unlocks_but_a_ptt_held_in_the_bag_needs_a_fresh_press() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let core = hosted(Template::Omarchy);
        core.inputs = Some(direct::Decoder::new(0xff, 0));
        let mut now = power::OFF_AFTER_MS;
        heartbeat(core, now);
        owt_core_poll(now);
        owt_core_panel_slept();
        let mut sample = |raw: u8, elapsed: u32| {
            now += elapsed;
            DIRECT_RAW.store(raw as usize, Ordering::Relaxed);
            DIRECT_IRQ.store(true, Ordering::Relaxed);
            owt_core_poll(now);
        };
        let held = 0xff & !direct::PTT;
        sample(held, 1);
        sample(held, direct::DEBOUNCE_MS);
        assert_eq!(core.power.screen(), power::Screen::Active);
        assert_eq!(core.ui.screen_lock, Some(0));
        assert!(core.ui.global.ptt.is_none());
        assert_eq!(drained(), nothing());
        owt_core_frame_ready();
        // The assembled wheel is B-leading clockwise. Four CW detents must
        // not unlock, and CW must also reset a partial CCW sequence.
        let clockwise = [held & !direct::ENC_B, held & !direct::ENC_A & !direct::ENC_B,
                         held & !direct::ENC_A, held];
        let counterclockwise = [held & !direct::ENC_A, held & !direct::ENC_A & !direct::ENC_B,
                                held & !direct::ENC_B, held];
        for _ in 0..4 {
            for raw in clockwise {
                sample(raw, 2);
            }
            assert_eq!(core.ui.screen_lock, Some(0));
            assert_eq!(drained(), nothing());
            assert!(core.ui.global.ptt.is_none());
        }
        for tick in 1..=3 {
            for raw in counterclockwise {
                sample(raw, 2);
            }
            assert_eq!(core.ui.screen_lock, Some(tick));
            assert_eq!(drained(), nothing());
        }
        for raw in clockwise {
            sample(raw, 2);
        }
        assert_eq!(core.ui.screen_lock, Some(0));
        assert_eq!(drained(), nothing());
        for tick in 1..=4 {
            // One complete CCW quadrature cycle is exactly one wheel tick.
            for raw in counterclockwise {
                sample(raw, 2);
            }
            assert_eq!(core.ui.screen_lock, (tick < 4).then_some(tick));
            assert_eq!(drained(), nothing());
            assert!(core.ui.global.ptt.is_none());
        }
        assert_eq!(core.ui.current_face().template, Template::Omarchy);
        sample(0xff, 1);
        sample(0xff, direct::DEBOUNCE_MS);
        assert_eq!(drained(), nothing());
        // Once unlocked, physical CCW selects the next window (wire key cw).
        for raw in counterclockwise {
            sample(raw | direct::PTT, 2);
        }
        assert_eq!(deck_lines(), [r#"{"m":"key","k":"cw","act":2,"shift":0,"f":2}"#]);
        sample(held, 1);
        sample(held, direct::DEBOUNCE_MS);
        assert_eq!(deck_lines(), [PTT_DOWN]);
        sample(0xff, 1);
        sample(0xff, direct::DEBOUNCE_MS);
        assert_eq!(deck_lines(), [PTT_UP]);
    }
}

#[test]
fn locked_cancel_hold_cannot_power_off_and_a_host_notice_cannot_unlock() {
    let _guard = lock(&CORE_LOCK);
    unsafe {
        let core = hosted(Template::Omarchy);
        heartbeat(core, power::OFF_AFTER_MS);
        owt_core_poll(power::OFF_AFTER_MS);
        owt_core_panel_slept();
        core.now += 1;
        apply(core, Control::Button(Button::Cancel, Edge::Press));
        owt_core_frame_ready();
        heartbeat(core, power::OFF_AFTER_MS + 4000);
        owt_core_poll(power::OFF_AFTER_MS + 4000);
        assert!(core.ui.power_hold().is_none());
        assert_eq!(core.ui.screen_lock, Some(0));
        host_line(br#"{"m":"card","agent":1,"id":1,"type":"q","text":"Ready?","choices":["Yes","No"]}"#);
        assert_eq!(core.ui.screen_lock, Some(0));
        assert_eq!(core.power.lock_progress(), Some(0));
        assert_eq!(core.ui.deck.card(1).unwrap().id, 1);
        assert_eq!(core.power.screen(), power::Screen::Active);
        apply(core, Control::Button(Button::Cancel, Edge::Release));
        assert!(core.ui.global.ptt.is_none());
    }
}
