//! Offline preparation for the USB sample-rate experiment. See
//! docs/usb-audio-rate-experiment.md. Uses the production EQ and codec.
#![allow(dead_code)]
#[path = "../src/audio/codec.rs"]
pub mod codec;
mod ble { pub use crate::codec; }
#[path = "../host/crates/deckd/src/audio/speaker_eq.rs"]
mod speaker_eq;
#[path = "../host/crates/deckd/src/audio/adpcm.rs"]
mod adpcm;
use std::{env, fs};

fn main() {
    let args: Vec<_> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("eq" | "eq-unity") if args.len() == 4 => {
            // Input: interleaved stereo f32 LE at 48 kHz. Output: mono i16 LE.
            let raw = fs::read(&args[2]).expect("read source");
            assert!(!raw.is_empty() && raw.len() % 8 == 0);
            let mut source: Vec<f32> = raw.chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
            assert!(source.iter().all(|s| s.is_finite()));
            let peak = source.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            assert!(peak > 0.001);
            // One common gain, before EQ. Never normalize the variants separately.
            let gain = if args[1] == "eq-unity" { 1.0 } else { 0.5 / peak };
            for sample in &mut source { *sample *= gain; }
            let mut mono = vec![0i16; source.len() / 2];
            speaker_eq::SpeakerEq::at_rate(48_000).render(&source, &mut mono);
            fs::write(&args[3], mono.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            println!("EQ master: {} samples, 48 kHz; gain {}, production curve enabled", mono.len(), gain);
        }
        Some("encode" | "encode-quality") if args.len() == 6 => {
            // Input: mono i16 LE at the declared rate. Output: 64-byte HID reports.
            let rate: u32 = args[2].parse().unwrap();
            assert!(matches!(rate, 16_000 | 48_000));
            let raw = fs::read(&args[3]).expect("read PCM");
            assert!(!raw.is_empty() && raw.len() % 2 == 0);
            let pcm: Vec<i16> = raw.chunks_exact(2)
                .map(|b| i16::from_le_bytes(b.try_into().unwrap())).collect();
            let stream: u8 = args[5].parse().unwrap();
            let mut reports = Vec::new();
            let mut frame = [0; codec::MAX_FRAME];
            let mut report = [0; 64];
            let n = codec::start_frame(stream, &mut frame);
            frame[3] = (rate / 1000) as u8;
            codec::report(&frame[..n], &mut report).unwrap();
            reports.extend(report);
            let per_frame = codec::samples_per_frame(codec::REPORT_FRAME);
            let mut state = codec::State::default();
            let mut seq = 0u8;
            let mut decoded = Vec::new();
            for chunk in pcm.chunks(per_frame) {
                let mut padded = vec![0; per_frame];
                padded[..chunk.len()].copy_from_slice(chunk);
                let n = if args[1] == "encode-quality" {
                    adpcm::audio_frame(stream, seq, &mut state, &padded, &mut frame)
                } else {
                    codec::audio_frame(stream, seq, &mut state, &padded, &mut frame)
                };
                codec::report(&frame[..n], &mut report).unwrap();
                reports.extend(report);
                if let codec::Frame::Audio { mut state, data, .. } = codec::parse(&frame[..n]) {
                    let mut out = vec![0; per_frame];
                    codec::decode(&mut state, data, &mut out);
                    decoded.extend(out);
                }
                seq = seq.wrapping_add(1);
            }
            let n = codec::end_frame(stream, seq, codec::END_RELEASE, &mut frame);
            codec::report(&frame[..n], &mut report).unwrap();
            reports.extend(report);
            fs::write(&args[4], &reports).unwrap();
            fs::write(format!("{}.decoded.raw", args[4]), decoded.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            let signal: f64 = pcm.iter().map(|&s| f64::from(s).powi(2)).sum();
            let error: f64 = pcm.iter().zip(&decoded).map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2)).sum();
            println!("{rate} Hz: {} reports, {} samples, codec SNR {:.2} dB", reports.len() / 64, decoded.len(), 10.0 * (signal / error).log10());
        }
        _ => panic!("usage: usb-audio-rate eq|eq-unity stereo48.f32 mono48.raw | encode RATE mono.raw reports.hid STREAM"),
    }
}
