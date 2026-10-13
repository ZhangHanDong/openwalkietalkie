//! Development-board input shim: T-SF32-Keyboard -> `Control`.
//!
//! OpenWalkieTalkie's own hardware has a scroll wheel and six buttons. The
//! LilyGo board has neither, so until our board exists the keypad expansion
//! stands in for them. This module is the *only* place that substitution is
//! visible; when the real wheel and buttons arrive, this file is replaced by an
//! encoder + GPIO reader and nothing above `control` changes.
//!
//! Physical keypad, with the TCA8418 key numbers LilyGo's SDK assigns
//! (`t_sf32_display_hw_device/input_keyboard/inc/tca8418.h`):
//!
//! ```text
//!            C0            C1        C2        C3
//!   R4 |  41 PREV  ^     42 ESC   43 HOME   44 MAIL  (envelope)
//!   R3 |  31 ENTER .     32 '1'   33 '2'    34 '3'
//!   R2 |  21 NEXT  v     22 '4'   23 '5'    24 '6'
//!   R1 |  11 CLOSE x     12 '7'   13 '8'    14 '9'
//!   R0 |   1 STOP        2 '*'     3 '0'     4 '#'
//! ```
//!
//! The binding itself lives in `keymap`, shared with the native runtime. Up,
//! dot and down run down the left edge, so the dot reads naturally as ENTER
//! between the two wheel keys; the top row carries CANCEL, SHIFT and PTT, and
//! '1'/'2' next to the dot are the front LEFT/RIGHT keys.
//!
//! | Keypad key        | Control                              |
//! |-------------------|--------------------------------------|
//! | `v` down (21)     | wheel clockwise ("next", repeats)    |
//! | `^` up (41)       | wheel counter-clockwise ("previous") |
//! | `.` dot (31)      | Enter                                |
//! | ESC (42)          | Cancel                               |
//! | HOME (43)         | Shift                                |
//! | envelope (44)     | PushToTalk (held)                    |
//! | `1` (32)          | Left                                 |
//! | `2` (33)          | Right                                |
//!
//! Every other key, including the old `x` cross (11), is unbound and logged.
//! What the controls mean (SHIFT chords such as HOME + down for the next
//! face, the dark-screen wake rule) is decided above `control`; see
//! `docs/agent-command-deck.md`.

use embassy_time::{with_timeout, Duration, Instant, Timer};
use sifli_hal::gpio::Input;

use crate::board::{BusDevice, BusError};
use crate::control::{stamp, Button, Control, Edge, Wheel, CONTROLS};
use crate::logln;

use super::keymap::{self, Mapped};
use super::tca8418::{self, KeyEvent, Tca8418};
use super::xl9555::{self, Xl9555};

/// Matrix geometry of the T-SF32-Keyboard.
const ROWS: u8 = 5;
const COLS: u8 = 4;

/// Fast enough that a press never feels dropped, slow enough that the I2C bus
/// is idle almost all the time.
const POLL: Duration = Duration::from_millis(crate::power::IDLE_POLL_MS as u64);
/// Hold up or down and the wheel keeps turning. A real encoder gives detents
/// as fast as you can spin it; a key held down has to fake that, and these are
/// the usual key-repeat numbers.
const REPEAT_DELAY: Duration = Duration::from_millis(keymap::REPEAT_DELAY_MS as u64);
const REPEAT_PERIOD: Duration = Duration::from_millis(keymap::REPEAT_PERIOD_MS as u64);

/// Longest a held wheel key may keep repeating. The TCA8418's FIFO is ten
/// events deep and we drain it on a poll, so a burst of typing can in principle
/// overflow it and lose a release. Without a ceiling that would strand the
/// wheel spinning forever; with one it stops and says so.
const REPEAT_MAX: Duration = Duration::from_millis(keymap::REPEAT_MAX_MS as u64);

/// A wheel key being held, and when it should emit its next detent. It keeps
/// the direction, not the key number, so a repeat can never turn the other way.
struct Repeat {
    dir: Wheel,
    next: Instant,
    /// When the hold began, for the `REPEAT_MAX` ceiling.
    started: Instant,
}

#[embassy_executor::task]
pub async fn task(bus: BusDevice, mut interrupt: Input<'static>) {
    let mut bus = bus;

    logln!("keypad: bringing up keyboard board");
    if let Err(e) = power_up(&mut bus).await {
        logln!(
            "keypad: XL9555 power-up failed ({:?}) -- is the keyboard attached?",
            e
        );
        // Fall through anyway: if the board happens to be powered already the
        // keypad may still answer, and the scan below will say so either way.
    }

    logln!("keypad: I2C1 devices after power-up:");
    let found = super::probe_known(&mut bus);
    if found == 0 {
        logln!("keypad: no keyboard board on the bus; giving up");
        return;
    }

    let mut keypad = Tca8418::new(bus, tca8418::ADDR);
    if let Err(e) = keypad.configure(ROWS, COLS) {
        logln!("keypad: TCA8418 configure failed: {:?}", e);
        return;
    }
    logln!(
        "keypad: TCA8418 ready ({}x{} matrix), PA11 interrupt + 50ms fallback",
        ROWS,
        COLS
    );

    let mut repeat: Option<Repeat> = None;
    let mut bus_failed = false;

    loop {
        let poll = if repeat.is_some() {
            Duration::from_millis(15)
        } else {
            POLL
        };
        let irq = with_timeout(poll, interrupt.wait_for_low()).await.is_ok();
        let mut acknowledge = irq;

        // Drain the FIFO; the chip has already debounced everything in it.
        loop {
            match keypad.next_event() {
                Ok(None) => {
                    bus_failed = false;
                    break;
                }
                Ok(Some(event)) => {
                    bus_failed = false;
                    acknowledge = true;
                    handle(event, &mut repeat);
                }
                Err(e) => {
                    logln!("keypad: read failed: {:?}", e);
                    // A key-up lost with the bus must not leave a button held
                    // or the wheel spinning. Once per outage: the app ignores
                    // repeated releases, but at the poll rate they would crowd
                    // real input out of the control queue.
                    if !core::mem::replace(&mut bus_failed, true) {
                        repeat = None;
                        for button in Button::ALL {
                            CONTROLS.send(stamp(Control::Button(button, Edge::Release))).await;
                        }
                    }
                    break;
                }
            }
        }
        if acknowledge {
            if let Err(error) = keypad.acknowledge() {
                logln!("keypad: interrupt acknowledge failed: {:?}", error);
                Timer::after(POLL).await;
            }
        }

        let now = Instant::now();
        let stalled = match repeat.as_mut() {
            Some(r) if now.saturating_duration_since(r.started) > REPEAT_MAX => {
                logln!(
                    "keypad: wheel {:?} repeated for {}s, stopping (lost release?)",
                    r.dir,
                    REPEAT_MAX.as_secs()
                );
                true
            }
            Some(r) => {
                if now >= r.next {
                    emit(Control::Wheel(r.dir));
                    r.next = now + REPEAT_PERIOD;
                }
                false
            }
            None => false,
        };
        if stalled {
            repeat = None;
        }
    }
}

/// Bring up the keyboard board's rails and release the keypad from reset.
///
/// Mirrors LilyGo's sequence: the TCA8418 samples its configuration coming out
/// of reset, so its rail has to be stable before `KEY_RST` is released or it
/// comes back with a scrambled matrix.
async fn power_up(bus: &mut BusDevice) -> Result<(), BusError> {
    let mut expander = Xl9555::new(bus, xl9555::ADDR);
    expander.init()?;
    Timer::after_millis(20).await;

    // Shares a rail with the keypad on this board, so it comes up first even
    // though nothing here talks to the ESP32C6.
    expander.set(xl9555::WIFI_EN, true)?;
    Timer::after_millis(20).await;

    // Lighting is simulated on the OLED. The physical LED rail stays off.
    Timer::after_millis(100).await;

    expander.set(xl9555::KEY_RST, false)?;
    Timer::after_millis(10).await;
    expander.set(xl9555::KEY_RST, true)?;
    Timer::after_millis(50).await;

    Ok(())
}

fn handle(event: KeyEvent, repeat: &mut Option<Repeat>) {
    let edge = if event.pressed {
        Edge::Press
    } else {
        Edge::Release
    };
    match keymap::map(event.key) {
        // Repeat bookkeeping runs on both edges even though only a press is a
        // `Control` (a detent has no release): the release is the one event
        // that stops the repeat, and dropping it would spin the wheel forever.
        Mapped::Wheel(dir) => {
            if event.pressed {
                let now = Instant::now();
                *repeat = Some(Repeat {
                    dir,
                    next: now + REPEAT_DELAY,
                    started: now,
                });
                emit(Control::Wheel(dir));
            } else if repeat.as_ref().is_some_and(|r| r.dir == dir) {
                // Only the direction that started the repeat may stop it, so
                // rolling a finger from up to down does not leave it stuck.
                *repeat = None;
            }
        }
        // Buttons can be held (PTT, SHIFT), so both edges matter.
        Mapped::Button(button) => emit(Control::Button(button, edge)),
        // Unbound keys are logged rather than swallowed: this is how you read
        // off a key number when re-mapping the shim.
        Mapped::None if event.pressed => match event.position() {
            Some((row, col)) => {
                logln!("keypad: unbound key {} (R{}C{})", event.key, row, col)
            }
            None => logln!("keypad: GPI event {}", event.key),
        },
        Mapped::None => {}
    }
}

fn emit(control: Control) {
    if CONTROLS.try_send(stamp(control)).is_err() {
        logln!("keypad: control queue full, dropped {:?}", control);
    }
}
