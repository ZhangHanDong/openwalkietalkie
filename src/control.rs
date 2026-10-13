//! The product's input vocabulary.
//!
//! OpenWalkieTalkie has a touch screen, a scroll wheel and six buttons. This
//! module is the seam between "what the user did" and "which chip noticed":
//! everything above it is written against `Control`, so the application does
//! not know or care whether a detent came from a quadrature encoder on the
//! real hardware or from an arrow key on the development keypad standing in
//! for one (see `input::keypad`).
//!
//! Keeping that seam honest is the whole point of developing on the LilyGo
//! board: when the real hardware arrives, only `input` changes.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

mod types;
pub use types::*;

/// Depth is generous relative to how fast a human can act; if this ever backs
/// up it means a consumer is blocked, which is worth noticing rather than
/// absorbing silently.
const DEPTH: usize = 16;

/// A control and the millisecond the input task saw it. The app task can be
/// a whole frame behind, and the chord timings (double-SHIFT) are measured
/// from these stamps, not from when the control was dequeued.
pub type Stamped = (Control, u32);

pub static CONTROLS: Channel<CriticalSectionRawMutex, Stamped, DEPTH> = Channel::new();

pub fn now_ms() -> u32 {
    embassy_time::Instant::now().as_millis() as u32
}

pub fn stamp(control: Control) -> Stamped {
    (control, now_ms())
}
