//! Input sources.
//!
//! Everything here converts some piece of hardware into `control::Control` and
//! pushes it onto `control::CONTROLS`. Nothing above this module names a chip.
//!
//! The development board (default build) stands its T-SF32-Keyboard in for
//! the wheel and buttons (`keypad`); the Rev A hardware (`--features hw-a`)
//! reads them from GPIOs (`gpio`, decoded by `direct`). Touch is the same
//! controller on both, on different pins (`board`).

pub mod cst9220;
#[cfg(feature = "hw-a")]
pub mod direct;
#[cfg(feature = "hw-a")]
pub mod gpio;
#[cfg(not(feature = "hw-a"))]
pub mod keymap;
#[cfg(not(feature = "hw-a"))]
pub mod keypad;
#[cfg(not(feature = "hw-a"))]
pub mod tca8418;
pub mod touch;
pub mod touch_state;
#[cfg(not(feature = "hw-a"))]
pub mod xl9555;

#[cfg(not(feature = "hw-a"))]
use embedded_hal::i2c::I2c;

#[cfg(not(feature = "hw-a"))]
use crate::logln;

/// AW21009, the keypad's LED driver. Nothing here drives it yet; it is listed
/// so a boot probe can tell "the keyboard board is present but its LED rail is
/// still off" apart from "the keyboard board is not plugged in".
#[cfg(not(feature = "hw-a"))]
pub const AW21009_ADDR: u8 = 0x25;

/// Devices this firmware expects to find on I2C1.
#[cfg(not(feature = "hw-a"))]
const KNOWN: &[(u8, &str)] = &[
    (xl9555::ADDR, "XL9555 GPIO expander (keyboard board)"),
    (AW21009_ADDR, "AW21009 keypad LED driver (keyboard board)"),
    (tca8418::ADDR, "TCA8418 keypad scanner (keyboard board)"),
];

/// Address-probe one device: a zero-length write either gets an ACK or not.
#[cfg(not(feature = "hw-a"))]
pub fn probe<I: I2c>(i2c: &mut I, addr: u8) -> bool {
    i2c.write(addr, &[]).is_ok()
}

/// Probe the addresses this firmware depends on and log which of them answered.
///
/// Deliberately *not* a full 0x08..=0x77 sweep. Probing address 0x5A on this
/// board wedges sifli-hal's blocking I2C driver outright -- it never returns,
/// with no timeout -- and since the boot path runs before the keypad task, a
/// blind sweep silently takes the whole firmware down at 0x5A having logged
/// everything below it, which reads exactly like a crash somewhere else.
///
/// The CST9220 touch controller is at 0x5A and needs two-byte commands.
/// The touch task identifies it after reset with its documented protocol.
#[cfg(not(feature = "hw-a"))]
pub fn probe_known<I: I2c>(i2c: &mut I) -> u32 {
    let mut found = 0;
    for (addr, name) in KNOWN {
        if probe(i2c, *addr) {
            logln!("  0x{:02X} {}", addr, name);
            found += 1;
        } else {
            logln!("  0x{:02X} -- absent ({})", addr, name);
        }
    }
    found
}
