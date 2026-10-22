//! Band-limited 16 → 48 kHz reconstruction, and native 48 kHz packing.
//! A 127-tap Kaiser FIR suppresses the interpolation images left by linear
//! interpolation. 7.3 kHz cutoff, beta 8.6; three Q15 polyphase branches,
//! each normalized to unity DC gain. Group delay: 63 / 48000 = 1.3125 ms.
//! Generate with scipy.signal.firwin(127, 7300, fs=48000,
//! window=('kaiser', 8.6)) * 3; quantize each phase to Q15, correct its sum.
use super::codec;

pub const FRAME_WORDS: usize = 3 * codec::FRAME_SAMPLES / 2;
const TAPS: usize = 43;
const FADE: usize = 240; // 5 ms at the DAC's 48 kHz rate.
const COEFFICIENTS: [[i32; TAPS]; 3] = [
    [
        0, 2, -5, 12, -23, 38, -53, 63, -60, 32, 34, -151, 330, -574, 881, -1238, 1622, -2004,
        2350, -2627, 2805, 29900, 2805, -2627, 2350, -2004, 1622, -1238, 881, -574, 330, -151, 34,
        32, -60, 63, -53, 38, -23, 12, -5, 2, 0,
    ],
    [
        0, -1, -1, 6, -17, 37, -69, 111, -160, 207, -238, 233, -167, 10, 267, -698, 1325, -2224,
        3584, -6088, 14688, 25528, -4843, 1678, -369, -290, 610, -723, 705, -611, 482, -347, 227,
        -131, 63, -21, -1, 10, -10, 8, -4, 2, 0,
    ],
    [
        2, -4, 8, -10, 10, -1, -21, 63, -131, 227, -347, 482, -611, 705, -723, 610, -290, -369,
        1678, -4843, 25528, 14688, -6088, 3584, -2224, 1325, -698, 267, 10, -167, 233, -238, 207,
        -160, 111, -69, 37, -17, 6, -1, -1, 0, 0,
    ],
];

pub struct Upsampler {
    history: [i16; TAPS],
    head: usize,
    prev: i16,
}

impl Default for Upsampler {
    fn default() -> Self {
        Self {
            history: [0; TAPS],
            head: 0,
            prev: 0,
        }
    }
}

impl Upsampler {
    pub fn run_at(&mut self, rate_khz: u8, input: &[i16], out: &mut [u32]) -> usize {
        if rate_khz == 16 {
            return self.run(input, out);
        }
        if rate_khz != 48 {
            return 0;
        }
        let n = (input.len() / 2).min(out.len());
        for (pair, word) in input.chunks_exact(2).zip(out.iter_mut()).take(n) {
            *word = pack(pair[0], pair[1]);
            self.prev = pair[1];
        }
        n
    }

    /// Even-sized input; state persists across packet boundaries.
    pub fn run(&mut self, input: &[i16], out: &mut [u32]) -> usize {
        let count = input.len().min(out.len() * 2 / 3).min(codec::FRAME_SAMPLES) & !1;
        let mut at = 0;
        let mut first = 0;
        for &sample in &input[..count] {
            self.head = (self.head + TAPS - 1) % TAPS;
            self.history[self.head] = sample;
            for coefficients in &COEFFICIENTS {
                let mut sum = 0i64;
                let mut pos = self.head;
                for &coefficient in coefficients {
                    sum += i64::from(self.history[pos]) * i64::from(coefficient);
                    pos += 1;
                    if pos == TAPS {
                        pos = 0;
                    }
                }
                let value =
                    ((sum + 16384) >> 15).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
                if at % 2 == 0 {
                    first = value;
                } else {
                    out[at / 2] = pack(first, value);
                }
                self.prev = value;
                at += 1;
            }
        }
        at / 2
    }

    /// Fade from the last actual DAC sample, then clear filter history so
    /// stale audio cannot emerge after a gap or a native-rate stream.
    pub fn silence(&mut self, out: &mut [u32]) -> usize {
        let words = out.len().min(FRAME_WORDS);
        let from = i32::from(self.prev);
        let sample = |i: usize| -> i16 {
            if i >= FADE {
                0
            } else {
                (from * (FADE - 1 - i) as i32 / FADE as i32) as i16
            }
        };
        for (i, word) in out[..words].iter_mut().enumerate() {
            *word = pack(sample(2 * i), sample(2 * i + 1));
        }
        *self = Self::default();
        words
    }
}

pub fn pack(first: i16, second: i16) -> u32 {
    u32::from(first as u16) | (u32::from(second as u16) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    fn unpack(words: &[u32]) -> Vec<i16> {
        words
            .iter()
            .flat_map(|w| [*w as u16 as i16, (*w >> 16) as u16 as i16])
            .collect()
    }
    fn render(pcm: &[i16], chunk: usize) -> Vec<i16> {
        let mut up = Upsampler::default();
        let mut result = Vec::new();
        for block in pcm.chunks(chunk) {
            let mut out = [0; FRAME_WORDS];
            let n = up.run(block, &mut out);
            result.extend(unpack(&out[..n]));
        }
        result
    }
    #[test]
    fn packet_boundaries_do_not_change_samples_or_gain() {
        let input: Vec<i16> = (0..1600)
            .map(|i| ((i * 2137 % 60000) - 30000) as i16)
            .collect();
        assert_eq!(render(&input, 320), render(&input, 110));
        assert_eq!(render(&input, 320), render(&input, 2));
        let dc = render(&[8000; 640], 320);
        assert_eq!(dc.len(), 1920);
        assert!(dc[200..].iter().all(|&x| x == 8000));
    }
    #[test]
    fn speech_band_is_preserved_and_ultrasonic_images_are_suppressed() {
        let input: Vec<i16> = (0..3200)
            .map(|i| (12000.0 * (std::f64::consts::TAU * 3000.0 * i as f64 / 16000.0).sin()) as i16)
            .collect();
        let out = render(&input, 110);
        let signal = &out[480..];
        let amplitude = |hz: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &x) in signal.iter().enumerate() {
                let phase = std::f64::consts::TAU * hz * i as f64 / 48000.0;
                re += f64::from(x) * phase.cos();
                im += f64::from(x) * phase.sin();
            }
            2.0 * (re * re + im * im).sqrt() / signal.len() as f64
        };
        assert!((amplitude(3000.0) - 12000.0).abs() < 50.0);
        assert!(amplitude(13000.0) < 3.0);
        assert!(amplitude(19000.0) < 3.0);
    }
    #[test]
    fn native_48k_is_exact_and_a_gap_fades_without_stale_history() {
        let mut up = Upsampler::default();
        let mut out = [0; FRAME_WORDS];
        let input = [-32768, -12, 0, 37, 32767, 15000];
        let n = up.run_at(48, &input, &mut out);
        assert_eq!(unpack(&out[..n]), input);
        let n = up.silence(&mut out);
        let faded = unpack(&out[..n]);
        assert!(faded[0] > 14000);
        assert!(faded[240..].iter().all(|&x| x == 0));
        let n = up.run(&[0; 320], &mut out);
        assert!(out[..n].iter().all(|&x| x == 0));
        assert_eq!(up.run_at(32, &input, &mut out), 0);
    }
}
