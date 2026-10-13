//! The speaker's sample path from the wire to the DAC: 16 kHz to 48 kHz by
//! linear interpolation (speech through a small speaker needs no better, and
//! the DAC's own filter follows), packed the way the audio block's 16-bit
//! mono TX channel takes it: two samples a 32-bit word, the first in the low
//! half. (The pinned HAL documents one sample a word in the low half; played
//! that way everything comes out at half speed.) Pure, so the host tests
//! mount it.
use super::codec;

/// DAC words per 20 ms frame: 960 samples at 48 kHz, two a word.
pub const FRAME_WORDS: usize = 3 * codec::FRAME_SAMPLES / 2;
/// How long a gap's silence takes to fade out from the last sample, in
/// 16 kHz samples (5 ms).
const FADE: usize = 80;

#[derive(Default)]
pub struct Upsampler {
    prev: i16,
}

impl Upsampler {
    /// `input` (an even number of 16 kHz samples, at most a frame) as packed
    /// DAC words in `out`. Returns the words written.
    pub fn run(&mut self, input: &[i16], out: &mut [u32]) -> usize {
        let mut samples = [0i16; 3 * codec::FRAME_SAMPLES];
        let mut n = 0;
        for (&x, slot) in input.iter().zip(samples.chunks_exact_mut(3)) {
            let a = i32::from(self.prev);
            let d = i32::from(x) - a;
            slot[0] = clamp(a + d / 3);
            slot[1] = clamp(a + 2 * d / 3);
            slot[2] = x;
            self.prev = x;
            n += 3;
        }
        let mut words = 0;
        for (pair, word) in samples[..n].chunks_exact(2).zip(out.iter_mut()) {
            *word = pack(pair[0], pair[1]);
            words += 1;
        }
        words
    }

    /// 20 ms of silence, faded out from the last sample over 5 ms so a gap
    /// does not click.
    pub fn silence(&mut self, out: &mut [u32]) -> usize {
        let mut pcm = [0i16; codec::FRAME_SAMPLES];
        let from = i32::from(self.prev);
        for (i, sample) in pcm.iter_mut().take(FADE).enumerate() {
            *sample = (from * (FADE - 1 - i) as i32 / FADE as i32) as i16;
        }
        self.run(&pcm, out)
    }
}

/// One DAC word: `first` plays before `second`.
pub fn pack(first: i16, second: i16) -> u32 {
    u32::from(first as u16) | (u32::from(second as u16) << 16)
}

fn clamp(sample: i32) -> i16 {
    sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpack(words: &[u32]) -> std::vec::Vec<i16> {
        words
            .iter()
            .flat_map(|w| [*w as u16 as i16, (*w >> 16) as u16 as i16])
            .collect()
    }

    #[test]
    fn a_frame_fills_480_words_first_sample_in_the_low_half() {
        let mut up = Upsampler::default();
        let input: std::vec::Vec<i16> = (0..codec::FRAME_SAMPLES as i16).map(|i| i * 3).collect();
        let mut out = [0u32; FRAME_WORDS];
        assert_eq!(up.run(&input, &mut out), FRAME_WORDS);
        let samples = unpack(&out);
        assert_eq!(samples.len(), 960);
        // From 0 to 0, then 0 -> 3 in thirds, then 3 -> 6, ...: the ramp
        // goes up by one a 48 kHz sample, in order.
        for (i, s) in samples.iter().enumerate() {
            assert_eq!(*s as i32, (i as i32 - 2).max(0), "sample {i}");
        }
        assert_eq!(out[2], pack(2, 3));
    }

    #[test]
    fn interpolation_runs_from_the_previous_frame_and_clamps() {
        let mut up = Upsampler::default();
        let mut out = [0u32; FRAME_WORDS];
        up.run(&[i16::MAX, i16::MAX], &mut out);
        let n = up.run(&[i16::MIN, i16::MIN], &mut out);
        assert_eq!(n, 3);
        let samples = unpack(&out[..n]);
        // From MAX towards MIN in thirds, then MIN held.
        assert_eq!(&samples[..], &[10922, -10923, -32768, -32768, -32768, -32768]);
    }

    #[test]
    fn silence_fades_out_and_ends_at_zero() {
        let mut up = Upsampler::default();
        let mut out = [0u32; FRAME_WORDS];
        up.run(&[8000, 8000], &mut out);
        assert_eq!(up.silence(&mut out), FRAME_WORDS);
        let samples = unpack(&out);
        // No step bigger than the fade's slope (8000 / 80 a 16 kHz sample,
        // a third of that per 48 kHz sample, plus rounding).
        let mut last = 8000i32;
        for s in &samples {
            assert!((last - *s as i32).abs() <= 8000 / 80 / 3 + 2, "step {last} -> {s}");
            last = *s as i32;
        }
        assert!(samples[3 * FADE..].iter().all(|s| *s == 0));
    }
}
