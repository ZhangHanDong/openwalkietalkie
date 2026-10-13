//! Development keypad (T-SF32-Keyboard, TCA8418 key numbers) -> product
//! controls. Shared by both runtimes so they cannot disagree about a key, or
//! about which way a held wheel key keeps turning.
use crate::control::{Button, Wheel};

/// Held wheel keys repeat like a spun encoder. The cap stops a wheel whose
/// release was lost in the scanner's ten-event FIFO.
pub const REPEAT_DELAY_MS: u32 = 400;
pub const REPEAT_PERIOD_MS: u32 = 90;
pub const REPEAT_MAX_MS: u32 = 10_000;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mapped {
    /// One detent per press, then auto-repeat. A detent has no release, and
    /// repeat state stores this direction, never the key number.
    Wheel(Wheel),
    /// Both edges matter.
    Button(Button),
    None,
}

pub const fn map(key: u8) -> Mapped {
    match key {
        21 => Mapped::Wheel(Wheel::Cw),           // `v` down / NEXT
        41 => Mapped::Wheel(Wheel::Ccw),          // `^` up / PREV
        31 => Mapped::Button(Button::Enter),      // `.` dot
        42 => Mapped::Button(Button::Cancel),     // ESC
        43 => Mapped::Button(Button::Shift),      // HOME
        44 => Mapped::Button(Button::PushToTalk), // MAIL (envelope)
        32 => Mapped::Button(Button::Left),       // '1'
        33 => Mapped::Button(Button::Right),      // '2'
        _ => Mapped::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_binds_exactly_the_eight_documented_keys() {
        let table = [
            (21, Mapped::Wheel(Wheel::Cw)),
            (41, Mapped::Wheel(Wheel::Ccw)),
            (31, Mapped::Button(Button::Enter)),
            (42, Mapped::Button(Button::Cancel)),
            (43, Mapped::Button(Button::Shift)),
            (44, Mapped::Button(Button::PushToTalk)),
            (32, Mapped::Button(Button::Left)),
            (33, Mapped::Button(Button::Right)),
        ];
        for key in 0..=u8::MAX {
            let expected = table
                .iter()
                .find(|(bound, _)| *bound == key)
                .map_or(Mapped::None, |(_, mapped)| *mapped);
            assert_eq!(map(key), expected, "key {key}");
        }
        // The old cross key (Decline) is deliberately unbound.
        assert_eq!(map(11), Mapped::None);
        // Every button is reachable from exactly one key.
        for button in Button::ALL {
            let keys = (0..=u8::MAX).filter(|key| map(*key) == Mapped::Button(button));
            assert_eq!(keys.count(), 1, "{button:?}");
        }
        for wheel in [Wheel::Cw, Wheel::Ccw] {
            let keys = (0..=u8::MAX).filter(|key| map(*key) == Mapped::Wheel(wheel));
            assert_eq!(keys.count(), 1, "{wheel:?}");
        }
    }

    #[test]
    fn repeat_timing_matches_a_held_key_not_a_stuck_one() {
        assert!(REPEAT_PERIOD_MS < REPEAT_DELAY_MS);
        assert!(REPEAT_DELAY_MS < REPEAT_MAX_MS);
        assert_eq!(
            (REPEAT_DELAY_MS, REPEAT_PERIOD_MS, REPEAT_MAX_MS),
            (400, 90, 10_000)
        );
    }
}
