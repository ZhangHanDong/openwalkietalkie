//! The voice link's wire format: IMA ADPCM (16 kHz mono, 4 bits a sample)
//! in self-contained frames, shared by the firmware and deckd.
//!
//! No `std`, no allocation, no hardware: the firmware mounts it as
//! `audio::codec`, the host tests and deckd mount the same file by path, so
//! both ends can never disagree about a bit.
//!
//! Every value written to the audio service's characteristics is one frame,
//! and byte 0 says which:
//!
//! | kind | name   | layout after the kind byte                       | who sends     |
//! |------|--------|--------------------------------------------------|---------------|
//! | 0x01 | START  | stream, codec (1), rate kHz (16), frame ms (20)  | both          |
//! | 0x02 | AUDIO  | stream, seq, predictor i16 LE, step index, data  | both          |
//! | 0x03 | END    | stream, next seq, reason                         | both          |
//! | 0x10 | MIC    | mode (0 off, 1 on, 2 loopback), seconds (1..=60) | host to deck  |
//! | 0x11 | LISTEN | on (0 or 1)                                      | host to deck  |
//! | 0x12 | INFO   | the five info bytes                              | deck to host  |
//!
//! An AUDIO frame carries the encoder state from *before* its first sample,
//! so each frame decodes on its own: a lost frame costs its 20 ms and
//! nothing after it. Samples come two per data byte, low nibble first.
//!
//! Over the USB cable the same frames travel on report channel [`CHANNEL`]
//! of the HID vendor collection, one frame per report, so a frame is at
//! most [`REPORT_FRAME`] bytes there and an AUDIO frame carries
//! `samples_per_frame(REPORT_FRAME)` samples. LISTEN and INFO exist only
//! on that link: LISTEN is the cable's counterpart of subscribing to the
//! mic characteristic, and the deck answers it with INFO, the info
//! characteristic's value, which is how the host learns the link is ready.

/// IMA ADPCM step sizes (the standard 89-entry table).
pub const STEP: [i16; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408,
    449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
    2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630,
    9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];

/// How each code moves the step index.
const INDEX_STEP: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

pub const KIND_START: u8 = 0x01;
pub const KIND_AUDIO: u8 = 0x02;
pub const KIND_END: u8 = 0x03;
pub const KIND_MIC: u8 = 0x10;
pub const KIND_LISTEN: u8 = 0x11;
pub const KIND_INFO: u8 = 0x12;

/// The vendor collection's report id.
pub const REPORT_ID: u8 = 6;
/// The HID report channel the frames use over USB: `[6, CHANNEL, len,
/// frame[..len]]`, one frame per 64-byte report.
pub const CHANNEL: u8 = 7;
/// The longest frame one report carries (64 less the report id, the
/// channel and the length byte).
pub const REPORT_FRAME: usize = 61;
/// The info characteristic's five bytes: version, codec bits, rate (kHz),
/// frame (ms), features (bit 0 a microphone, bit 1 a speaker).
pub const INFO_LEN: usize = 5;

pub const CODEC_IMA_ADPCM: u8 = 1;
/// Optional USB speaker rate; microphone and BLE remain at RATE_HZ.
pub const USB_SPEAKER_RATE_HZ: u32 = 48_000;
/// INFO feature bit: USB speaker accepts 48 kHz ADPCM STARTs.
pub const FEATURE_USB_SPEAKER_48K: u8 = 1 << 2;
pub const RATE_HZ: u32 = 16_000;
pub const RATE_KHZ: u8 = 16;
pub const FRAME_MS: u8 = 20;
/// 20 ms at 16 kHz.
pub const FRAME_SAMPLES: usize = 320;
/// kind, stream, seq, predictor (2), step index.
pub const AUDIO_HEADER: usize = 6;
/// The largest frame either end builds: a full 20 ms AUDIO frame.
pub const MAX_FRAME: usize = AUDIO_HEADER + FRAME_SAMPLES / 2;

/// MIC modes: stop, capture as a press would, or capture with the speaker
/// left on (a loopback test of both paths; never used for speech).
pub const MIC_OFF: u8 = 0;
pub const MIC_ON: u8 = 1;
pub const MIC_LOOPBACK: u8 = 2;

/// END reasons.
pub const END_RELEASE: u8 = 0;
pub const END_ABORT: u8 = 1;
pub const END_LIMIT: u8 = 2;
pub const END_ERROR: u8 = 3;

/// The encoder or decoder state between samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub predictor: i16,
    pub index: u8,
}

impl State {
    /// One sample in, one 4-bit code out; the state follows the decoder's
    /// reconstruction, so both ends stay in step.
    pub fn encode(&mut self, sample: i16) -> u8 {
        let step = i32::from(STEP[usize::from(self.index)]);
        let mut diff = i32::from(sample) - i32::from(self.predictor);
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut delta = step >> 3;
        if diff >= step {
            code |= 4;
            diff -= step;
            delta += step;
        }
        if diff >= step >> 1 {
            code |= 2;
            diff -= step >> 1;
            delta += step >> 1;
        }
        if diff >= step >> 2 {
            code |= 1;
            delta += step >> 2;
        }
        self.advance(code, delta);
        code
    }

    /// One 4-bit code in, one sample out.
    pub fn decode(&mut self, code: u8) -> i16 {
        let code = code & 0x0F;
        let step = i32::from(STEP[usize::from(self.index)]);
        let mut delta = step >> 3;
        if code & 4 != 0 {
            delta += step;
        }
        if code & 2 != 0 {
            delta += step >> 1;
        }
        if code & 1 != 0 {
            delta += step >> 2;
        }
        self.advance(code, delta);
        self.predictor
    }

    fn advance(&mut self, code: u8, delta: i32) {
        let predictor = if code & 8 != 0 {
            i32::from(self.predictor) - delta
        } else {
            i32::from(self.predictor) + delta
        };
        self.predictor = predictor.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        let index = i32::from(self.index) + i32::from(INDEX_STEP[usize::from(code)]);
        self.index = index.clamp(0, (STEP.len() - 1) as i32) as u8;
    }
}

/// Encode `pcm` (an even number of samples) into `out`, two samples a byte,
/// low nibble first. Returns the bytes written.
pub fn encode(state: &mut State, pcm: &[i16], out: &mut [u8]) -> usize {
    let mut n = 0;
    for (pair, byte) in pcm.chunks_exact(2).zip(out.iter_mut()) {
        let low = state.encode(pair[0]);
        let high = state.encode(pair[1]);
        *byte = low | (high << 4);
        n += 1;
    }
    n
}

/// Decode `data` into `out` (two samples a byte). Returns the samples written.
pub fn decode(state: &mut State, data: &[u8], out: &mut [i16]) -> usize {
    let mut n = 0;
    for (byte, pair) in data.iter().zip(out.chunks_exact_mut(2)) {
        pair[0] = state.decode(byte & 0x0F);
        pair[1] = state.decode(byte >> 4);
        n += 2;
    }
    n
}

/// How many samples one AUDIO frame may carry when a value can be at most
/// `max_value` bytes: a full 20 ms frame when it fits, else as many as do
/// (always even).
pub const fn samples_per_frame(max_value: usize) -> usize {
    if max_value <= AUDIO_HEADER {
        return 0;
    }
    let fit = (max_value - AUDIO_HEADER) * 2;
    if fit < FRAME_SAMPLES {
        fit
    } else {
        FRAME_SAMPLES
    }
}

pub fn start_frame(stream: u8, out: &mut [u8]) -> usize {
    out[..5].copy_from_slice(&[KIND_START, stream, CODEC_IMA_ADPCM, RATE_KHZ, FRAME_MS]);
    5
}

pub fn end_frame(stream: u8, next_seq: u8, reason: u8, out: &mut [u8]) -> usize {
    out[..4].copy_from_slice(&[KIND_END, stream, next_seq, reason]);
    4
}

pub fn mic_frame(mode: u8, seconds: u8, out: &mut [u8]) -> usize {
    out[..3].copy_from_slice(&[KIND_MIC, mode, seconds]);
    3
}

/// USB only: subscribe to (or drop) the deck's microphone frames.
pub fn listen_frame(on: bool, out: &mut [u8]) -> usize {
    out[..2].copy_from_slice(&[KIND_LISTEN, u8::from(on)]);
    2
}

/// USB only: the deck's answer to LISTEN, the info characteristic's bytes.
pub fn info_frame(info: &[u8; INFO_LEN], out: &mut [u8]) -> usize {
    out[0] = KIND_INFO;
    out[1..1 + INFO_LEN].copy_from_slice(info);
    1 + INFO_LEN
}

/// A channel-7 report around `frame`: the report id, the channel, the
/// length, the frame, zero padding. `None` when the frame does not fit.
pub fn report(frame: &[u8], out: &mut [u8; 64]) -> Option<()> {
    if frame.is_empty() || frame.len() > REPORT_FRAME {
        return None;
    }
    out.fill(0);
    out[0] = REPORT_ID;
    out[1] = CHANNEL;
    out[2] = frame.len() as u8;
    out[3..3 + frame.len()].copy_from_slice(frame);
    Some(())
}

/// The frame inside a channel-7 report body (with or without its leading
/// report id); `None` for a report of another channel.
pub fn report_frame(body: &[u8]) -> Option<&[u8]> {
    let body = match body {
        [REPORT_ID, CHANNEL, ..] => &body[1..],
        _ => body,
    };
    let [CHANNEL, len, payload @ ..] = body else {
        return None;
    };
    let len = usize::from(*len).min(REPORT_FRAME).min(payload.len());
    Some(&payload[..len])
}

/// One AUDIO frame: the header with the state before `pcm`, then `pcm`
/// encoded (advancing `state`). `out` needs `AUDIO_HEADER + pcm.len() / 2`.
pub fn audio_frame(stream: u8, seq: u8, state: &mut State, pcm: &[i16], out: &mut [u8]) -> usize {
    let [lo, hi] = state.predictor.to_le_bytes();
    out[..AUDIO_HEADER].copy_from_slice(&[KIND_AUDIO, stream, seq, lo, hi, state.index]);
    AUDIO_HEADER + encode(state, pcm, &mut out[AUDIO_HEADER..])
}

/// A frame, parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    Start { stream: u8, codec: u8, rate_khz: u8, frame_ms: u8 },
    /// `state` is the decoder state before `data`'s first sample.
    Audio { stream: u8, seq: u8, state: State, data: &'a [u8] },
    End { stream: u8, next_seq: u8, reason: u8 },
    Mic { mode: u8, seconds: u8 },
    /// USB only, host to deck.
    Listen { on: bool },
    /// USB only, deck to host.
    Info { bytes: [u8; INFO_LEN] },
    /// Unknown kind or short: ignore it (a newer peer).
    Other,
}

pub fn parse(bytes: &[u8]) -> Frame<'_> {
    match bytes {
        [KIND_START, stream, codec, rate_khz, frame_ms, ..] => Frame::Start {
            stream: *stream,
            codec: *codec,
            rate_khz: *rate_khz,
            frame_ms: *frame_ms,
        },
        [KIND_AUDIO, stream, seq, lo, hi, index, data @ ..] if usize::from(*index) < STEP.len() => {
            Frame::Audio {
                stream: *stream,
                seq: *seq,
                state: State {
                    predictor: i16::from_le_bytes([*lo, *hi]),
                    index: *index,
                },
                data,
            }
        }
        [KIND_END, stream, next_seq, reason, ..] => Frame::End {
            stream: *stream,
            next_seq: *next_seq,
            reason: *reason,
        },
        [KIND_MIC, mode, seconds, ..] => Frame::Mic {
            mode: *mode,
            seconds: *seconds,
        },
        [KIND_LISTEN, on, ..] => Frame::Listen { on: *on != 0 },
        [KIND_INFO, a, b, c, d, e, ..] => Frame::Info {
            bytes: [*a, *b, *c, *d, *e],
        },
        _ => Frame::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, hz: f32, amplitude: f32) -> std::vec::Vec<i16> {
        (0..len)
            .map(|i| {
                let t = i as f32 / RATE_HZ as f32;
                (amplitude * (2.0 * core::f32::consts::PI * hz * t).sin()) as i16
            })
            .collect()
    }

    fn snr_db(reference: &[i16], decoded: &[i16]) -> f32 {
        let signal: f64 = reference.iter().map(|&s| f64::from(s).powi(2)).sum();
        let noise: f64 = reference
            .iter()
            .zip(decoded)
            .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
            .sum();
        (10.0 * (signal / noise.max(1.0)).log10()) as f32
    }

    #[test]
    fn a_speech_band_tone_survives_the_round_trip() {
        let pcm = tone(16_000, 440.0, 12_000.0);
        let mut bytes = std::vec![0u8; pcm.len() / 2];
        let mut out = std::vec![0i16; pcm.len()];
        assert_eq!(encode(&mut State::default(), &pcm, &mut bytes), pcm.len() / 2);
        assert_eq!(decode(&mut State::default(), &bytes, &mut out), pcm.len());
        // Skip the first few ms while the step size climbs to the signal.
        let snr = snr_db(&pcm[400..], &out[400..]);
        assert!(snr > 20.0, "SNR {snr} dB");
    }

    #[test]
    fn known_vector_matches_the_reference_algorithm() {
        // Worked by hand with the IMA/DVI reference: steps 7, 7, 16, 34,
        // 73, 157 (indices 0, 0, 8, 16, 24, 32), so the ramp reconstructs as
        // 0, 11, 41, 104, -32, -13.
        let mut state = State::default();
        let codes: std::vec::Vec<u8> =
            [0i16, 100, 200, 300, -300, 0].iter().map(|&s| state.encode(s)).collect();
        assert_eq!(codes, [0, 7, 7, 7, 15, 0]);
        let mut decoder = State::default();
        let decoded: std::vec::Vec<i16> = codes.iter().map(|&c| decoder.decode(c)).collect();
        assert_eq!(decoded, [0, 11, 41, 104, -32, -13]);
        assert_eq!(decoder, state);
        assert_eq!(state.index, 31);
    }

    #[test]
    fn every_frame_decodes_on_its_own() {
        let pcm = tone(FRAME_SAMPLES * 3, 1000.0, 8000.0);
        let mut encoder = State::default();
        let mut frames = std::vec::Vec::new();
        for (seq, chunk) in pcm.chunks(FRAME_SAMPLES).enumerate() {
            let mut frame = [0u8; MAX_FRAME];
            let n = audio_frame(7, seq as u8, &mut encoder, chunk, &mut frame);
            assert_eq!(n, MAX_FRAME);
            frames.push(frame);
        }
        // Decode only the last frame, as if the first two were lost.
        let Frame::Audio { stream, seq, mut state, data } = parse(&frames[2]) else {
            panic!("not audio");
        };
        assert_eq!((stream, seq, data.len()), (7, 2, FRAME_SAMPLES / 2));
        let mut out = [0i16; FRAME_SAMPLES];
        decode(&mut state, data, &mut out);
        let snr = snr_db(&pcm[2 * FRAME_SAMPLES + 40..], &out[40..]);
        assert!(snr > 15.0, "SNR {snr} dB");
    }

    #[test]
    fn control_frames_round_trip_and_unknown_kinds_are_ignored() {
        let mut buf = [0u8; 8];
        let n = start_frame(3, &mut buf);
        assert_eq!(
            parse(&buf[..n]),
            Frame::Start { stream: 3, codec: CODEC_IMA_ADPCM, rate_khz: 16, frame_ms: 20 }
        );
        let n = end_frame(3, 9, END_ABORT, &mut buf);
        assert_eq!(parse(&buf[..n]), Frame::End { stream: 3, next_seq: 9, reason: END_ABORT });
        let n = mic_frame(MIC_LOOPBACK, 5, &mut buf);
        assert_eq!(parse(&buf[..n]), Frame::Mic { mode: MIC_LOOPBACK, seconds: 5 });
        assert_eq!(parse(&[0x7F, 1, 2]), Frame::Other);
        assert_eq!(parse(&[KIND_AUDIO, 1]), Frame::Other);
        // A step index past the table is malformed, not a crash.
        assert_eq!(parse(&[KIND_AUDIO, 1, 2, 0, 0, 200, 0x11]), Frame::Other);
        let n = listen_frame(true, &mut buf);
        assert_eq!(parse(&buf[..n]), Frame::Listen { on: true });
        let n = info_frame(&[1, 1, 16, 20, 3], &mut buf);
        assert_eq!(parse(&buf[..n]), Frame::Info { bytes: [1, 1, 16, 20, 3] });
        assert_eq!(parse(&[KIND_INFO, 1, 1]), Frame::Other);
    }

    #[test]
    fn a_usb_report_holds_one_frame_of_at_most_sixty_one_bytes() {
        let mut report = [0xAAu8; 64];
        let mut buf = [0u8; MAX_FRAME];
        let n = end_frame(2, 7, END_RELEASE, &mut buf);
        assert_eq!(super::report(&buf[..n], &mut report), Some(()));
        assert_eq!(&report[..7], &[REPORT_ID, CHANNEL, 4, KIND_END, 2, 7, END_RELEASE]);
        assert!(report[7..].iter().all(|&b| b == 0), "padded with zeros");
        assert_eq!(report_frame(&report), Some(&buf[..n]));
        assert_eq!(report_frame(&report[1..]), Some(&buf[..n]), "without the report id");
        assert_eq!(report_frame(&[REPORT_ID, 5, 2, b'{', b'}']), None, "a deck line");
        assert_eq!(report_frame(&[CHANNEL, 9, 1, 2]), Some(&[1u8, 2][..]), "a short body is cut, not refused");
        // A full 20 ms frame does not fit; the frame that does carries 110 samples.
        assert_eq!(super::report(&[0; MAX_FRAME], &mut report), None);
        assert_eq!(super::report(&[], &mut report), None);
        assert_eq!(samples_per_frame(REPORT_FRAME), 110);
        let pcm = [100i16; 110];
        let n = audio_frame(1, 0, &mut State::default(), &pcm, &mut buf);
        assert_eq!(n, REPORT_FRAME);
        assert_eq!(super::report(&buf[..n], &mut report), Some(()));
    }

    #[test]
    fn frames_shrink_to_fit_a_small_attribute() {
        assert_eq!(samples_per_frame(244), FRAME_SAMPLES);
        assert_eq!(samples_per_frame(MAX_FRAME), FRAME_SAMPLES);
        assert_eq!(samples_per_frame(20), 28);
        assert_eq!(samples_per_frame(6), 0);
    }

    #[test]
    fn extremes_clamp_instead_of_wrapping() {
        let mut state = State::default();
        for _ in 0..200 {
            state.encode(i16::MAX);
        }
        assert!(state.predictor > 30_000);
        for _ in 0..200 {
            state.encode(i16::MIN);
        }
        assert!(state.predictor < -30_000);
        assert!(usize::from(state.index) < STEP.len());
    }
}
