//! Export original chiptunes/effects for review, without opening an audio device.
//! cargo run --manifest-path native/core/Cargo.toml --example arcade-audio \
//!   --target aarch64-apple-darwin --target-dir target/native-core-preview -- target/arcade-audio
#[path = "../../../src/ui/state/arcade/sound.rs"]
#[allow(dead_code)]
mod sound;
use std::{fs, io::Write, path::Path};
fn wav(path: &Path, word: u32, samples: usize) -> std::io::Result<()> {
    let mut synth = sound::Synth::default();
    let mut pcm = vec![0i16; samples];
    synth.run(word, &mut pcm);
    let mut f = fs::File::create(path)?;
    let size = samples as u32 * 2;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + size).to_le_bytes())?;
    f.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
    f.write_all(&16000u32.to_le_bytes())?;
    f.write_all(&32000u32.to_le_bytes())?;
    f.write_all(b"\x02\0\x10\0data")?;
    f.write_all(&size.to_le_bytes())?;
    for sample in pcm {
        f.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}
fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/arcade-audio".into());
    let out = Path::new(&out);
    fs::create_dir_all(out)?;
    for (track, name) in [(1, "flappy-bgm"), (2, "snake-bgm"), (3, "raiden-bgm")] {
        wav(
            &out.join(format!("{name}.wav")),
            sound::command(track, 2, sound::Effect::None, 0, 6),
            sound::music_loop_samples(track) as usize,
        )?;
    }
    use sound::Effect::*;
    for (fx, name) in [
        (Start, "start"),
        (Flap, "flap"),
        (Score, "score"),
        (Shot, "shot"),
        (Hit, "hit"),
        (Over, "game-over"),
        (Win, "win"),
    ] {
        wav(
            &out.join(format!("{name}.wav")),
            sound::command(0, 1, fx, 1, 6),
            16000,
        )?;
    }
    println!(
        "Exported three music loops and seven effects to {} (no playback)",
        out.display()
    );
    Ok(())
}
