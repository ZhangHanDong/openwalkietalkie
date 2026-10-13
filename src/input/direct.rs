//! Direct inputs of the Rev A hardware: six buttons and the wheel's quadrature
//! encoder, decoded from raw pin levels into `Control`s.
//!
//! Hardware-free, so the Embassy task (`gpio.rs`), the native runtime and the
//! host tests share one decoder. The runtime samples the eight pins into a
//! `Raw` mask and hands it here with the millisecond clock; this module does
//! the debouncing and the quadrature decoding and says how soon it wants the
//! next sample.
//!
//! Wiring (`hardware/rev-a/.../source/main/src/main.cohdl`, `controls.cohdl`):
//! every input is pulled up to IO_3V3 through 100k on the board and pulled low
//! by its contact, through 1k with 1nF to ground at the MCU pin. That RC is
//! EMI filtering (1 µs), not debouncing, which happens here. The encoder is
//! an Alps EC05E1220401: 12 detents and 12 pulses per turn, so one detent is
//! one complete quadrature cycle and the contacts rest open at a detent.

use crate::control::{Button, Control, Edge, Wheel};

/// Bit order of a `Raw` sample, LSB first. A set bit is a high (released,
/// open) pin.
pub const PTT: u8 = 1 << 0;
pub const ENTER: u8 = 1 << 1;
pub const ESC: u8 = 1 << 2;
pub const SHIFT: u8 = 1 << 3;
pub const LEFT: u8 = 1 << 4;
pub const RIGHT: u8 = 1 << 5;
pub const ENC_A: u8 = 1 << 6;
pub const ENC_B: u8 = 1 << 7;

/// All six button bits.
pub const BUTTON_BITS: u8 = PTT | ENTER | ESC | SHIFT | LEFT | RIGHT;

/// Raw levels of the eight pins, `PTT` .. `ENC_B`. Everything released and
/// the wheel at a detent reads `0xFF`.
pub type Raw = u8;

/// A button has to hold a new level this long before it counts. Alps rates
/// the SKTD and PTS636 bounce under 5 ms; 8 leaves margin without making a
/// tap feel late.
pub const DEBOUNCE_MS: u32 = 8;
/// How often the decoder wants sampling while something is changing.
pub const SETTLE_POLL_MS: u32 = 2;
/// How long after the last change it keeps asking for fast samples: long
/// enough to see the end of a slow detent, short enough to go quiet between
/// presses.
pub const SETTLE_MS: u32 = 30;

/// Which encoder phase leads for a clockwise detent. Clockwise is "forward"
/// (down a list, right along the face strip). The sense depends on which
/// way the wheel sits over the encoder, and the `encoder-wheel/` board swaps
/// A/B in copper to keep it the same, so this is verified on the bench
/// (bring-up guide, Stage 3), not derived.
pub const A_LEADS_CLOCKWISE: bool = true;

/// SHIFT first: within one sample presses go out in this order and releases
/// in the reverse, so a chord whose two edges settle together reaches the UI
/// shifted (the UI latches the modifier on its press).
const BUTTONS: [(u8, Button); 6] = [
    (SHIFT, Button::Shift),
    (PTT, Button::PushToTalk),
    (ENTER, Button::Enter),
    (ESC, Button::Cancel),
    (LEFT, Button::Left),
    (RIGHT, Button::Right),
];

/// The encoder's resting state: both contacts open.
const DETENT: u8 = 0b11;

/// The controls one sample produced: at most one edge per button, plus one
/// detent per quadrature cycle completed since the last resting sample. Six
/// edges and a fast flick's worth of detents fit; beyond that the extra
/// detents are dropped rather than mis-ordered.
pub struct Events {
    items: [Option<Control>; 16],
    len: usize,
}
impl Events {
    const fn new() -> Self {
        Self {
            items: [None; 16],
            len: 0,
        }
    }
    fn push(&mut self, control: Control) {
        if self.len < self.items.len() {
            self.items[self.len] = Some(control);
            self.len += 1;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = Control> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }
    #[allow(dead_code)] // The runtimes iterate; the tests count.
    pub fn len(&self) -> usize {
        self.len
    }
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub struct Decoder {
    /// Debounced button levels (set = released).
    stable: u8,
    /// Last raw button levels, and when each bit last changed.
    raw: u8,
    changed_at: [u32; 6],
    /// Last encoder state, A in bit 1 and B in bit 0.
    phase: u8,
    /// Signed quadrature steps since the last detent; a full cycle is ±4.
    acc: i8,
    /// When anything last changed, for `busy`.
    last_change: u32,
}

impl Decoder {
    /// Start from the levels as they are: a button held through boot does
    /// nothing until it is released, and a wheel resting between detents
    /// counts from where it is.
    pub const fn new(raw: Raw, now: u32) -> Self {
        Self {
            stable: raw & BUTTON_BITS,
            raw: raw & BUTTON_BITS,
            changed_at: [now; 6],
            phase: encoder_phase(raw),
            acc: 0,
            last_change: now,
        }
    }

    /// True while the decoder wants to be sampled every `SETTLE_POLL_MS`:
    /// a button level that has not settled yet, the wheel away from a
    /// detent, or anything that moved within the last `SETTLE_MS`.
    pub fn busy(&self, now: u32) -> bool {
        self.raw != self.stable
            || self.phase != DETENT
            || now.wrapping_sub(self.last_change) < SETTLE_MS
    }

    /// Debounced button levels, set = released. For logging and tests.
    pub fn held(&self) -> u8 {
        !self.stable & BUTTON_BITS
    }

    /// Take one sample of all eight pins at `now`.
    pub fn sample(&mut self, raw: Raw, now: u32) -> Events {
        let mut events = Events::new();
        self.buttons(raw & BUTTON_BITS, now, &mut events);
        self.encoder(encoder_phase(raw), now, &mut events);
        events
    }

    fn buttons(&mut self, raw: u8, now: u32, events: &mut Events) {
        let mut pressed = 0u8;
        let mut released = 0u8;
        for (index, (bit, _)) in BUTTONS.iter().enumerate() {
            if (raw ^ self.raw) & bit != 0 {
                // Level moved (or bounced): the window restarts.
                self.raw ^= bit;
                self.changed_at[index] = now;
                self.last_change = now;
            }
            if (self.raw ^ self.stable) & bit != 0
                && now.wrapping_sub(self.changed_at[index]) >= DEBOUNCE_MS
            {
                self.stable ^= bit;
                if self.stable & bit == 0 {
                    pressed |= bit;
                } else {
                    released |= bit;
                }
            }
        }
        // Modifier first on the way down, last on the way up.
        for (bit, button) in BUTTONS {
            if pressed & bit != 0 {
                events.push(Control::Button(button, Edge::Press));
            }
        }
        for (bit, button) in BUTTONS.iter().rev() {
            if released & bit != 0 {
                events.push(Control::Button(*button, Edge::Release));
            }
        }
    }

    fn encoder(&mut self, phase: u8, now: u32, events: &mut Events) {
        if phase == self.phase {
            return;
        }
        self.last_change = now;
        // Gray-code order 00 -> 01 -> 11 -> 10 -> 00 counts +1 per step (B
        // leads A); the other way round is -1. Both bits changing at once
        // means a sample was missed and two transitions passed; they are
        // credited to the direction the run was already going, and count 0
        // when it is the first move out of the detent (direction unknown).
        let delta = match (self.phase, phase) {
            (0b00, 0b01) | (0b01, 0b11) | (0b11, 0b10) | (0b10, 0b00) => 1,
            (0b01, 0b00) | (0b11, 0b01) | (0b10, 0b11) | (0b00, 0b10) => -1,
            _ => 2 * self.acc.signum(),
        };
        self.acc = self.acc.saturating_add(delta);
        self.phase = phase;
        if phase != DETENT {
            return;
        }
        // Back at a detent. A full cycle is ±4. ±2 is a cycle whose first
        // move out of the detent was a missed sample, and is taken as one
        // detent: the one case this misreads is a nudge to the hump that
        // snaps back with the return sample missed, which costs a spurious
        // step, whereas demanding ±4 would lose real detents whenever a
        // sample lands late (a redraw runs between samples on both
        // runtimes). ±1 or 0 is a wiggle that came back without going
        // round. Several cycles pile up when the resting sample itself was
        // missed during a fast spin, so one detent is emitted per cycle held.
        let acc = core::mem::take(&mut self.acc);
        let magnitude = acc.unsigned_abs();
        if magnitude < 2 {
            return;
        }
        let a_leads = acc < 0;
        let wheel = if a_leads == A_LEADS_CLOCKWISE {
            Wheel::Cw
        } else {
            Wheel::Ccw
        };
        for _ in 0..(magnitude + 2) / 4 {
            events.push(Control::Wheel(wheel));
        }
    }
}

/// The encoder's two bits of a raw sample, A in bit 1 and B in bit 0.
const fn encoder_phase(raw: Raw) -> u8 {
    ((raw & ENC_A != 0) as u8) << 1 | (raw & ENC_B != 0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Raw = 0xFF;

    fn collect(events: Events) -> std::vec::Vec<Control> {
        events.iter().collect()
    }

    #[test]
    fn the_eight_bits_are_distinct_and_cover_the_mask() {
        let bits = [PTT, ENTER, ESC, SHIFT, LEFT, RIGHT, ENC_A, ENC_B];
        for (i, a) in bits.iter().enumerate() {
            assert_eq!(a.count_ones(), 1);
            for b in &bits[i + 1..] {
                assert_eq!(a & b, 0);
            }
        }
        assert_eq!(bits.iter().fold(0, |m, b| m | b), 0xFF);
        assert_eq!(BUTTON_BITS, 0x3F);
        assert_eq!(encoder_phase(IDLE), DETENT);
        assert_eq!(encoder_phase(IDLE & !ENC_A), 0b01);
        assert_eq!(encoder_phase(IDLE & !ENC_B), 0b10);
    }

    #[test]
    fn a_press_and_a_release_are_reported_once_each_after_the_window() {
        let mut d = Decoder::new(IDLE, 0);
        assert!(collect(d.sample(IDLE & !PTT, 0)).is_empty());
        assert!(d.busy(0));
        assert!(collect(d.sample(IDLE & !PTT, DEBOUNCE_MS - 1)).is_empty());
        assert_eq!(
            collect(d.sample(IDLE & !PTT, DEBOUNCE_MS)),
            [Control::Button(Button::PushToTalk, Edge::Press)]
        );
        assert_eq!(d.held(), PTT);
        // Holding it produces nothing more.
        for t in DEBOUNCE_MS + 1..200 {
            assert!(collect(d.sample(IDLE & !PTT, t)).is_empty());
        }
        assert!(collect(d.sample(IDLE, 200)).is_empty());
        assert_eq!(
            collect(d.sample(IDLE, 200 + DEBOUNCE_MS)),
            [Control::Button(Button::PushToTalk, Edge::Release)]
        );
        assert_eq!(d.held(), 0);
        // Quiet again once the settle time has passed.
        assert!(d.busy(200 + DEBOUNCE_MS));
        assert!(!d.busy(200 + DEBOUNCE_MS + SETTLE_MS));
    }

    #[test]
    fn a_bounce_shorter_than_the_window_is_nothing() {
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for (t, raw) in [(0, IDLE & !ENTER), (3, IDLE), (5, IDLE & !ENTER), (7, IDLE)] {
            out.extend(collect(d.sample(raw, t)));
        }
        for t in 8..100 {
            out.extend(collect(d.sample(IDLE, t)));
        }
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(d.held(), 0);
    }

    #[test]
    fn a_bounce_on_the_way_down_delays_the_press_but_keeps_it() {
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for (t, raw) in [(0, IDLE & !SHIFT), (3, IDLE), (4, IDLE & !SHIFT)] {
            out.extend(collect(d.sample(raw, t)));
        }
        assert!(out.is_empty());
        assert!(collect(d.sample(IDLE & !SHIFT, 4 + DEBOUNCE_MS - 1)).is_empty());
        assert_eq!(
            collect(d.sample(IDLE & !SHIFT, 4 + DEBOUNCE_MS)),
            [Control::Button(Button::Shift, Edge::Press)]
        );
    }

    #[test]
    fn every_button_maps_to_its_own_bit() {
        for (bit, button) in BUTTONS {
            let mut d = Decoder::new(IDLE, 0);
            d.sample(IDLE & !bit, 0);
            assert_eq!(
                collect(d.sample(IDLE & !bit, DEBOUNCE_MS)),
                [Control::Button(button, Edge::Press)]
            );
        }
        // Two buttons at once are two independent presses (SHIFT chords),
        // the modifier first so the UI sees the chord shifted.
        let mut d = Decoder::new(IDLE, 0);
        d.sample(IDLE & !SHIFT & !ENTER, 0);
        assert_eq!(
            collect(d.sample(IDLE & !SHIFT & !ENTER, DEBOUNCE_MS)),
            [
                Control::Button(Button::Shift, Edge::Press),
                Control::Button(Button::Enter, Edge::Press),
            ]
        );
        // And the modifier last when both let go together.
        d.sample(IDLE, 100);
        assert_eq!(
            collect(d.sample(IDLE, 100 + DEBOUNCE_MS)),
            [
                Control::Button(Button::Enter, Edge::Release),
                Control::Button(Button::Shift, Edge::Release),
            ]
        );
    }

    #[test]
    fn a_chord_whose_edges_settle_together_after_a_bounce_is_still_shift_first() {
        // SHIFT lands first but bounces, so its window ends with ENTER's.
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for (t, raw) in [(0, IDLE & !SHIFT), (3, IDLE), (6, IDLE & !SHIFT & !ENTER)] {
            out.extend(collect(d.sample(raw, t)));
        }
        assert!(out.is_empty());
        assert_eq!(
            collect(d.sample(IDLE & !SHIFT & !ENTER, 6 + DEBOUNCE_MS)),
            [
                Control::Button(Button::Shift, Edge::Press),
                Control::Button(Button::Enter, Edge::Press),
            ]
        );
    }

    #[test]
    fn a_button_held_through_boot_does_not_act_until_pressed_again() {
        let mut d = Decoder::new(IDLE & !PTT, 0);
        assert!(collect(d.sample(IDLE & !PTT, 50)).is_empty());
        assert_eq!(d.held(), PTT);
        d.sample(IDLE, 100);
        assert_eq!(
            collect(d.sample(IDLE, 100 + DEBOUNCE_MS)),
            [Control::Button(Button::PushToTalk, Edge::Release)]
        );
    }

    /// A leads: A opens/closes before B leaving the detent.
    fn a_leads_cycle() -> [Raw; 4] {
        [IDLE & !ENC_A, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_B, IDLE]
    }
    fn b_leads_cycle() -> [Raw; 4] {
        [IDLE & !ENC_B, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_A, IDLE]
    }
    fn expected(a_leads: bool) -> Control {
        Control::Wheel(if a_leads == A_LEADS_CLOCKWISE {
            Wheel::Cw
        } else {
            Wheel::Ccw
        })
    }

    #[test]
    fn one_full_cycle_is_one_detent_in_each_direction() {
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for (i, raw) in a_leads_cycle().into_iter().enumerate() {
            out.extend(collect(d.sample(raw, 10 * i as u32)));
        }
        assert_eq!(out, [expected(true)]);
        out.clear();
        for (i, raw) in b_leads_cycle().into_iter().enumerate() {
            out.extend(collect(d.sample(raw, 100 + 10 * i as u32)));
        }
        assert_eq!(out, [expected(false)]);
        // Several detents in a row, as fast as the wheel can be spun.
        out.clear();
        let mut t = 200;
        for _ in 0..12 {
            for raw in a_leads_cycle() {
                out.extend(collect(d.sample(raw, t)));
                t += 1;
            }
        }
        assert_eq!(out.len(), 12);
        assert!(out.iter().all(|c| *c == expected(true)));
    }

    #[test]
    fn going_part_way_and_back_is_nothing() {
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for raw in [IDLE & !ENC_A, IDLE, IDLE & !ENC_A, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_A, IDLE] {
            out.extend(collect(d.sample(raw, 0)));
        }
        assert!(out.is_empty(), "{out:?}");
        assert!(!d.busy(SETTLE_MS));
    }

    #[test]
    fn a_missed_sample_still_counts_the_detent_with_the_right_sign() {
        // Detent -> both closed (two transitions in one sample) -> A opens -> detent.
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for raw in [IDLE & !ENC_A & !ENC_B, IDLE & !ENC_B, IDLE] {
            out.extend(collect(d.sample(raw, 0)));
        }
        assert_eq!(out, [expected(true)]);
        // The mirror image.
        out.clear();
        for raw in [IDLE & !ENC_A & !ENC_B, IDLE & !ENC_A, IDLE] {
            out.extend(collect(d.sample(raw, 0)));
        }
        assert_eq!(out, [expected(false)]);
    }

    #[test]
    fn detents_whose_resting_sample_was_missed_are_all_counted() {
        // Two detents, B leading, with the 11 between them never sampled:
        // 10 -> 00 -> 01 -> (11 missed) -> 10 -> 00 -> 01 -> 11.
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        let step = [IDLE & !ENC_B, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_A];
        for raw in step.iter().chain(step.iter()).chain([IDLE].iter()) {
            out.extend(collect(d.sample(*raw, 0)));
        }
        assert_eq!(out, [expected(false), expected(false)]);
        // Three detents with two resting samples missed, A leading.
        out.clear();
        let step = [IDLE & !ENC_A, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_B];
        for raw in step.iter().chain(step.iter()).chain(step.iter()).chain([IDLE].iter()) {
            out.extend(collect(d.sample(*raw, 0)));
        }
        assert_eq!(out, [expected(true), expected(true), expected(true)]);
    }

    #[test]
    fn a_nudge_that_snaps_back_with_a_missed_sample_reads_as_one_detent() {
        // The documented trade-off of the ±2 rule: 11 -> 10 -> 00 -> 11, the
        // return through 10 never sampled, is indistinguishable from a cycle
        // with one transition missed and counts as a detent.
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for raw in [IDLE & !ENC_B, IDLE & !ENC_A & !ENC_B, IDLE] {
            out.extend(collect(d.sample(raw, 0)));
        }
        assert_eq!(out, [expected(false)]);
    }

    #[test]
    fn a_saturated_accumulator_neither_panics_nor_floods() {
        // 200 cycles without ever sampling the detent saturates the i8.
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        for _ in 0..200 {
            for raw in [IDLE & !ENC_A, IDLE & !ENC_A & !ENC_B, IDLE & !ENC_B] {
                out.extend(collect(d.sample(raw, 0)));
            }
        }
        assert!(out.is_empty());
        let events = d.sample(IDLE, 0);
        assert!(!events.is_empty());
        assert!(events.len() <= 16);
        assert!(events.iter().all(|c| c == expected(true)));
    }

    #[test]
    fn a_button_edge_and_a_detent_in_one_sample_come_out_in_order() {
        let mut d = Decoder::new(IDLE, 0);
        // SHIFT goes down as the wheel leaves the detent; its window ends on
        // the same sample that completes the cycle.
        let cycle = a_leads_cycle();
        assert!(collect(d.sample(cycle[0] & !SHIFT, 0)).is_empty());
        assert!(collect(d.sample(cycle[1] & !SHIFT, 2)).is_empty());
        assert!(collect(d.sample(cycle[2] & !SHIFT, 4)).is_empty());
        let events = d.sample(cycle[3] & !SHIFT, DEBOUNCE_MS);
        assert_eq!(events.len(), 2);
        assert_eq!(
            collect(events),
            [Control::Button(Button::Shift, Edge::Press), expected(true)]
        );
    }

    #[test]
    fn contact_bounce_inside_a_cycle_is_ignored() {
        let mut d = Decoder::new(IDLE, 0);
        let mut out = std::vec::Vec::new();
        let seq = [
            IDLE & !ENC_A,
            IDLE,
            IDLE & !ENC_A,
            IDLE & !ENC_A & !ENC_B,
            IDLE & !ENC_A,
            IDLE & !ENC_A & !ENC_B,
            IDLE & !ENC_B,
            IDLE & !ENC_A & !ENC_B,
            IDLE & !ENC_B,
            IDLE,
            IDLE & !ENC_B,
            IDLE,
        ];
        for raw in seq {
            out.extend(collect(d.sample(raw, 0)));
        }
        assert_eq!(out, [expected(true)]);
    }

    #[test]
    fn the_wheel_and_a_button_in_one_sample_are_both_reported() {
        let mut d = Decoder::new(IDLE, 0);
        d.sample(IDLE & !SHIFT, 0);
        d.sample(IDLE & !SHIFT, DEBOUNCE_MS);
        let mut out = std::vec::Vec::new();
        for raw in a_leads_cycle() {
            out.extend(collect(d.sample(raw & !SHIFT, 20)));
        }
        assert_eq!(out, [expected(true)]);
        assert_eq!(d.held(), SHIFT);
        assert!(d.busy(20));
    }

    #[test]
    fn busy_follows_settling_and_the_detent() {
        let mut d = Decoder::new(IDLE, 0);
        assert!(d.busy(0));
        assert!(!d.busy(SETTLE_MS));
        d.sample(IDLE & !ENC_A, 1000);
        assert!(d.busy(1000 + SETTLE_MS + 1), "away from the detent");
        d.sample(IDLE, 2000);
        assert!(d.busy(2000));
        assert!(!d.busy(2000 + SETTLE_MS));
        assert!(d.sample(IDLE, 3000).is_empty());
    }

    #[test]
    fn timing_constants_fit_together() {
        assert!(SETTLE_POLL_MS < DEBOUNCE_MS);
        assert!(DEBOUNCE_MS < SETTLE_MS);
        assert!(SETTLE_POLL_MS <= crate::power::ACTIVE_POLL_MS);
    }
}
