//! The bench probe's signal maths, over 16 kHz `i16` PCM: a sine to play,
//! the level of what came back, and how much of it is the tone.
//!
//! The loopback check asks one question — did the tone the speaker played
//! come back through the microphone? — and answers it with a Goertzel
//! filter at the tone's frequency against one at an off-frequency
//! neighbour. A ratio, not an absolute level, because nobody knows the
//! deck's acoustic gain in advance (the design note says about +17 dB from
//! speaker to microphone, the amplifier's input divider is being reworked,
//! and a bench is not an anechoic room), while a tone 15 dB above its
//! neighbour is unmistakable and silence or broadband noise is not.
use super::codec::RATE_HZ;

/// 0 dBFS: a sample of ±32768.
pub const FULL_SCALE: f64 = 32_768.0;
/// The quietest level reported. Silence reads this rather than −∞, so a
/// ratio of two levels is always a number.
pub const FLOOR_DBFS: f64 = -150.0;

/// An amplitude (or an RMS) in dBFS, floored at [`FLOOR_DBFS`].
#[must_use]
pub fn dbfs(amplitude: f64) -> f64 {
    if amplitude > 0.0 {
        (20.0 * (amplitude / FULL_SCALE).log10()).max(FLOOR_DBFS)
    } else {
        FLOOR_DBFS
    }
}

/// A sine at a fixed frequency, continuous across [`Sine::fill`] calls.
#[derive(Clone, Debug)]
pub struct Sine {
    phase: f64,
    step: f64,
    amplitude: f64,
}

impl Sine {
    /// `peak_dbfs` is the sine's peak level: 0 dBFS is a full-scale sine.
    #[must_use]
    pub fn new(hz: f64, peak_dbfs: f64) -> Sine {
        Sine {
            phase: 0.0,
            step: std::f64::consts::TAU * hz / f64::from(RATE_HZ),
            amplitude: FULL_SCALE * 10f64.powf(peak_dbfs / 20.0),
        }
    }

    /// The next `out.len()` samples.
    pub fn fill(&mut self, out: &mut [i16]) {
        for sample in out {
            let value = (self.amplitude * self.phase.sin()).round();
            *sample = value.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
            self.phase = (self.phase + self.step) % std::f64::consts::TAU;
        }
    }
}

/// A linear fade-in and fade-out over `ramp` samples at each end of a
/// `total`-sample signal: the gain for sample `index`. A tone that starts at
/// full level clicks, and the click is broadband — the opposite of a tone.
#[must_use]
pub fn fade(index: usize, total: usize, ramp: usize) -> f64 {
    if ramp == 0 || index >= total {
        return if index < total { 1.0 } else { 0.0 };
    }
    let from_start = index as f64 / ramp as f64;
    let to_end = (total - 1 - index) as f64 / ramp as f64;
    from_start.min(to_end).min(1.0)
}

/// RMS and peak over everything pushed since the last reset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    sum_sq: f64,
    count: u64,
    peak: u16,
}

impl Level {
    pub fn push(&mut self, samples: &[i16]) {
        for &s in samples {
            let x = f64::from(s);
            self.sum_sq += x * x;
            self.peak = self.peak.max(s.unsigned_abs());
        }
        self.count += samples.len() as u64;
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// RMS in dBFS (a full-scale square wave is 0 dBFS, a full-scale sine
    /// −3 dBFS); [`FLOOR_DBFS`] with nothing pushed.
    #[must_use]
    pub fn rms_dbfs(&self) -> f64 {
        if self.count == 0 {
            return FLOOR_DBFS;
        }
        dbfs((self.sum_sq / self.count as f64).sqrt())
    }

    /// The largest magnitude seen (32768 for −32768).
    #[must_use]
    pub fn peak(&self) -> u16 {
        self.peak
    }

    #[must_use]
    pub fn peak_dbfs(&self) -> f64 {
        dbfs(f64::from(self.peak))
    }
}

/// The peak amplitude, in dBFS, of the component of `samples` at `hz`: a
/// Goertzel filter over a Hann-windowed block, evaluated at exactly `hz`
/// (not the nearest bin). A sine of peak level L at `hz` reads L.
///
/// The window costs a little resolution and buys a great deal of rejection:
/// a tone that is not centred on a bin leaks into every other bin through a
/// rectangular window's −13 dB sidelobes, which would make a strong tone
/// look like noise at the neighbouring frequency.
#[must_use]
pub fn tone_dbfs(samples: &[i16], hz: f64) -> f64 {
    let n = samples.len();
    if n < 2 {
        return FLOOR_DBFS;
    }
    let w = std::f64::consts::TAU * hz / f64::from(RATE_HZ);
    let coeff = 2.0 * w.cos();
    let span = (n - 1) as f64;
    let (mut s1, mut s2, mut gain) = (0.0f64, 0.0f64, 0.0f64);
    for (i, &x) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / span).cos();
        gain += window;
        let s = f64::from(x) * window + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    // |X(w)|², whatever w is: the Goertzel output differs from the DFT term
    // only by a phase.
    let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
    // A sine of peak A contributes A/2 at +w (and A/2 at −w), times the
    // window's sum.
    dbfs(2.0 * power.sqrt() / gain)
}

/// What the loopback check compares the tone against: 1.25 times its
/// frequency, which is between harmonics (distortion lands on 2× and 3×),
/// hundreds of window bins away in a one-second block, and still in the
/// band a small speaker and a PDM microphone both pass. Near Nyquist, 0.8
/// times instead.
#[must_use]
pub fn off_frequency(hz: f64) -> f64 {
    let above = hz * 1.25;
    if above < 0.475 * f64::from(RATE_HZ) {
        above
    } else {
        hz * 0.8
    }
}

/// `tone - off` in dB: how far the tone stands above its neighbour.
#[must_use]
pub fn tone_ratio_db(samples: &[i16], hz: f64) -> f64 {
    tone_dbfs(samples, hz) - tone_dbfs(samples, off_frequency(hz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::codec;

    fn sine(hz: f64, peak_dbfs: f64, n: usize) -> Vec<i16> {
        let mut out = vec![0i16; n];
        Sine::new(hz, peak_dbfs).fill(&mut out);
        out
    }

    /// A deterministic white-ish noise (an LCG), full band.
    fn noise(n: usize, amplitude: f64) -> Vec<i16> {
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let u = f64::from(x >> 8) / f64::from(1u32 << 24) * 2.0 - 1.0;
                (u * amplitude) as i16
            })
            .collect()
    }

    #[test]
    fn the_sine_has_its_level_and_frequency_and_continues_across_fills() {
        let pcm = sine(1000.0, -6.0206, 16_000);
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((16_370..=16_384).contains(&peak), "peak {peak}");
        // 1000 Hz: 2000 sign changes a second.
        let crossings = pcm.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!((1998..=2001).contains(&crossings), "{crossings}");
        // Two half-frames are one frame.
        let mut a = Sine::new(440.0, -20.0);
        let mut halves = vec![0i16; 320];
        a.fill(&mut halves[..160]);
        a.fill(&mut halves[160..]);
        assert_eq!(halves, sine(440.0, -20.0, 320));
    }

    #[test]
    fn the_goertzel_reads_a_tone_at_its_level_and_nothing_beside_it() {
        for hz in [300.0, 1000.0, 1234.5, 3000.0, 6500.0] {
            let pcm = sine(hz, -6.0206, 16_000);
            let at = tone_dbfs(&pcm, hz);
            assert!((at + 6.02).abs() < 0.1, "{hz} Hz reads {at:.2} dBFS");
            let off = tone_dbfs(&pcm, off_frequency(hz));
            assert!(off < -90.0, "{hz} Hz leaks {off:.1} dBFS");
            assert!(tone_ratio_db(&pcm, hz) > 80.0);
        }
        // Short blocks (one 20 ms frame) still resolve a quarter apart.
        let frame = sine(1000.0, -20.0, 320);
        assert!(tone_ratio_db(&frame, 1000.0) > 30.0);
        assert_eq!(tone_dbfs(&[], 1000.0), FLOOR_DBFS);
        assert_eq!(tone_dbfs(&[0; 16_000], 1000.0), FLOOR_DBFS);
    }

    #[test]
    fn noise_is_not_a_tone_and_a_buried_tone_still_is() {
        // One block of noise puts each Goertzel bin anywhere within several
        // dB of the other (each is one exponentially distributed power), so
        // the bound is the probe's own threshold: noise is not a pass.
        let hiss = noise(16_000, 8000.0);
        let ratio = tone_ratio_db(&hiss, 1000.0);
        assert!(ratio < 15.0, "noise reads {ratio:.1} dB");
        // A −30 dBFS tone under −20 dBFS-ish noise: narrowband wins.
        let tone = sine(1000.0, -30.0, 16_000);
        let mixed: Vec<i16> = hiss
            .iter()
            .zip(&tone)
            .map(|(a, b)| (i32::from(*a) / 3 + i32::from(*b)) as i16)
            .collect();
        assert!(tone_ratio_db(&mixed, 1000.0) > 15.0);
    }

    #[test]
    fn a_tone_through_the_codec_still_passes_the_loopback_threshold() {
        // The whole wire: 20 ms frames, each decoded with its own state.
        let pcm = sine(1000.0, -20.0, 16_000);
        let mut encoder = codec::State::default();
        let mut decoded = Vec::new();
        for (seq, chunk) in pcm.chunks(codec::FRAME_SAMPLES).enumerate() {
            let mut frame = [0u8; codec::MAX_FRAME];
            let n = codec::audio_frame(1, seq as u8, &mut encoder, chunk, &mut frame);
            let codec::Frame::Audio {
                mut state, data, ..
            } = codec::parse(&frame[..n])
            else {
                panic!("audio");
            };
            let mut out = [0i16; codec::FRAME_SAMPLES];
            let m = codec::decode(&mut state, data, &mut out);
            decoded.extend_from_slice(&out[..m]);
        }
        let ratio = tone_ratio_db(&decoded, 1000.0);
        assert!(ratio > 15.0, "{ratio:.1} dB");
        assert!((tone_dbfs(&decoded, 1000.0) + 20.0).abs() < 0.5);
    }

    #[test]
    fn levels_are_rms_and_peak_in_dbfs() {
        let mut level = Level::default();
        assert_eq!(level.rms_dbfs(), FLOOR_DBFS);
        level.push(&[i16::MAX, i16::MIN, i16::MAX, i16::MIN]);
        assert!(level.rms_dbfs().abs() < 0.01, "{}", level.rms_dbfs());
        assert_eq!(level.peak(), 32_768);
        assert_eq!(level.peak_dbfs(), 0.0);
        assert_eq!(level.count(), 4);

        let mut level = Level::default();
        level.push(&sine(1000.0, 0.0, 16_000));
        assert!(
            (level.rms_dbfs() + 3.01).abs() < 0.05,
            "{}",
            level.rms_dbfs()
        );

        let mut level = Level::default();
        level.push(&[0; 320]);
        assert_eq!(level.rms_dbfs(), FLOOR_DBFS);
        assert_eq!(dbfs(-1.0), FLOOR_DBFS);
    }

    #[test]
    fn fades_ramp_both_ends_and_off_frequencies_stay_in_band() {
        assert_eq!(fade(0, 1000, 160), 0.0);
        assert_eq!(fade(80, 1000, 160), 0.5);
        assert_eq!(fade(500, 1000, 160), 1.0);
        assert_eq!(fade(999, 1000, 160), 0.0);
        assert_eq!(fade(1000, 1000, 160), 0.0);
        assert_eq!(fade(5, 10, 0), 1.0);
        assert_eq!(off_frequency(1000.0), 1250.0);
        assert_eq!(off_frequency(440.0), 550.0);
        assert_eq!(off_frequency(7000.0), 5600.0);
        for hz in [100.0, 1000.0, 6000.0, 7000.0] {
            let off = off_frequency(hz);
            assert!(off < 0.5 * f64::from(RATE_HZ) && (off - hz).abs() > 0.19 * hz);
        }
    }
}
