//! Rev A inputs: six buttons and the wheel on GPIOs -> `Control`.
//!
//! This is the file that replaces the dev board's `keypad` shim. The pins are
//! sampled together into one mask and decoded by `direct`, which also says
//! how soon it wants the next sample. Idle, the task sleeps on any edge of
//! any pin with the usual fallback poll; while something is settling it
//! samples every `SETTLE_POLL_MS`.
//!
//! | Control | Pin  | Switch | Control | Pin  | Switch |
//! |---------|------|--------|---------|------|--------|
//! | PTT     | PA24 | SW5    | LEFT    | PA37 | SW3    |
//! | ENTER   | PA25 | SW1    | RIGHT   | PA38 | SW4    |
//! | ESC     | PA26 | SW2    | wheel A | PA39 | J2.2   |
//! | SHIFT   | PA27 | SW6    | wheel B | PA40 | J2.3   |
//!
//! All eight are documented standby/hibernate wake pins of the SF32LB525UC6
//! (PA24-PA27, PA34-PA44), so full sleep can be added without rewiring.

use embassy_futures::select::select_array;
use embassy_time::{with_timeout, Duration};
use sifli_hal::gpio::Input;

use crate::control::{now_ms, stamp, Control, Edge, CONTROLS};
use crate::logln;

use super::direct::{self, Decoder, Raw};

pub struct Pins {
    pub ptt: Input<'static>,
    pub enter: Input<'static>,
    pub cancel: Input<'static>,
    pub shift: Input<'static>,
    pub left: Input<'static>,
    pub right: Input<'static>,
    pub enc_a: Input<'static>,
    pub enc_b: Input<'static>,
}

impl Pins {
    fn raw(&self) -> Raw {
        let mut raw = 0;
        for (bit, pin) in [
            (direct::PTT, &self.ptt),
            (direct::ENTER, &self.enter),
            (direct::ESC, &self.cancel),
            (direct::SHIFT, &self.shift),
            (direct::LEFT, &self.left),
            (direct::RIGHT, &self.right),
            (direct::ENC_A, &self.enc_a),
            (direct::ENC_B, &self.enc_b),
        ] {
            if pin.is_high() {
                raw |= bit;
            }
        }
        raw
    }
}

#[embassy_executor::task]
pub async fn task(mut pins: Pins) {
    let raw = pins.raw();
    let mut decoder = Decoder::new(raw, now_ms());
    logln!(
        "inputs: six buttons + wheel on GPIO (PTT PA24, ENTER PA25, ESC PA26, SHIFT PA27, LEFT PA37, RIGHT PA38, wheel PA39/PA40), levels 0x{:02X}, {} leads clockwise",
        raw,
        if direct::A_LEADS_CLOCKWISE { "A" } else { "B" }
    );
    if decoder.held() != 0 {
        logln!(
            "inputs: held at boot (mask 0x{:02X}); ignored until released",
            decoder.held()
        );
    }
    loop {
        let wait = if decoder.busy(now_ms()) {
            direct::SETTLE_POLL_MS
        } else {
            crate::power::IDLE_POLL_MS
        };
        // Any edge on any pin ends the wait early; the fallback poll covers an
        // edge that lands between the sample and arming the interrupts.
        let _ = with_timeout(
            Duration::from_millis(wait as u64),
            select_array([
                pins.ptt.wait_for_any_edge(),
                pins.enter.wait_for_any_edge(),
                pins.cancel.wait_for_any_edge(),
                pins.shift.wait_for_any_edge(),
                pins.left.wait_for_any_edge(),
                pins.right.wait_for_any_edge(),
                pins.enc_a.wait_for_any_edge(),
                pins.enc_b.wait_for_any_edge(),
            ]),
        )
        .await;
        let events = decoder.sample(pins.raw(), now_ms());
        for control in events.iter() {
            match control {
                // A release ends a hold (PTT, SHIFT): wait for room rather
                // than strand it, as the touch task does for its releases.
                Control::Button(_, Edge::Release) => CONTROLS.send(stamp(control)).await,
                // A press or a detent dropped on a full queue is one a blocked
                // consumer could not have used in time anyway.
                _ => {
                    if CONTROLS.try_send(stamp(control)).is_err() {
                        logln!("inputs: control queue full, dropped {:?}", control);
                    }
                }
            }
        }
    }
}
