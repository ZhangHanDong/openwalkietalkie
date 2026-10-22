//! Export the installed Kokoro engine's normal, sentence-levelled PCM for
//! repeatable deck listening tests. No playback, downloads or settings changes.
#[cfg(target_os = "macos")]
fn main() {
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };

    use arc_swap::ArcSwap;
    use deckd::audio::{kokoro::KokoroTts, traits::*};
    use deckd::kokoro::Settings;

    struct NoFallback;
    impl TtsEngine for NoFallback {
        fn info(&self) -> EngineInfo {
            EngineInfo {
                name: "unavailable".into(),
                on_device: true,
                locales: vec![],
            }
        }
        fn voices(&self) -> Vec<VoiceInfo> {
            vec![]
        }
        fn synthesize(&self, _: &TtsRequest) -> Result<Box<dyn TtsJob>, AudioError> {
            Err(AudioError::Engine(
                "Kokoro must load; no substitute voice for this test".into(),
            ))
        }
    }

    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "usage: tts-clarity MODEL_DIR VOICE TEXT_FILE OUTPUT.f32 SPEED"
    );
    let dir = PathBuf::from(&args[1]);
    let speed: f32 = args[5].parse().expect("speed");
    assert!(speed.is_finite() && (0.5..=2.0).contains(&speed));
    assert!(
        deckd::kokoro::speaker_id(&args[2]).is_some(),
        "unknown Kokoro voice"
    );
    let settings = Settings {
        enabled: true,
        model: dir.file_name().unwrap().to_str().unwrap().into(),
        voices: vec![args[2].clone()],
        threads: 2,
    };
    let engine = Arc::new(KokoroTts::new(
        Arc::new(ArcSwap::from_pointee(settings)),
        dir.parent().unwrap().into(),
        Arc::new(NoFallback),
    ));
    assert!(engine.present(), "model must already be installed");
    engine.ensure_loaded();
    let deadline = Instant::now() + Duration::from_secs(120);
    while !engine.status().loaded {
        let status = engine.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        assert!(
            status.loading && Instant::now() < deadline,
            "model did not load"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let job = engine
        .synthesize(&TtsRequest {
            text: fs::read_to_string(&args[3]).expect("read text"),
            voice: Some(args[2].clone()),
            rate: speed * 0.5,
        })
        .expect("synthesize");
    let format = job.format();
    assert_eq!(format.channels, 1);
    let mut pcm = Vec::<f32>::new();
    loop {
        match job
            .chunks()
            .recv_timeout(Duration::from_secs(120))
            .expect("speech timed out")
        {
            TtsChunk::Pcm(chunk) => pcm.extend(chunk),
            TtsChunk::End => break,
            TtsChunk::Error(error) => panic!("{error}"),
        }
    }
    assert!(!pcm.is_empty() && pcm.iter().all(|s| s.is_finite()));
    fs::write(
        &args[4],
        pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),
    )
    .unwrap();
    let metadata = serde_json::json!({
        "engine": "kokoro", "voice": args[2], "rate_hz": format.rate,
        "channels": format.channels, "samples": pcm.len(), "speed": speed,
        "duration_s": pcm.len() as f64 / f64::from(format.rate),
        "sentence_leveling": "production",
    });
    fs::write(
        format!("{}.json", args[4]),
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();
    println!("{metadata}");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("The deck's Kokoro engine requires macOS");
}
