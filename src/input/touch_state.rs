//! Convert controller samples into one contact's down/move/up lifecycle.
use super::cst9220::Report;
use crate::control::{Touch, TouchPhase};
use crate::orientation;

pub struct Contact {
    last: Option<(u16, u16)>,
}

impl Contact {
    pub const fn new() -> Self {
        Self { last: None }
    }
    pub fn active(&self) -> bool {
        self.last.is_some()
    }

    pub fn sample(&mut self, report: Report) -> Option<Touch> {
        let (x, y) = match report {
            Report::Released => return self.release(),
            Report::Invalid => return self.cancel(),
            Report::NotReady => return None,
            Report::Contact { x, y } => (x, y),
        };
        // Store logical coordinates so moves, releases and cancels all match
        // the mounted display. Invalid samples cancel without hitting a button.
        let Some(point) = orientation::point(x, y) else {
            return self.cancel();
        };
        if self.last == Some(point) {
            return None;
        }
        let phase = if self.last.is_some() {
            TouchPhase::Move
        } else {
            TouchPhase::Down
        };
        self.last = Some(point);
        Some(Touch {
            phase,
            x: point.0,
            y: point.1,
        })
    }

    pub fn release(&mut self) -> Option<Touch> {
        self.finish(TouchPhase::Up)
    }
    pub fn cancel(&mut self) -> Option<Touch> {
        self.finish(TouchPhase::Cancel)
    }
    fn finish(&mut self, phase: TouchPhase) -> Option<Touch> {
        self.last.take().map(|(x, y)| Touch { phase, x, y })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Explicit logical oracle per mounting, never computed by the mapping
    /// under test: (upright LilyGo board, half-turned Rev A hardware).
    fn logical(upright: (u16, u16), turned: (u16, u16)) -> (u16, u16) {
        if orientation::ROTATE_180 {
            turned
        } else {
            upright
        }
    }
    #[test]
    fn tracks_contact_and_uses_last_valid_position_on_release() {
        let mut c = Contact::new();
        let sample = Report::Contact { x: 20, y: 400 };
        let (x, y) = logical((20, 400), (459, 79));
        assert_eq!(
            c.sample(sample),
            Some(Touch {
                phase: TouchPhase::Down,
                x,
                y,
            })
        );
        assert_eq!(c.sample(sample), None);
        let (x, y) = logical((30, 400), (449, 79));
        assert_eq!(
            c.sample(Report::Contact { x: 30, y: 400 }),
            Some(Touch {
                phase: TouchPhase::Move,
                x,
                y,
            })
        );
        assert_eq!(
            c.sample(Report::Released),
            Some(Touch {
                phase: TouchPhase::Up,
                x,
                y
            })
        );
        assert_eq!(c.release(), None);
    }
    #[test]
    fn invalid_samples_cancel_without_a_false_tap() {
        let (x, y) = logical((400, 440), (79, 39));
        for invalid in [
            Report::Invalid,
            Report::Contact { x: 480, y: 2 },
            Report::Contact { x: 2, y: 480 },
            Report::Contact {
                x: u16::MAX,
                y: u16::MAX,
            },
        ] {
            let mut c = Contact::new();
            assert_eq!(c.sample(invalid), None);
            c.sample(Report::Contact { x: 400, y: 440 });
            assert_eq!(
                c.sample(invalid),
                Some(Touch {
                    phase: TouchPhase::Cancel,
                    x,
                    y,
                })
            );
            assert!(!c.active());
            assert_eq!(c.sample(Report::Released), None);
            assert_eq!(
                c.sample(Report::Contact { x: 400, y: 440 }).unwrap().phase,
                TouchPhase::Down
            );
        }
    }

    #[test]
    fn polling_between_reports_preserves_a_real_hold() {
        let mut c = Contact::new();
        c.sample(Report::Contact { x: 400, y: 440 });
        assert_eq!(c.sample(Report::NotReady), None);
        assert!(c.active());
        let (x, y) = logical((400, 440), (79, 39));
        assert_eq!(
            c.sample(Report::Released),
            Some(Touch {
                phase: TouchPhase::Up,
                x,
                y,
            })
        );
    }
}
