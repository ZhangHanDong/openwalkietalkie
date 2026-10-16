//! Bluetooth link quality, independent of the USB/host connection. The
//! runtimes sample RSSI; this module filters it and draws the shared status.
use super::{kit, layout, signature::SigBuilder, theme};
use crate::display::Band;
use embedded_graphics::{
    prelude::*,
    primitives::{Line, PrimitiveStyle},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    /// Connected, but no valid measurement yet (or the last read failed).
    Unknown,
    Signal(Level),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Level {
    Weak = 1,
    Fair = 2,
    Good = 3,
    Strong = 4,
}

impl Status {
    fn key(self) -> u32 {
        match self {
            Self::Disconnected => 0,
            Self::Unknown => 1,
            Self::Signal(level) => 1 + level as u32,
        }
    }
}

/// Half-weight moving average in quarter dBm, with 3 dB hysteresis at each
/// bar boundary. A new connection must start with a fresh filter. An invalid
/// or missing reading clears it so stale bars never imply a healthy signal.
pub struct RssiFilter {
    average: Option<i16>,
    bars: u8,
}

impl RssiFilter {
    pub const fn new() -> Self {
        Self {
            average: None,
            bars: 0,
        }
    }

    pub fn sample(&mut self, rssi: Option<i8>) -> Status {
        // HCI Read RSSI uses 127 for unavailable; -128 is reserved.
        let Some(rssi @ -127..=20) = rssi else {
            *self = Self::new();
            return Status::Unknown;
        };
        const BOUNDARIES: [i16; 3] = [-85 * 4, -70 * 4, -55 * 4];
        let sample = i16::from(rssi) * 4;
        let average = match self.average {
            Some(previous) => previous + (sample - previous) / 2,
            None => {
                self.bars = 1 + BOUNDARIES.iter().filter(|&&b| sample >= b).count() as u8;
                sample
            }
        };
        self.average = Some(average);
        while self.bars < 4 && average >= BOUNDARIES[self.bars as usize - 1] + 12 {
            self.bars += 1;
        }
        while self.bars > 1 && average < BOUNDARIES[self.bars as usize - 2] - 12 {
            self.bars -= 1;
        }
        Status::Signal(match self.bars {
            1 => Level::Weak,
            2 => Level::Fair,
            3 => Level::Good,
            _ => Level::Strong,
        })
    }
}

pub fn draw(band: &mut Band, status: Status) {
    if !kit::visible(band, layout::BLUETOOTH) {
        return;
    }
    let color = match status {
        Status::Disconnected => theme::TEXT_FAINT,
        Status::Signal(Level::Weak) => theme::AMBER,
        _ => theme::BLUE,
    };
    let style = PrimitiveStyle::with_stroke(kit::rgb(color), 2);
    // The Bluetooth rune, above the header and clear of the notice marker.
    for ((x0, y0), (x1, y1)) in [
        ((383, 11), (383, 27)),
        ((383, 11), (389, 17)),
        ((389, 17), (377, 23)),
        ((377, 15), (389, 21)),
        ((389, 21), (383, 27)),
    ] {
        let _ = Line::new(Point::new(x0, y0), Point::new(x1, y1))
            .into_styled(style)
            .draw(band);
    }
    if status == Status::Disconnected {
        kit::fill(band, layout::Rect::new(402, 22, 16, 2), color);
        return;
    }
    let bars = match status {
        Status::Signal(level) => level as i32,
        _ => 0,
    };
    for i in 0..4 {
        let height = (i + 1) * 4;
        kit::fill(
            band,
            layout::Rect::new(398 + i * 7, 28 - height, 4, height),
            if i < bars {
                color
            } else {
                theme::OUTLINE_THUMB
            },
        );
    }
}

pub fn signature(status: Status, sig: &mut SigBuilder) {
    sig.region(layout::BLUETOOTH, status.key());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_read_covers_the_full_rssi_range() {
        for (dbm, level) in [
            (-127, Level::Weak),
            (-86, Level::Weak),
            (-85, Level::Fair),
            (-71, Level::Fair),
            (-70, Level::Good),
            (-56, Level::Good),
            (-55, Level::Strong),
            (20, Level::Strong),
        ] {
            assert_eq!(RssiFilter::new().sample(Some(dbm)), Status::Signal(level));
        }
    }

    #[test]
    fn boundary_noise_does_not_flicker_but_sustained_changes_do() {
        let mut filter = RssiFilter::new();
        assert_eq!(filter.sample(Some(-70)), Status::Signal(Level::Good));
        for dbm in [-71, -69, -72, -70, -71, -68] {
            assert_eq!(filter.sample(Some(dbm)), Status::Signal(Level::Good));
        }
        for _ in 0..8 {
            filter.sample(Some(-95));
        }
        assert_eq!(filter.sample(Some(-95)), Status::Signal(Level::Weak));
        for _ in 0..8 {
            filter.sample(Some(-40));
        }
        assert_eq!(filter.sample(Some(-40)), Status::Signal(Level::Strong));
    }

    #[test]
    fn missing_or_invalid_rssi_clears_the_previous_signal() {
        let mut filter = RssiFilter::new();
        for invalid in [None, Some(127), Some(-128), Some(21)] {
            assert_eq!(filter.sample(Some(-40)), Status::Signal(Level::Strong));
            assert_eq!(filter.sample(invalid), Status::Unknown);
            assert_eq!(filter.sample(Some(-95)), Status::Signal(Level::Weak));
            // Start the next case with a fresh filter too.
            filter = RssiFilter::new();
        }
    }
}
