//! Rev A speaker correction for the 16 kHz USB/BLE output.
//!
//! Conservative tuning from the supplied 0.1 W / 0.1 m response plot:
//! cut the broad 1.1 kHz hump by 5 dB and the 6.3 kHz peak by 6 dB.
//! Do not invert the bass roll-off or the narrow nulls: the plot is not
//! an in-enclosure calibration.
//! Everything above 8 kHz is outside this transport's Nyquist limit.
//!
//! This runs after the mixer/resampler, before PCM quantization and ADPCM.
//! Coefficients use the RBJ high-pass and peaking equations:
//! <https://www.w3.org/TR/audio-eq-cookbook/>.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

/// Three dB of transient headroom; every EQ band also has gain <= unity.
const PREAMP: f64 = 0.707_945_784_384_137_9;
/// A final sample ceiling (-1 dBFS) handles summed lanes and filter overshoot.
/// This is a safety clamp, not an automatic loudness control.
const CEILING: f64 = 0.891_250_938_133_745_6;

/// One instance per output open. State survives block and talk-spurt
/// boundaries, including the silence that lets the filters decay.
pub(super) struct SpeakerEq {
    filters: [Biquad; 3],
}

impl SpeakerEq {
    pub(super) fn new() -> Self {
        let rate = f64::from(crate::ble::codec::RATE_HZ);
        Self {
            filters: [
                Biquad::high_pass(rate, 250.0, FRAC_1_SQRT_2),
                Biquad::peak(rate, 1100.0, 1.0, -5.0),
                Biquad::peak(rate, 6300.0, 4.0, -6.0),
            ],
        }
    }

    /// Average stereo in floating point, EQ, then round to the codec's PCM.
    /// No allocation, locking, or block-dependent gain changes.
    pub(super) fn render(&mut self, stereo: &[f32], mono: &mut [i16]) {
        let (frames, _) = stereo.as_chunks::<2>();
        for (out, &[left, right]) in mono.iter_mut().zip(frames) {
            // Sanitize before the recursive filters, where one NaN could
            // otherwise poison all subsequent audio. Convert before adding
            // so even two finite f32::MAX samples cannot overflow.
            let finite = |s: f32| if s.is_finite() { f64::from(s) } else { 0.0 };
            let mut sample = 0.5 * (finite(left) + finite(right)) * PREAMP;
            for filter in &mut self.filters {
                sample = filter.process(sample);
            }
            *out = (sample.clamp(-CEILING, CEILING) * 32768.0).round() as i16;
        }
    }
}

/// Normalized biquad, transposed direct form II. f64 keeps quiet decay
/// accurate and avoids accumulating roundoff over long output sessions.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b: b.map(|v| v / a[0]),
            a: [a[1] / a[0], a[2] / a[0]],
            z: [0.0; 2],
        }
    }

    fn high_pass(rate: f64, hz: f64, q: f64) -> Self {
        let (sin, cos) = (TAU * hz / rate).sin_cos();
        let alpha = sin / (2.0 * q);
        Self::new(
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        )
    }

    fn peak(rate: f64, hz: f64, q: f64, db: f64) -> Self {
        let (sin, cos) = (TAU * hz / rate).sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10.0_f64.powf(db / 40.0);
        Self::new(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    fn process(&mut self, input: f64) -> f64 {
        let out = self.b[0] * input + self.z[0];
        self.z[0] = self.b[1] * input - self.a[0] * out + self.z[1];
        self.z[1] = self.b[2] * input - self.a[1] * out;
        // Far below PCM resolution; keep long idle periods out of denormals.
        for z in &mut self.z {
            if z.abs() < 1e-20 {
                *z = 0.0;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = crate::ble::codec::RATE_HZ as usize;

    fn tone(hz: f64, amplitude: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let sample = (amplitude * (TAU * hz * i as f64 / RATE as f64).sin()) as f32;
                [sample, sample]
            })
            .collect()
    }

    fn gain_db(hz: f64) -> f64 {
        let input = tone(hz, 0.25, RATE);
        let mut output = vec![0; RATE];
        SpeakerEq::new().render(&input, &mut output);
        // Discard startup, measure actual PCM rather than the coefficient
        // formula. Half a second is an integer cycle count for these tones.
        let power = output[RATE / 2..]
            .iter()
            .map(|&s| (f64::from(s) / 32768.0).powi(2))
            .sum::<f64>()
            / (RATE / 2) as f64;
        10.0 * (power / (0.25 * 0.25 / 2.0)).log10()
    }

    #[test]
    fn measured_pcm_cuts_the_humps_and_unusable_bass() {
        for (hz, low, high) in [
            (60.0, -30.0, -27.0),
            (100.0, -20.0, -18.0),
            (250.0, -6.5, -6.0),
            (1100.0, -8.2, -7.9),
            (3000.0, -3.8, -3.3),
            (6300.0, -9.2, -8.9),
            (7500.0, -3.3, -3.0),
        ] {
            let db = gain_db(hz);
            assert!((low..=high).contains(&db), "{hz} Hz: {db} dB");
        }
        for hz in (100..8000).step_by(100) {
            assert!(gain_db(f64::from(hz)) <= -2.99, "{hz} Hz was boosted");
        }
    }

    #[test]
    fn arbitrary_block_boundaries_produce_identical_pcm() {
        let input = tone(1100.0, 0.8, 17003);
        let mut whole = vec![0; input.len() / 2];
        SpeakerEq::new().render(&input, &mut whole);
        for block in [1, 113, 320, 512] {
            let mut eq = SpeakerEq::new();
            let mut split = vec![0; whole.len()];
            for (source, dest) in input.chunks(block * 2).zip(split.chunks_mut(block)) {
                eq.render(source, dest);
            }
            assert_eq!(whole, split, "block size {block}");
        }
    }

    #[test]
    fn stereo_is_averaged_before_filtering_and_quantization() {
        let input = tone(3000.0, 0.25, 1600);
        let (frames, _) = input.as_chunks::<2>();
        let left_only: Vec<_> = frames.iter().flat_map(|s| [s[0] * 2.0, 0.0]).collect();
        let opposed: Vec<_> = frames.iter().flat_map(|s| [s[0], -s[0]]).collect();
        let mut expected = vec![0; 1600];
        let mut actual = vec![0; 1600];
        SpeakerEq::new().render(&input, &mut expected);
        SpeakerEq::new().render(&left_only, &mut actual);
        assert_eq!(actual, expected);
        SpeakerEq::new().render(&opposed, &mut actual);
        assert!(actual.iter().all(|&s| s == 0));
    }

    #[test]
    fn overloads_are_bounded_and_invalid_samples_do_not_poison_state() {
        let mut eq = SpeakerEq::new();
        let mut output = vec![0; RATE];
        eq.render(&tone(3000.0, 8.0, RATE), &mut output);
        let limit = (CEILING * 32768.0).round() as i16;
        assert!(output.iter().all(|s| s.abs() <= limit));
        assert!(output.contains(&limit), "exercise the safety ceiling");

        eq.render(&[f32::MAX; 640], &mut output[..320]);
        eq.render(
            &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0],
            &mut output[..2],
        );
        eq.render(&vec![0.0; RATE * 2], &mut output);
        assert!(output[RATE / 2..].iter().all(|&s| s == 0));
        eq.render(&tone(3000.0, 0.25, RATE), &mut output);
        assert!(output[RATE / 2..].iter().any(|&s| s > 1000));
    }

    #[test]
    fn silence_stays_silent_and_the_tail_decays_within_hangover() {
        let mut eq = SpeakerEq::new();
        let mut output = vec![99; RATE * 3 / 10];
        eq.render(&vec![0.0; output.len() * 2], &mut output);
        assert!(output.iter().all(|&s| s == 0));
        eq.render(&tone(1100.0, 1.0, 320), &mut output[..320]);
        eq.render(&vec![0.0; output.len() * 2], &mut output);
        assert!(output[320..].iter().all(|&s| s == 0));
        // A close/open constructs a fresh filter, without the previous tail.
        SpeakerEq::new().render(&[0.0; 2], &mut output[..1]);
        assert_eq!(output[0], 0);
    }
}
