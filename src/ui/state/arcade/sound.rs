//! Original three-voice chiptunes and short effects, generated at 16 kHz.
//! Pulse lead, triangle bass and noise percussion; no samples or allocation.
//! The speaker owns the synthesizer. UI publishes one atomic command, so mute
//! and leaving the face cannot be stuck behind queued audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Effect {
    None,
    Start,
    Flap,
    Score,
    Shot,
    Hit,
    Over,
    Win,
}
// track 0..3, mode 0..2, effect, 16-bit event serial, volume 0..10.
pub fn command(track: u8, mode: u8, effect: Effect, serial: u16, volume: u8) -> u32 {
    u32::from(track.min(3))
        | (u32::from(mode.min(2)) << 2)
        | ((effect as u32) << 4)
        | (u32::from(serial) << 8)
        | (u32::from(volume.min(10)) << 24)
}
pub fn enabled(word: u32) -> bool {
    word & 12 != 0 && (word >> 24) & 15 != 0
}
// Frequencies for C4..B5. Rests are zero. These melodies are original.
const HZ: [u16; 24] = [
    262, 277, 294, 311, 330, 349, 370, 392, 415, 440, 466, 494, 523, 554, 587, 622, 659, 698, 740,
    784, 831, 880, 932, 988,
];
const LEADS: [[u8; 32]; 3] = [
    [
        13, 0, 17, 20, 17, 0, 15, 0, 13, 17, 20, 0, 22, 20, 17, 0, 18, 0, 22, 20, 18, 17, 15, 0,
        13, 0, 8, 10, 13, 0, 0, 0,
    ],
    [
        1, 0, 8, 0, 11, 8, 6, 0, 1, 0, 6, 8, 11, 0, 8, 0, 4, 0, 11, 0, 13, 11, 8, 0, 6, 8, 6, 4, 1,
        0, 0, 0,
    ],
    [
        10, 10, 0, 17, 15, 0, 13, 15, 10, 0, 8, 10, 13, 15, 17, 0, 18, 0, 17, 15, 13, 0, 10, 0, 8,
        10, 13, 8, 10, 0, 0, 0,
    ],
];
const BASS: [[u8; 8]; 3] = [
    [1, 1, 6, 6, 8, 8, 1, 8],
    [1, 1, 6, 6, 4, 4, 8, 8],
    [10, 10, 6, 6, 1, 1, 8, 8],
];
#[derive(Default)]
pub struct Synth {
    track: u8,
    serial: u16,
    effect: u8,
    effect_left: u32,
    effect_len: u32,
    sample: u32,
    lead_phase: u32,
    bass_phase: u32,
    fx_phase: u32,
    noise: u32,
    gain: i32,
}
fn phase(phase: &mut u32, hz: u32) -> u32 {
    *phase = phase.wrapping_add(hz * 65536 / 16000);
    *phase & 65535
}
impl Synth {
    pub fn pending(&self) -> bool {
        self.effect_left > 0
    }
    pub fn run(&mut self, word: u32, pcm: &mut [i16]) {
        let track = (word & 3) as u8;
        let mode = (word >> 2) & 3;
        let serial = (word >> 8) as u16;
        if track != self.track {
            self.track = track;
            self.sample = 0;
            self.lead_phase = 0;
            self.bass_phase = 0;
        }
        if serial != self.serial {
            self.serial = serial;
            self.effect = ((word >> 4) & 15) as u8;
            self.effect_len = match self.effect {
                1 => 3200,
                2 => 1120,
                3 => 2400,
                4 => 560,
                5 => 2400,
                6 => 8000,
                7 => 9600,
                _ => 0,
            };
            self.effect_left = self.effect_len;
            self.fx_phase = 0;
        }
        if !enabled(word) {
            self.effect_left = 0;
            self.gain = 0;
            pcm.fill(0);
            return;
        }
        let volume = ((word >> 24) & 15).min(10) as i32;
        for sample in pcm {
            let mut value = 0;
            self.noise = self.noise.wrapping_mul(1664525).wrapping_add(1013904223);
            if track > 0 && mode == 2 {
                let beat = [2000, 2400, 1600][track as usize - 1];
                let step = self.sample / beat;
                let at = self.sample % beat;
                let note = LEADS[track as usize - 1][step as usize % 32];
                // Short attack and release soften edges, retaining the 8-bit pulse timbre.
                let env = at.min(80).min(beat.saturating_sub(at).min(240)) as i32;
                if note > 0 {
                    value +=
                        if phase(&mut self.lead_phase, u32::from(HZ[note as usize - 1])) < 16384 {
                            1950
                        } else {
                            -650
                        } * env
                            / 80;
                }
                let bass = HZ[BASS[track as usize - 1][(step / 4) as usize % 8] as usize - 1] / 4;
                let p = phase(&mut self.bass_phase, u32::from(bass)) as i32;
                value += ((p - 32768).abs() - 16384) * 450 / 16384;
                if at < 320 {
                    value += if self.noise & 0x80000000 != 0 {
                        200
                    } else {
                        -200
                    } * (320 - at) as i32
                        / 320;
                }
            }
            if self.effect_left > 0 {
                let t = self.effect_len - self.effect_left;
                let hz = match self.effect {
                    1 => [523, 659, 784, 1047][(t / 800).min(3) as usize],
                    2 => 450 + t / 2,
                    3 => {
                        if t < 1200 {
                            784
                        } else {
                            1047
                        }
                    }
                    4 => 1500u32.saturating_sub(t * 2).max(180),
                    5 => 180 + (self.noise & 127),
                    6 => 450u32.saturating_sub(t / 24).max(80),
                    7 => [523, 659, 784, 1047][(t / 2400).min(3) as usize],
                    _ => 0,
                };
                let amp = 1000 * self.effect_left.min(240).min(t.min(40)) as i32 / 40;
                let high = if self.effect == 5 {
                    self.noise & 0x80000000 != 0
                } else {
                    phase(&mut self.fx_phase, hz) < 32768
                };
                value += if high { amp } else { -amp };
                self.effect_left -= 1;
            }
            self.sample = self.sample.wrapping_add(1);
            self.gain = (self.gain + 1).min(160);
            *sample = (value * volume * self.gain / 10 / 160).clamp(-4096, 4096) as i16;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mute_volume_and_sfx_only_are_silent_without_effects() {
        let mut synth = Synth::default();
        let mut pcm = [1; 320];
        for word in [
            0,
            command(1, 0, Effect::Shot, 1, 6),
            command(1, 2, Effect::Shot, 2, 0),
            command(1, 1, Effect::None, 3, 6),
        ] {
            synth.run(word, &mut pcm);
            assert!(pcm.iter().all(|&s| s == 0));
        }
    }
    #[test]
    fn original_tracks_differ_and_stay_bounded() {
        let mut hashes = [0u64; 3];
        for track in 1..=3 {
            let mut s = Synth::default();
            let mut pcm = [0; 320];
            for _ in 0..200 {
                s.run(command(track, 2, Effect::None, 0, 10), &mut pcm);
                for n in pcm {
                    assert!(n.abs() <= 4096);
                    hashes[track as usize - 1] = hashes[track as usize - 1]
                        .wrapping_mul(31)
                        .wrapping_add(n as u64);
                }
            }
        }
        assert!(hashes.iter().all(|&h| h != 0));
        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[1], hashes[2]);
    }
    #[test]
    fn effects_finish_do_not_retrigger_and_mute_cancels() {
        for fx in [
            Effect::Start,
            Effect::Flap,
            Effect::Score,
            Effect::Shot,
            Effect::Hit,
            Effect::Over,
            Effect::Win,
        ] {
            let mut s = Synth::default();
            let mut pcm = [0; 320];
            let word = command(0, 1, fx, 1, 6);
            s.run(word, &mut pcm);
            assert!(pcm.iter().any(|&n| n != 0));
            for _ in 0..40 {
                s.run(word, &mut pcm);
            }
            assert!(!s.pending());
            assert!(pcm.iter().all(|&n| n == 0));
            s.run(command(0, 1, fx, 2, 6), &mut pcm);
            assert!(s.pending());
            s.run(0, &mut pcm);
            assert!(!s.pending());
            assert!(pcm.iter().all(|&n| n == 0));
        }
    }
}
