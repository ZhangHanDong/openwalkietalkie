//! Battery readings and a deliberately approximate voltage-based charge estimate.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeState {
    Unknown,
    Battery,
    Usb,
    Charging,
    Full,
}

impl ChargeState {
    /// SGM41562S REG08: PG_STAT bit 1; CHG_STAT bits 4:3.
    pub fn from_register(status: u8) -> Self {
        if status & 0x02 == 0 {
            return Self::Battery;
        }
        match (status >> 3) & 3 {
            1 | 2 => Self::Charging,
            3 => Self::Full,
            _ => Self::Usb,
        }
    }

    /// The REG08 encoding of a state a charger without a bus reports (Rev
    /// B's BQ24073, through its CHG pin), so the estimator qualifies a
    /// termination the same way for both chargers. `from_register` inverts
    /// it, except that `Unknown` comes back as `Battery`.
    #[cfg_attr(not(feature = "hw-b"), allow(dead_code))]
    pub fn as_register(self) -> u8 {
        match self {
            Self::Unknown | Self::Battery => 0x00,
            Self::Usb => 0x02,
            Self::Charging => 0x02 | (2 << 3),
            Self::Full => 0x02 | (3 << 3),
        }
    }

    /// The state behind a BQ24073's CHG pin, which is low (`charging`) while
    /// the cell takes current and high at termination, in a fault and with
    /// no input; only a USB host tells the last apart (`usb_host`: the
    /// chip's VBUS pin is not wired). High with a host and the cell within
    /// 80 mV of the charge voltage is a termination; lower than that the
    /// charger has stopped for another reason (its NTC window, its timer)
    /// and nothing is being added.
    #[cfg_attr(not(feature = "hw-b"), allow(dead_code))]
    pub fn from_chg_pin(charging: bool, usb_host: bool, millivolts: Option<u16>, full_mv: u16) -> Self {
        if charging {
            Self::Charging
        } else if !usb_host {
            Self::Battery
        } else if millivolts.is_some_and(|mv| mv >= full_mv.clamp(4100, 4400) - 80) {
            Self::Full
        } else {
            Self::Usb
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryState {
    pub millivolts: Option<u16>,
    pub charge: ChargeState,
    /// Charger REG04's configured termination voltage, not a measured voltage.
    pub full_mv: u16,
    /// History-aware estimate, shared by the display and host reports.
    pub estimated_percent: Option<u8>,
}

impl BatteryState {
    pub const UNKNOWN: Self = Self {
        millivolts: None,
        charge: ChargeState::Unknown,
        full_mv: 4200,
        estimated_percent: None,
    };

    pub fn percentage(self) -> Option<u8> {
        self.estimated_percent
    }

    /// Poll the charger more often where a brief termination event matters.
    pub fn near_full(self) -> bool {
        matches!(self.charge, ChargeState::Charging | ChargeState::Full)
            && self.millivolts.is_some_and(|mv| {
                (2500..=4600).contains(&mv) && mv >= self.full_mv.clamp(4100, 4400) - 100
            })
    }

    /// Uncorrected voltage lookup for bootstrapping/diagnostics, rounded to 5%.
    /// Do not use this directly for UI or host SOC: load and charging bias it.
    pub fn voltage_percentage(self) -> Option<u8> {
        let mv = self.millivolts?;
        // Approximate single-cell discharge curve. Adapt the upper portion to
        // the charger's configured endpoint for standard and high-voltage cells.
        let full = self.full_mv.clamp(4100, 4400);
        let points = [
            (3300u16, 0u16),
            (3500, 5),
            (3600, 10),
            (3700, 20),
            (3800, 40),
            (full - 300, 60),
            (full - 200, 75),
            (full - 100, 90),
            (full, 100),
        ];
        let mut percent = 100;
        if mv <= points[0].0 {
            percent = 0;
        } else {
            for pair in points.windows(2) {
                let [(lo_mv, lo_pct), (hi_mv, hi_pct)] = pair else {
                    unreachable!()
                };
                if mv <= *hi_mv && hi_mv > lo_mv {
                    percent = lo_pct
                        + ((mv - lo_mv) as u32 * (hi_pct - lo_pct) as u32 / (hi_mv - lo_mv) as u32)
                            as u16;
                    break;
                }
            }
        }
        let rounded = ((percent + 2) / 5 * 5).min(100) as u8;
        Some(rounded)
    }
}

/// Stabilizes a voltage estimate; it is not a current-integrating fuel gauge.
/// USB load relief and charging polarization must not look like instant charge.
pub struct Estimator {
    percent: Option<u8>,
    filtered_mv: Option<u16>,
    charge: ChargeState,
    last_at: Option<u32>,
    settle_ms: u32,
    step_ms: u32,
    charger_status: Option<(u8, u32)>,
    completion: Option<Completion>,
    low_since: Option<u32>,
    /// The board's charger reports nothing (Rev A's BQ24073): the voltage
    /// curve is the only knowledge, so an unknown charge state follows it in
    /// both directions instead of holding.
    voltage_only: bool,
}

struct Completion {
    since: u32,
    last_voltage_at: u32,
    samples: u8,
    full_mv: u16,
}

impl Estimator {
    const SETTLE_MS: u32 = 120_000;
    const STEP_MS: u32 = 60_000;
    const CONFIRM_MS: u32 = 30_000;
    // Normal samples are 15s apart. Never spend unobserved time as SOC credit.
    const MAX_GAP_MS: u32 = 60_000;
    // Near full, runtimes read REG08 every second. Allow scheduling jitter,
    // but never treat a cached charge indication as continuing observation.
    const STATUS_MAX_GAP_MS: u32 = 2_500;

    pub const fn new() -> Self {
        Self {
            percent: None,
            filtered_mv: None,
            charge: ChargeState::Unknown,
            last_at: None,
            settle_ms: 0,
            step_ms: 0,
            charger_status: None,
            completion: None,
            low_since: None,
            voltage_only: false,
        }
    }

    /// For a board whose charger has no status output (Rev A; Rev B reads
    /// its CHG pin). The estimate tracks the voltage curve up and down at
    /// the usual slew; 100 means the top of the curve, never a confirmed
    /// termination, and while USB is present the charging voltage biases it
    /// a little high.
    #[cfg_attr(any(not(feature = "hw-a"), feature = "hw-b"), allow(dead_code))]
    pub const fn voltage_only() -> Self {
        let mut estimator = Self::new();
        estimator.voltage_only = true;
        estimator
    }

    fn eligible_status(status: u8) -> bool {
        // PG, Fast/Done, without watchdog, thermal regulation or PPM limiting.
        status & 0x02 != 0 && (status >> 3) & 3 >= 2 && status & 0x85 == 0
    }

    /// Observe a fresh REG08 read without reusing a cached ADC sample.
    /// A true result requests a fresh voltage sample for a new Done event.
    pub fn observe_charger(&mut self, status: Option<u8>, now: u32) -> bool {
        let previous = self
            .charger_status
            .filter(|(_, at)| now.wrapping_sub(*at) <= Self::STATUS_MAX_GAP_MS);
        let eligible = status.is_some_and(Self::eligible_status);
        if previous.is_none()
            || !eligible
            || self.completion.as_ref().is_some_and(|candidate| {
                now.wrapping_sub(candidate.last_voltage_at) > Self::MAX_GAP_MS
            })
        {
            self.completion = None;
        }
        let done = eligible && status.is_some_and(|raw| (raw >> 3) & 3 == 3);
        let was_done =
            previous.is_some_and(|(raw, _)| Self::eligible_status(raw) && (raw >> 3) & 3 == 3);
        self.charger_status = status.map(|raw| (raw, now));
        done && !was_done
    }

    /// Start time of the completion event awaiting fresh voltage confirmation.
    pub fn completion_since(&self) -> Option<u32> {
        self.completion.as_ref().map(|candidate| candidate.since)
    }

    /// `sample.millivolts` must be a fresh ADC conversion, never a UI snapshot.
    pub fn update(&mut self, mut sample: BatteryState, now: u32) -> BatteryState {
        // Ignore any estimate carried in by a caller reusing its UI snapshot.
        sample.estimated_percent = None;
        let Some(mv) = sample.millivolts.filter(|mv| (2500..=4600).contains(mv)) else {
            sample.millivolts = None;
            self.last_at = None;
            self.filtered_mv = None;
            self.completion = None;
            self.low_since = None;
            self.step_ms = 0;
            return sample;
        };
        let elapsed = self.last_at.map(|at| now.wrapping_sub(at));
        let continuous = elapsed.is_some_and(|dt| dt <= Self::MAX_GAP_MS);
        let changed = self.charge != sample.charge || !continuous;
        let dt = if changed { 0 } else { elapsed.unwrap_or(0) };
        self.last_at = Some(now);
        self.charge = sample.charge;
        if changed {
            self.settle_ms = Self::SETTLE_MS;
            self.step_ms = 0;
            self.low_since = None;
            self.filtered_mv = Some(mv);
        } else {
            self.filtered_mv = Some(
                self.filtered_mv
                    .map_or(mv, |old| ((old as u32 * 3 + mv as u32 + 2) / 4) as u16),
            );
        }

        // Use the voltage curve only as a target. Full is independently
        // qualified below, never inferred from a high charging voltage alone.
        let target = BatteryState {
            millivolts: self.filtered_mv,
            charge: ChargeState::Battery,
            ..sample
        }
        .voltage_percentage()
        .unwrap();
        if self.percent.is_none() {
            self.percent = Some(
                if matches!(sample.charge, ChargeState::Charging | ChargeState::Full) {
                    target.min(95)
                } else {
                    target
                },
            );
        }

        // Done was brief in observed hardware reports. Retain its evidence
        // through Fast/Done cycling, while requiring continued clean status and
        // at least three fresh near-termination voltages over 30 seconds.
        let status = self.charger_status.filter(|(raw, at)| {
            now.wrapping_sub(*at) <= Self::STATUS_MAX_GAP_MS && Self::eligible_status(*raw)
        });
        let qualifying_voltage = status.is_some()
            && matches!(sample.charge, ChargeState::Charging | ChargeState::Full)
            && mv >= sample.full_mv.clamp(4100, 4400) - 80;
        if !qualifying_voltage
            || self.percent == Some(100)
            || self.completion.as_ref().is_some_and(|candidate| {
                candidate.full_mv != sample.full_mv
                    || now.wrapping_sub(candidate.last_voltage_at) > Self::MAX_GAP_MS
            })
        {
            self.completion = None;
        }
        if qualifying_voltage && self.percent != Some(100) {
            if let Some(candidate) = self.completion.as_mut() {
                if candidate.last_voltage_at != now {
                    candidate.samples = candidate.samples.saturating_add(1);
                    candidate.last_voltage_at = now;
                }
            } else if sample.charge == ChargeState::Full
                && status.is_some_and(|(raw, _)| (raw >> 3) & 3 == 3)
            {
                self.completion = Some(Completion {
                    since: now,
                    last_voltage_at: now,
                    samples: 1,
                    full_mv: sample.full_mv,
                });
            }
        }
        let full = self.completion.as_ref().is_some_and(|candidate| {
            candidate.samples >= 3 && now.wrapping_sub(candidate.since) >= Self::CONFIRM_MS
        });
        let low = if mv <= 3400 {
            now.wrapping_sub(*self.low_since.get_or_insert(now)) >= Self::CONFIRM_MS
        } else {
            self.low_since = None;
            false
        };

        let active_ms = dt.saturating_sub(self.settle_ms);
        self.settle_ms = self.settle_ms.saturating_sub(dt);
        let percent = self.percent.as_mut().unwrap();
        if low {
            // Do not hide sustained low voltage behind slow presentation.
            // This is an indicator, not the board's low-voltage cutoff.
            *percent = (*percent).min(target.min(5));
            self.step_ms = 0;
        } else if full {
            *percent = 100;
            self.step_ms = 0;
            self.completion = None;
        } else {
            let next = match sample.charge {
                ChargeState::Charging => target.min(95).max(*percent),
                // USB present without charging cannot add stored charge.
                ChargeState::Battery | ChargeState::Usb => target.min(*percent),
                // No charger status by design: the curve is all there is.
                ChargeState::Unknown if self.voltage_only => target,
                // A status that went missing on a board that has one holds.
                ChargeState::Unknown | ChargeState::Full => *percent,
            };
            if next == *percent {
                self.step_ms = 0;
            } else {
                self.step_ms += active_ms;
                let steps = (self.step_ms / Self::STEP_MS) as u8;
                self.step_ms %= Self::STEP_MS;
                if next > *percent {
                    *percent = percent.saturating_add(steps).min(next);
                } else {
                    *percent = percent.saturating_sub(steps).max(next);
                }
            }
        }
        // Retain 5% presentation granularity without discarding 1% history.
        sample.estimated_percent = Some(((*percent as u16 + 2) / 5 * 5).min(100) as u8);
        sample
    }
}

/// ADC channel 7 sees VBAT through the SF32LB52's internal divider. SiFli's
/// drv_adc.c uses 2.01 as its nominal VBAT factor; Sample::to_mv doesn't apply it.
pub fn battery_mv(adc_mv: u16) -> Option<u16> {
    let mv = (adc_mv as u32 * 201 + 50) / 100;
    (2500..=4600).contains(&mv).then_some(mv as u16)
}

/// Factory calibration of both the ADC offset and the internal VBAT divider.
/// The two external-input points determine the offset; the known VBAT point
/// determines the combined slope. Integer ratios avoid intermediate rounding.
#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    span_mv: i64,
    offset_num: i64,
    reference_num: i64,
    reference_mv: i64,
}

impl Calibration {
    pub fn new(
        low_raw: u16,
        low_mv: u16,
        high_raw: u16,
        high_mv: u16,
        vbat_raw: u16,
        vbat_mv: u16,
    ) -> Option<Self> {
        if low_raw == 0
            || high_raw <= low_raw
            || low_mv == 0
            || high_mv <= low_mv
            || vbat_raw == 0
            || vbat_raw > 4095
            || high_raw > 4095
            || !(2500..=4600).contains(&vbat_mv)
        {
            return None;
        }
        let span_mv = (high_mv - low_mv) as i64;
        let offset_num = low_raw as i64 * span_mv - low_mv as i64 * (high_raw - low_raw) as i64;
        let reference_num = vbat_raw as i64 * span_mv - offset_num;
        if reference_num <= 0 {
            return None;
        }
        Some(Self {
            span_mv,
            offset_num,
            reference_num,
            reference_mv: vbat_mv as i64,
        })
    }

    pub fn convert(self, raw: u16) -> Option<u16> {
        if raw > 4095 {
            return None;
        }
        let num = (raw as i64 * self.span_mv - self.offset_num) * self.reference_mv;
        let mv = (num + self.reference_num / 2) / self.reference_num;
        (2500..=4600).contains(&mv).then_some(mv as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: u8 = 0x12;
    const DONE: u8 = 0x1a;

    fn reading(mv: u16, charge: ChargeState) -> BatteryState {
        BatteryState {
            millivolts: Some(mv),
            charge,
            ..BatteryState::UNKNOWN
        }
    }

    fn begin_completion(estimator: &mut Estimator, now: u32) {
        assert!(estimator.observe_charger(Some(DONE), now));
        assert_eq!(
            estimator
                .update(reading(4190, ChargeState::Full), now)
                .percentage(),
            Some(95)
        );
        assert_eq!(estimator.completion_since(), Some(now));
    }

    fn poll_fast(estimator: &mut Estimator, start: u32, end: u32) {
        for now in (start..=end).step_by(1000) {
            assert!(!estimator.observe_charger(Some(FAST), now));
        }
    }

    #[test]
    fn usb_insertion_does_not_turn_30_percent_into_55() {
        let mut estimator = Estimator::new();
        assert_eq!(
            estimator
                .update(reading(3750, ChargeState::Battery), 0)
                .percentage(),
            Some(30)
        );
        let charging = reading(3875, ChargeState::Charging);
        assert_eq!(charging.voltage_percentage(), Some(55));
        for now in (15_000..=135_000).step_by(15_000) {
            let state = estimator.update(charging, now);
            assert_eq!(state.percentage(), Some(30));
            assert_eq!(state.charge, ChargeState::Charging);
            assert_eq!(state.millivolts, Some(3875)); // Keep real voltage observable.
        }
        for now in (150_000..=315_000).step_by(15_000) {
            estimator.update(charging, now);
        }
        assert_eq!(estimator.percent, Some(33)); // 1%/minute after settling.
        assert_eq!(estimator.update(charging, 330_000).percentage(), Some(35));
    }

    #[test]
    fn usb_without_charge_and_load_relief_cannot_add_capacity() {
        for charge in [ChargeState::Usb, ChargeState::Battery, ChargeState::Unknown] {
            let mut estimator = Estimator::new();
            estimator.update(reading(3750, ChargeState::Battery), 0);
            for now in (15_000..=3_600_000).step_by(15_000) {
                assert_eq!(
                    estimator.update(reading(4050, charge), now).percentage(),
                    Some(30)
                );
            }
        }
    }

    #[test]
    fn a_chg_pin_state_round_trips_through_the_register_encoding() {
        for state in [
            ChargeState::Battery,
            ChargeState::Usb,
            ChargeState::Charging,
            ChargeState::Full,
        ] {
            assert_eq!(ChargeState::from_register(state.as_register()), state);
        }
        assert_eq!(ChargeState::from_register(ChargeState::Unknown.as_register()), ChargeState::Battery);
        // Only a charge in progress or a termination qualifies a Done event.
        assert!(Estimator::eligible_status(ChargeState::Charging.as_register()));
        assert!(Estimator::eligible_status(ChargeState::Full.as_register()));
        assert!(!Estimator::eligible_status(ChargeState::Usb.as_register()));
        assert!(!Estimator::eligible_status(ChargeState::Battery.as_register()));
    }

    #[test]
    fn the_chg_pin_reads_charging_full_usb_or_battery() {
        // Low: charging, whatever else is known.
        assert_eq!(ChargeState::from_chg_pin(true, false, None, 4200), ChargeState::Charging);
        assert_eq!(ChargeState::from_chg_pin(true, true, Some(3700), 4200), ChargeState::Charging);
        // High without a host: no input (or a charger-only supply, which
        // this cannot see).
        assert_eq!(ChargeState::from_chg_pin(false, false, Some(4190), 4200), ChargeState::Battery);
        // High with a host: a termination near the charge voltage, else a
        // charger that stopped for another reason.
        assert_eq!(ChargeState::from_chg_pin(false, true, Some(4120), 4200), ChargeState::Full);
        assert_eq!(ChargeState::from_chg_pin(false, true, Some(4119), 4200), ChargeState::Usb);
        assert_eq!(ChargeState::from_chg_pin(false, true, None, 4200), ChargeState::Usb);
    }

    #[test]
    fn a_rev_b_charge_ends_in_a_confirmed_full_only_after_the_pin_stays_high() {
        // Rev B feeds the estimator the CHG pin through the register encoding:
        // charging holds the estimate under 100, and the pin going high with
        // the cell at the charge voltage confirms 100 after three samples
        // over 30 s, as the dev board's Done does.
        let charging = ChargeState::Charging.as_register();
        let full = ChargeState::Full.as_register();
        let mut estimator = Estimator::new();
        let mut now = 0;
        for _ in 0..40 {
            estimator.observe_charger(Some(charging), now);
            let state = estimator.update(reading(4190, ChargeState::Charging), now);
            assert!(state.percentage().unwrap() <= 95, "charging never shows 100");
            now += 15_000;
        }
        assert!(estimator.observe_charger(Some(full), now), "the pin going high is a Done edge");
        // Near full the runtimes read the status every second (the evidence
        // lapses after 2.5 s) and sample the voltage every 15 s.
        let start = now;
        let mut confirmed_at = None;
        for second in 0..=90u32 {
            now = start + second * 1000;
            if second > 0 {
                estimator.observe_charger(Some(full), now);
            }
            if second % 15 == 0 {
                let state = estimator.update(reading(4190, ChargeState::Full), now);
                if state.percentage() == Some(100) {
                    confirmed_at.get_or_insert(now);
                }
            }
        }
        assert_eq!(confirmed_at, Some(start + 30_000), "100 after three samples over 30 s");
        // The pin dropping again (a top-up) is charging again.
        estimator.observe_charger(Some(charging), now);
        let state = estimator.update(reading(4150, ChargeState::Charging), now);
        assert_eq!(state.charge, ChargeState::Charging);
    }

    #[test]
    fn voltage_only_estimator_follows_the_curve_both_ways_at_the_slew_rate() {
        // Rev A never learns the charge state, so the curve is the only knowledge.
        let mut estimator = Estimator::voltage_only();
        assert_eq!(
            estimator
                .update(reading(3750, ChargeState::Unknown), 0)
                .percentage(),
            Some(30)
        );
        // Settles, then climbs 1% per minute towards the curve with no
        // charging status needed; 4150 mV is 95% on the curve.
        let high = reading(4150, ChargeState::Unknown);
        let mut last = 30;
        let mut rose = false;
        for now in (15_000..=7_200_000).step_by(15_000) {
            let percent = estimator.update(high, now).percentage().unwrap();
            assert!(percent >= last, "fell while the voltage stayed high");
            rose |= percent > last;
            last = percent;
        }
        assert!(rose);
        assert_eq!(last, 95);
        // And follows a sustained lower voltage back down; 3700 mV is 20%.
        let low = reading(3700, ChargeState::Unknown);
        for now in (7_215_000..=14_400_000).step_by(15_000) {
            let percent = estimator.update(low, now).percentage().unwrap();
            assert!(percent <= last, "rose while the voltage stayed low");
            last = percent;
        }
        assert_eq!(last, 20);
        // The ordinary estimator still holds on an unknown status: on the dev
        // board that is a charger read that failed, not a missing charger.
        let mut holding = Estimator::new();
        holding.update(reading(3750, ChargeState::Battery), 0);
        for now in (15_000..=3_600_000).step_by(15_000) {
            assert_eq!(
                holding
                    .update(reading(4150, ChargeState::Unknown), now)
                    .percentage(),
                Some(30)
            );
        }
    }

    #[test]
    fn unplug_preserves_estimate_then_follows_sustained_discharge() {
        let mut estimator = Estimator::new();
        estimator.update(reading(3875, ChargeState::Charging), 0);
        for now in (15_000..=135_000).step_by(15_000) {
            assert_eq!(
                estimator
                    .update(reading(3750, ChargeState::Battery), now)
                    .percentage(),
                Some(55)
            );
        }
        for now in (150_000..=315_000).step_by(15_000) {
            estimator.update(reading(3750, ChargeState::Battery), now);
        }
        assert_eq!(estimator.percent, Some(52));
    }

    #[test]
    fn full_requires_consistent_status_voltage_and_time() {
        let mut estimator = Estimator::new();
        for now in (0..=690_000).step_by(1000) {
            let raw = if (615_000..=660_000).contains(&now) {
                DONE
            } else {
                FAST
            };
            estimator.observe_charger(Some(raw), now);
            if now % 15_000 == 0 {
                let mv = match now {
                    0..=600_000 => 4250,
                    615_000..=645_000 => 3800,
                    _ => 4190,
                };
                let state = estimator.update(reading(mv, ChargeState::from_register(raw)), now);
                assert_eq!(
                    state.percentage(),
                    Some(if now == 690_000 { 100 } else { 95 })
                );
            }
        }
        assert_eq!(estimator.completion_since(), None);
        estimator.observe_charger(Some(0), 705_000);
        assert_eq!(
            estimator
                .update(reading(4100, ChargeState::Battery), 705_000)
                .percentage(),
            Some(100)
        );
    }

    #[test]
    fn high_voltage_without_a_done_event_stays_at_95() {
        let mut estimator = Estimator::new();
        for now in (0..=600_000).step_by(1000) {
            assert!(!estimator.observe_charger(Some(FAST), now));
            if now % 15_000 == 0 {
                assert_eq!(
                    estimator
                        .update(reading(4250, ChargeState::Charging), now)
                        .percentage(),
                    Some(95)
                );
            }
        }
        assert_eq!(estimator.completion_since(), None);
    }

    #[test]
    fn a_brief_done_event_with_fresh_voltage_survives_fast_charge() {
        let mut estimator = Estimator::new();
        estimator.observe_charger(Some(FAST), 0);
        estimator.update(reading(4190, ChargeState::Charging), 0);
        begin_completion(&mut estimator, 1000);
        poll_fast(&mut estimator, 2000, 16_000);
        assert_eq!(
            estimator
                .update(reading(4190, ChargeState::Charging), 16_000)
                .percentage(),
            Some(95)
        );
        poll_fast(&mut estimator, 17_000, 31_000);
        let state = estimator.update(reading(4190, ChargeState::Charging), 31_000);
        assert_eq!(state.percentage(), Some(100));
        assert_eq!(state.charge, ChargeState::Charging); // Keep the true hardware status.
        assert_eq!(estimator.completion_since(), None);
        assert!(estimator.observe_charger(Some(DONE), 32_000));
        estimator.update(reading(4190, ChargeState::Full), 32_000);
        assert_eq!(estimator.completion_since(), None); // Already anchored.
    }

    #[test]
    fn repeated_done_events_do_not_restart_confirmation() {
        let mut estimator = Estimator::new();
        begin_completion(&mut estimator, 0);
        for now in (1000..=30_000).step_by(1000) {
            let raw = if now % 2000 == 0 { DONE } else { FAST };
            assert_eq!(estimator.observe_charger(Some(raw), now), raw == DONE);
            if now == 15_000 || now == 30_000 {
                let state = estimator.update(reading(4190, ChargeState::from_register(raw)), now);
                assert_eq!(
                    state.percentage(),
                    Some(if now == 30_000 { 100 } else { 95 })
                );
            } else {
                assert_eq!(estimator.completion_since(), Some(0));
            }
        }
    }

    #[test]
    fn status_observations_do_not_reuse_cached_voltage_or_advance_soc() {
        let mut estimator = Estimator::new();
        begin_completion(&mut estimator, 0);
        let filtered = estimator.filtered_mv;
        let last_at = estimator.last_at;
        let settle_ms = estimator.settle_ms;
        poll_fast(&mut estimator, 1000, 30_000);
        assert_eq!(estimator.percent, Some(95));
        assert_eq!(estimator.filtered_mv, filtered);
        assert_eq!(estimator.last_at, last_at);
        assert_eq!(estimator.settle_ms, settle_ms);
        // Thirty seconds and only two fresh conversions cannot confirm full.
        assert_eq!(
            estimator
                .update(reading(4190, ChargeState::Charging), 30_000)
                .percentage(),
            Some(95)
        );
        assert_eq!(estimator.completion.as_ref().unwrap().samples, 2);
        // Even duplicate updates at one timestamp are not distinct readings.
        estimator.update(reading(4190, ChargeState::Charging), 30_000);
        assert_eq!(estimator.completion.as_ref().unwrap().samples, 2);
        assert_eq!(estimator.percent, Some(95));
        poll_fast(&mut estimator, 31_000, 45_000);
        assert_eq!(
            estimator
                .update(reading(4190, ChargeState::Charging), 45_000)
                .percentage(),
            Some(100)
        );
    }

    #[test]
    fn done_without_a_fresh_corresponding_voltage_cannot_start_completion() {
        let mut estimator = Estimator::new();
        estimator.observe_charger(Some(FAST), 0);
        let cached = estimator.update(reading(4190, ChargeState::Charging), 0);
        assert!(estimator.observe_charger(Some(DONE), 1000));
        // The runtime keeps its UI snapshot, but never passes it to update().
        assert_eq!(cached.percentage(), Some(95));
        assert_eq!(estimator.completion_since(), None);
        poll_fast(&mut estimator, 2000, 31_000);
        estimator.update(reading(4190, ChargeState::Charging), 31_000);
        assert_eq!(estimator.percent, Some(95));
        assert_eq!(estimator.completion_since(), None);
        // A caller's Full label cannot replace a fresh raw Done register.
        estimator.update(reading(4190, ChargeState::Full), 31_000);
        assert_eq!(estimator.completion_since(), None);
        estimator.observe_charger(Some(DONE), 32_000);
        estimator.update(reading(4190, ChargeState::Full), 35_000);
        assert_eq!(estimator.completion_since(), None);
    }

    #[test]
    fn low_invalid_or_noncharging_voltage_samples_cancel_completion() {
        for sample in [
            reading(4119, ChargeState::Charging),
            reading(2499, ChargeState::Charging),
            reading(4601, ChargeState::Charging),
            BatteryState {
                charge: ChargeState::Charging,
                ..BatteryState::UNKNOWN
            },
            reading(4190, ChargeState::Battery),
            reading(4190, ChargeState::Usb),
            reading(4190, ChargeState::Unknown),
            BatteryState {
                full_mv: 4350,
                ..reading(4350, ChargeState::Charging)
            },
        ] {
            let mut estimator = Estimator::new();
            begin_completion(&mut estimator, 0);
            poll_fast(&mut estimator, 1000, 15_000);
            estimator.update(sample, 15_000);
            assert_eq!(estimator.completion_since(), None, "{sample:?}");
            for now in (16_000..=60_000).step_by(1000) {
                estimator.observe_charger(Some(FAST), now);
                if now % 15_000 == 0 {
                    assert_eq!(
                        estimator
                            .update(reading(4190, ChargeState::Charging), now)
                            .percentage(),
                        Some(95)
                    );
                }
            }
        }
    }

    #[test]
    fn precharge_errors_limits_and_unplug_cancel_completion() {
        for status in [
            None,
            Some(0x0a),
            Some(0x02),
            Some(0),
            Some(0x18),
            Some(FAST | 0x80),
            Some(DONE | 0x01),
            Some(DONE | 0x04),
        ] {
            let mut estimator = Estimator::new();
            begin_completion(&mut estimator, 0);
            assert!(!estimator.observe_charger(status, 1000));
            assert_eq!(estimator.completion_since(), None, "{status:?}");
            poll_fast(&mut estimator, 2000, 30_000);
            assert_eq!(
                estimator
                    .update(reading(4190, ChargeState::Charging), 30_000)
                    .percentage(),
                Some(95)
            );
        }
    }

    #[test]
    fn missing_status_observations_cancel_and_stale_done_can_trigger_a_new_sample() {
        let mut estimator = Estimator::new();
        begin_completion(&mut estimator, 0);
        assert!(!estimator.observe_charger(Some(DONE), 2500));
        assert_eq!(estimator.completion_since(), Some(0));
        assert!(estimator.observe_charger(Some(DONE), 5001));
        assert_eq!(estimator.completion_since(), None);
        estimator.update(reading(4190, ChargeState::Full), 5001);
        assert_eq!(estimator.completion_since(), Some(5001));
        // update also rejects an old status if no observation was attempted.
        estimator.update(reading(4190, ChargeState::Full), 7502);
        assert_eq!(estimator.completion_since(), None);
    }

    #[test]
    fn status_only_cannot_keep_completion_alive_across_a_voltage_gap() {
        let mut estimator = Estimator::new();
        begin_completion(&mut estimator, 0);
        poll_fast(&mut estimator, 1000, 60_000);
        assert_eq!(estimator.completion_since(), Some(0));
        estimator.observe_charger(Some(FAST), 60_001);
        assert_eq!(estimator.completion_since(), None);
        estimator.update(reading(4190, ChargeState::Charging), 60_001);
        assert_eq!(estimator.percent, Some(95));
    }

    #[test]
    fn completion_timing_survives_timer_wrap() {
        let mut estimator = Estimator::new();
        let start = u32::MAX - 20_000;
        begin_completion(&mut estimator, start);
        for dt in (1000..=30_000u32).step_by(1000) {
            let now = start.wrapping_add(dt);
            estimator.observe_charger(Some(FAST), now);
            if dt % 15_000 == 0 {
                assert_eq!(
                    estimator
                        .update(reading(4190, ChargeState::Charging), now)
                        .percentage(),
                    Some(if dt == 30_000 { 100 } else { 95 })
                );
            }
        }
    }

    #[test]
    fn recorded_done_then_3729_mv_vetoes_completion() {
        let mut estimator = Estimator::new();
        begin_completion(&mut estimator, 0);
        for now in (1000..=90_000).step_by(1000) {
            estimator.observe_charger(Some(FAST), now);
            if now % 15_000 == 0 {
                let mv = if now == 15_000 { 3729 } else { 4190 };
                assert_eq!(
                    estimator
                        .update(reading(mv, ChargeState::Charging), now)
                        .percentage(),
                    Some(95)
                );
                assert_eq!(estimator.completion_since(), None);
            }
        }
    }

    #[test]
    fn near_full_polling_requires_valid_charging_voltage() {
        assert!(reading(4100, ChargeState::Charging).near_full());
        assert!(reading(4190, ChargeState::Full).near_full());
        assert!(!reading(4099, ChargeState::Charging).near_full());
        assert!(!reading(4900, ChargeState::Full).near_full());
        assert!(!reading(4190, ChargeState::Battery).near_full());
        assert!(!reading(4190, ChargeState::Usb).near_full());
        assert!(!BatteryState::UNKNOWN.near_full());
        assert!(!BatteryState {
            full_mv: 4350,
            ..reading(4200, ChargeState::Full)
        }
        .near_full());
        assert!(BatteryState {
            full_mv: 4350,
            ..reading(4250, ChargeState::Full)
        }
        .near_full());
    }

    #[test]
    fn sustained_low_voltage_overrides_smoothing_but_a_single_dip_does_not() {
        let mut estimator = Estimator::new();
        estimator.update(reading(3900, ChargeState::Battery), 0);
        assert_eq!(
            estimator
                .update(reading(3300, ChargeState::Battery), 15_000)
                .percentage(),
            Some(60)
        );
        assert_eq!(
            estimator
                .update(reading(3900, ChargeState::Battery), 30_000)
                .percentage(),
            Some(60)
        );
        for now in [45_000, 60_000] {
            assert_eq!(
                estimator
                    .update(reading(3350, ChargeState::Battery), now)
                    .percentage(),
                Some(60)
            );
        }
        assert!(
            estimator
                .update(reading(3350, ChargeState::Battery), 75_000)
                .percentage()
                .unwrap()
                <= 5
        );
    }

    #[test]
    fn invalid_samples_and_long_gaps_do_not_discard_history_or_bank_charge() {
        let mut estimator = Estimator::new();
        estimator.update(reading(3750, ChargeState::Battery), 0);
        assert_eq!(
            estimator.update(BatteryState::UNKNOWN, 15_000).percentage(),
            None
        );
        assert_eq!(
            estimator
                .update(reading(4900, ChargeState::Full), 30_000)
                .percentage(),
            None
        );
        assert_eq!(
            estimator
                .update(reading(4100, ChargeState::Charging), 3_600_000)
                .percentage(),
            Some(30)
        );
        assert_eq!(
            estimator
                .update(reading(4150, ChargeState::Charging), 7_200_000)
                .percentage(),
            Some(30)
        );
    }

    #[test]
    fn estimator_time_is_wrap_safe_and_does_not_bank_idle_credit() {
        let mut estimator = Estimator::new();
        let start = u32::MAX - 20_000;
        estimator.update(reading(3750, ChargeState::Charging), start);
        for dt in (15_000..=600_000u32).step_by(15_000) {
            assert_eq!(
                estimator
                    .update(reading(3750, ChargeState::Charging), start.wrapping_add(dt))
                    .percentage(),
                Some(30)
            );
        }
        let state = estimator.update(
            reading(4170, ChargeState::Charging),
            start.wrapping_add(615_000),
        );
        assert_eq!(state.percentage(), Some(30));
        assert_eq!(estimator.percent, Some(30));
        for dt in (630_000..=795_000u32).step_by(15_000) {
            estimator.update(reading(4170, ChargeState::Charging), start.wrapping_add(dt));
        }
        assert_eq!(estimator.percent, Some(33));
    }

    #[test]
    fn missing_voltage_never_reports_a_stale_percentage_even_if_full() {
        let mut estimator = Estimator::new();
        assert_eq!(
            estimator.update(BatteryState::UNKNOWN, 0).percentage(),
            None
        );
        estimator.update(reading(3750, ChargeState::Battery), 15_000);
        assert_eq!(
            estimator
                .update(
                    BatteryState {
                        charge: ChargeState::Full,
                        ..BatteryState::UNKNOWN
                    },
                    30_000
                )
                .percentage(),
            None
        );
    }

    #[test]
    fn distinguishes_usb_charging_and_full() {
        assert_eq!(ChargeState::from_register(0), ChargeState::Battery);
        assert_eq!(ChargeState::from_register(2), ChargeState::Usb);
        assert_eq!(ChargeState::from_register(0x0a), ChargeState::Charging);
        assert_eq!(ChargeState::from_register(0x12), ChargeState::Charging);
        assert_eq!(ChargeState::from_register(0x1a), ChargeState::Full);
        assert_eq!(ChargeState::from_register(0x18), ChargeState::Battery);
    }

    #[test]
    fn divider_scaling_and_invalid_samples() {
        assert_eq!(battery_mv(2000), Some(4020));
        assert_eq!(battery_mv(2100), Some(4221));
        assert_eq!(battery_mv(0), None);
        assert_eq!(battery_mv(3300), None);
    }

    #[test]
    fn estimates_are_bounded_and_monotonic() {
        for full_mv in [4100, 4200, 4350, 4400] {
            let mut previous = 0;
            for mv in 2500..=4600 {
                let state = BatteryState {
                    millivolts: Some(mv),
                    full_mv,
                    charge: ChargeState::Battery,
                    ..BatteryState::UNKNOWN
                };
                let percent = state.voltage_percentage().unwrap();
                assert!((previous..=100).contains(&percent));
                assert_eq!(percent % 5, 0);
                previous = percent;
            }
        }
    }

    #[test]
    fn unknown_is_not_empty_or_full() {
        assert_eq!(BatteryState::UNKNOWN.voltage_percentage(), None);
        assert_eq!(
            BatteryState {
                charge: ChargeState::Full,
                ..BatteryState::UNKNOWN
            }
            .voltage_percentage(),
            None
        );
    }

    #[test]
    fn factory_offset_and_divider_are_both_applied() {
        let cal = Calibration::new(1500, 1000, 3000, 2500, 2600, 4200).unwrap();
        assert_eq!(cal.convert(2600), Some(4200));
        assert_eq!(cal.convert(2500), Some(4000));
        assert_eq!(cal.convert(2250), Some(3500));
        assert_eq!(cal.convert(0), None);
        assert_eq!(cal.convert(4095), None);
        let board = Calibration::new(1771, 1000, 3207, 2500, 2826, 4200).unwrap();
        assert_eq!(board.convert(2826), Some(4200));
    }

    #[test]
    fn rejects_blank_or_invalid_factory_data() {
        assert!(Calibration::new(0, 0, 0, 0, 0, 0).is_none());
        assert!(Calibration::new(1000, 1000, 1000, 2500, 2600, 4200).is_none());
        assert!(Calibration::new(1500, 1000, 3000, 2500, 2600, 0).is_none());
    }
}
