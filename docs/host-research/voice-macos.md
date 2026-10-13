# voice-macos

VOICE PIPELINE ON macOS INSIDE A RUST DAEMON — findings (checked 2026-09-18 on the dev Mac: sw_vers reports macOS 27.0 build 26A428, Darwin 27.0.0; rustc/cargo 1.92.0).

DEVICE AS SEEN BY COREAUDIO (verified with a scratchpad CoreAudio lister): "Codex Micro" is TWO devices: input AudioDeviceID 101, UID "C2-2D-81-AB-88-81:input", transport 'blue', 1 ch, nominal 16000 Hz (= mSBC); output AudioDeviceID 95, UID "C2-2D-81-AB-88-81:output", 2 ch, nominal 44100 Hz (= A2DP). Bluetooth services HFP/AVRCP/A2DP/HID/BLE. AudioDeviceIDs are per-connection; address devices by UID. Firmware docs (native/README.md:9-18, validation 2026-09-09 at :243-249) confirm: any process starting IO on the HFP input makes macOS open SCO/eSCO, the Codex Micro OUTPUT then switches to the 16 kHz (mSBC) / 8 kHz (CVSD) voice link (A2DP suspended); when the last input IO stops the output returns to 44.1 kHz A2DP. This does not touch the Mac's default output (AirPods/speakers) — SCO is per-link; tools/test-speaker.c already plays to Codex Micro "without changing the Mac's default output or volume". Consequence for the design: the Codex Micro output device's nominal sample rate flips 44100→16000→44100 around every PTT hold; cpal (0.18.2, coreaudio backend) registers a kAudioDevicePropertyNominalSampleRate listener and reports StreamInvalidated, and kAudioDevicePropertyDeviceIsAlive → DeviceNotAvailable on BT drop, so the output stream MUST be re-opened after each PTT session (fits barge-in: PTT press tears playback down anyway). Echo quality with speaker+mic active is unevaluated (native/README.md:256-257).

(1) CAPTURE FROM RUST. Options: cpal 0.18.2 (enumerate Host::input_devices(), match Device::name()=="Codex Micro" and input-capable, or by UID via objc2-core-audio 0.3.2; cpal pins the AUHAL to the device with kAudioOutputUnitProperty_CurrentDevice; request exactly the device's default config (16000 Hz / 1 ch / f32) — if you request another rate cpal calls set_sample_rate on kAudioDevicePropertyNominalSampleRate and waits up to 1 s, which a Bluetooth HFP endpoint cannot honour). coreaudio-rs 0.14.2 (AudioUnit::new(IOType::HalOutput), set_property, set_input_callback, macos_helpers::{audio_unit_from_device_id, get_device_name, set_device_sample_rate}) if you need lower-level control (e.g. kAudioDevicePropertyDeviceIsRunningSomewhere to detect the Codex app already holding the mic). objc2-avf-audio 0.3.2 AVAudioEngine works but on macOS its inputNode follows the system default input unless you set kAudioOutputUnitProperty_CurrentDevice on inputNode.audioUnit (Apple forums 71008/133080/775015) — no advantage over cpal for raw PCM. Open/close only while PTT is held: build the input Stream on `ptt act:1`, drop it on `ptt act:0`, `abort:1`, `up`, host-gap, or the firmware's 60 s auto-release (docs/agent-command-deck.md:218-221). Detect CVSD (8 kHz nominal) vs mSBC (16 kHz) from the device's nominal rate at open time and resample with rubato 5.0.0 for engines that need 16 k/24 k.

(2) STT WITH PARTIALS. (a) Apple SFSpeechRecognizer via objc2-speech 0.3.2 — bound: SFSpeechRecognizer::{initWithLocale, isAvailable, supportsOnDeviceRecognition, requestAuthorization [block2], recognitionTaskWithRequest_resultHandler [block2 + SFSpeechRecognitionRequest/Result/Task features]}, SFSpeechAudioBufferRecognitionRequest (append AVAudioPCMBuffer, shouldReportPartialResults, requiresOnDeviceRecognition). Verified locally with a Swift probe: en-US available=true, supportsOnDeviceRecognition=true; zh-CN and en-GB on-device=false ("No Assistant asset for language"); 62 supported locales. Apple docs: macOS 10.15+, not deprecated; server mode stops tasks >1 minute and is rate-limited per device/app per day; on-device honoured only when supportsOnDeviceRecognition, "won't be as accurate"; requestAuthorization CRASHES if Info.plist lacks NSSpeechRecognitionUsageDescription. Cost 0, audio never leaves the Mac. Latency of partials: not measured (typically sub-second; uncertain). (b) SpeechAnalyzer/SpeechTranscriber (macOS 26.0+): Swift-only actor API (interfaceLanguage swift, no ObjC → not reachable from objc2); local probe: 45 supported locales, 9 English locales installed; volatile results = partials via AsyncSequence. Needs a Swift helper (@_cdecl static lib the way fluidaudio-rs does, or an XPC/subprocess). (c) whisper.cpp via whisper-rs 0.16.0 (crates.io 2026-03-12; repo moved to codeberg.org/tazz4843/whisper-rs, GitHub archived 2025-07-30; features metal, coreml, openmp; WhisperVadContext with Silero v6.2.0). Not streaming by design — sliding-window re-decoding (whisper.cpp stream example: `--step 500 --length 5000` or VAD mode `--step 0 --length 30000 -vth 0.6`), partials are re-decodes of the window. Apple Silicon: whisper.cpp large on M4 24 GB ≈1.23 s/clip (mac-whisper-speedtest); large-v3-turbo on M2 Pro Metal ≈60 s audio in 2.8 s (secondary blog); RAM tiny 273 MB … large 3.9 GB; CoreML encoder >3x faster but first run compiles on ANE (slow), macOS 14+ recommended. (d) NVIDIA Parakeet TDT 0.6B v3 (25 languages): fluidaudio-rs 0.14.1 (2026-05-11; CoreML/ANE, macOS 14+, Swift 5.10+, build.rs compiles a Swift package; streaming via SlidingWindowAsrManager + 120M EOU model; ~190x RT batch on M4 Pro; first init downloads ~500 MB and takes 20-30 s ANE compile) — fastest local engine in mac-whisper-speedtest (0.19 s vs whisper.cpp 1.23 s); parakeet-rs 0.3.7 (ONNX; EOU 160 ms chunks; CoreML EP "unstable", use CPU/WebGPU); sherpa-onnx 1.13.8. (e) Kyutai STT (stt-1b-en_fr: 0.5 s delay + semantic VAD; 2.6b-en: 2.5 s delay), true streaming, Rust stt-rs on candle 0.9.1 with `metal` feature, weights CC-BY-4.0. (f) Cloud: OpenAI Realtime transcription — wss://api.openai.com/v1/realtime?intent=transcription, session type "transcription", model gpt-live-transcribe $0.017/min (also gpt-realtime-whisper $0.017/min), input audio/pcm 24 kHz base64 via input_audio_buffer.append, partials `conversation.item.input_audio_transcription.delta`, finals `.completed`; batch REST: gpt-4o-transcribe ≈$0.006/min, gpt-4o-mini-transcribe ≈$0.003/min, gpt-transcribe $0.0045/min, whisper-1 $0.006/min. Deepgram Nova-3 streaming $0.0077/min list ($0.0048 promo), multilingual $0.0092; Flux (turn-aware) $0.0077 list, EOT median <300 ms (260 ms P50), p95 1.5 s; Deepgram's own guidance: ≤300 ms transcript latency, 200-500 ms total; accepts encoding=linear16 sample_rate=16000 (device-native), interim_results=true, endpointing=300; $200 free credit. Privacy: all cloud options ship the operator's voice off-box; PTT audio is short (≤60 s) so cost is negligible (≈$0.03/h of held PTT at $0.017/min).

(3) TTS. AVSpeechSynthesizer via objc2-avf-audio 0.3.2 (macOS 10.14+): speakUtterance plays to the SYSTEM default output (no per-device routing on macOS; outputChannels needs AVAudioSessionRoute/iOS), so use writeUtterance_toBufferCallback [features AVAudioBuffer, block2] to get PCM (typically 1 ch 22050 Hz per voice; a zero-frameLength buffer ends the utterance; forum threads 122690/684419 warn format varies per voice → convert per buffer) and play it through the daemon's own output unit. Per-agent voices: AVSpeechSynthesisVoice identifiers (185 voices installed per `say -v ?`; premium/enhanced voices are downloaded in System Settings; Siri voices are NOT exposed to AVSpeechSynthesizer). `say` CLI: `-a "Codex Micro"` targets a device by name/ID without changing the default output; `say -v Samantha --data-format=LEI16@16000 --file-format=WAVE -o x.wav` renders 16 kHz Int16 mono PCM; measured 0.85 s wall (incl. process start) for a 2.7 s utterance. Cloud TTS: OpenAI gpt-4o-mini-tts $0.60/1M text in + $12/1M audio out (≈$0.015/min), tts-1 $15/1M chars; ElevenLabs Flash v2.5 75 ms model time / ≈135 ms TTFB, ≈$0.05/1k chars; Deepgram Aura-2 $0.030/1k chars. Local neural: Kokoro 82M via FluidAudio (CoreML).

(4) ROUTING + MIXER. Pin an output HAL unit to UID "...:output" (cpal picks a non-default Device → sets kAudioOutputUnitProperty_CurrentDevice; or coreaudio-rs audio_unit_from_device_id). System default output untouched. Mixer: write a small f32 mixer in the daemon (3 priority lanes question > ready > chatter, plus chime lane; higher lane pre-empts with ~100-150 ms fade; lower lanes queue with TTL; volume from the device's `set speaker` 0..10). Barge-in: `ptt act:1` (one ≤60-byte report) → cut all lanes, drop output stream, open input; `ptt act:0` → close input, wait for nominal rate listener to report 44100, reopen output, flush stale chatter (keep unanswered question prompts). `ptt ctx:codex` is only a stop-speaking signal (agent-command-deck.md:226-228). Pre-render chimes as PCM at 44.1 k (and 16 k in case anything must play during SCO). rodio 0.22.2 (DeviceSinkBuilder::from_device + mixer::Mixer) and kira 0.12.4 are alternatives, but rodio's docs cite cpal 0.17 (likely a version mismatch with cpal 0.18.2) and neither models priority/preemption — custom is smaller.

(5) PERMISSIONS / WHICH PROCESS OWNS AUDIO. Microphone (kTCCServiceMicrophone) and Speech Recognition (kTCCServiceSpeechRecognition) are per-client TCC grants. A bare LaunchAgent binary is keyed by its executable path (naudio #105: row `/usr/local/bin/na_audio_daemon`; denial yields SILENCE, not an error); it needs an embedded __TEXT,__info_plist (embed_plist 1.2.2 or `-sectcreate`) with NSMicrophoneUsageDescription + NSSpeechRecognitionUsageDescription (Speech requestAuthorization crashes without it; embed-plist-rs #1 shows "Abort trap: 6" when the plist is not recognised) and a stable code signature; precedent: `hear` (sveinbjornt/hear) is a product-type "tool" binary with CREATE_INFOPLIST_SECTION_IN_BINARY=YES, both usage keys, Developer ID signed, macOS 13+, and its README says an abort on first run is fixed by launching via Finder "Open" (i.e. the prompt needs a responsible UI context). LaunchDaemons (root) are reported never to get mic access (forum 123314, unanswered). Apple DTS (Quinn, forum 692758, Oct 2021): an agent embedded in an app bundle at Contents/MacOS gets the parent app's name in the TCC alert, the grant goes to the app and extends to the nested agent (macOS 11.6). t3code #728: child-process prompts require a validly signed parent bundle + usage keys. Cloud STT/TTS need no TCC beyond Microphone. Therefore: the daemon should own all audio, ship INSIDE the gpui app bundle (Contents/MacOS/<daemon>, registered with SMAppService, macOS 13+) so one prompt in the GUI covers it; for headless/dev installs, the bare binary works with embedded plist + Developer ID signing (ad-hoc signing likely re-prompts on every rebuild — unverified).

V1 RECOMMENDATION: daemon owns capture, STT, TTS, mixer (GUI is a pure client). Capture: cpal 0.18.2 by UID, native 16 k/1 ch/f32, stream lifetime = PTT hold. STT: Apple SFSpeechRecognizer on-device (objc2-speech 0.3.2), en-US, partials → `transcript` (send last ~120 bytes, `final:1` on release); cloud fallback plugin (gpt-live-transcribe or Deepgram Nova-3 at 16 k linear16) selectable per user; keep engine behind a trait so Parakeet (fluidaudio-rs) or a SpeechTranscriber Swift helper can replace it in v2 (better accuracy, zh-CN). TTS: AVSpeechSynthesizer writeUtterance_toBufferCallback → rubato → custom mixer → HAL unit pinned to the Codex Micro output; one AVSpeechSynthesisVoice per agent slot (4 distinct en-US enhanced voices), `say -a`/`-o` as the dev fallback. Package the daemon inside the app bundle (SMAppService) for TCC; embed Info.plist in the bare binary for CLI installs.

## Facts
- [verified] Dev Mac: sw_vers ProductVersion 27.0, BuildVersion 26A428; Darwin 27.0.0; cargo/rustc 1.92.0
  - source: local `sw_vers`, `cargo --version`, 2026-09-18
- [verified] Codex Micro is two CoreAudio devices: input id=101 uid "C2-2D-81-AB-88-81:input" transport 'blue' 1ch nominal 16000 Hz; output id=95 uid "C2-2D-81-AB-88-81:output" 2ch nominal 44100 Hz; Bluetooth services HFP AVRCP A2DP HID BLE, VID 0x303A PID 0x8360
  - source: local CoreAudio lister (scratchpad/list_audio.c) + `system_profiler SPAudioDataType/SPBluetoothDataType`, 2026-09-18
- [verified] HFP negotiates CVSD 8 kHz or mSBC 16 kHz; macOS opens the voice (SCO) link when an application uses the input; opening the microphone switches the Codex Micro output to the HFP link (8/16 kHz) and music returns to A2DP when microphone use ends
  - source: /Users/tony/projects/conol/openmicrodial/native/README.md:9-18; codec/rate logic native/src/bluetooth.c:429-433
- [verified] Hardware validation 2026-09-09: Mac exposed 44.1 kHz 2-ch output; simultaneous playback+capture switched to 16 kHz HFP; 5 s capture got 80,640 samples; Mac returned to 44.1 kHz after capture; echo quality not evaluated
  - source: native/README.md:243-257
- [verified] tools/test-speaker.c plays to Codex Micro via AudioQueue kAudioQueueProperty_CurrentDevice by device UID without changing the Mac's default output or volume
  - source: /Users/tony/projects/conol/openmicrodial/tools/test-speaker.c:38-71 and native/README.md:235-238
- [verified] Deck protocol: host→device `transcript {agent, text (200, tail), final?}` for live STT; device→host `ptt {act, agent, ctx talk/answer/name/codex, card?, abort?}`; `set` speaker/mic/announce/chime are host-side values
  - source: docs/deck-protocol.md:117,124,134,140; docs/agent-command-deck.md:280-282
- [verified] PTT session auto-releases after 60 s; CANCEL aborts (abort:1); on Codex face `ptt ctx:codex` is used by the host only to stop speaking
  - source: docs/agent-command-deck.md:218-228
- [verified] Spec: host-side mixer priority queue questions > ready > chatter, PTT preempts everything; barge-in cuts TTS; question chime two-tone rising, ready chime single knock; auto-announce on by default; per-agent TTS voices recommended
  - source: scratchpad/spec.md:110-117,135
- [verified] Current host code is a Codex icon-sync loop over hidapi 2.6.6 with no audio code
  - source: /Users/tony/projects/conol/openmicrodial/host/src/main.rs:1-218, host/Cargo.toml:8-11
- [verified] crates.io versions: cpal 0.18.2; coreaudio-rs 0.14.2; objc2-speech 0.3.2; objc2-avf-audio 0.3.2; objc2-av-foundation 0.3.2; objc2-core-audio 0.3.2; objc2-audio-toolbox 0.3.2; whisper-rs 0.16.0; whisper-rs-sys 0.15.0; rodio 0.22.2; kira 0.12.4; sherpa-onnx 1.13.8; parakeet-rs 0.3.7; fluidaudio-rs 0.14.1; voice_activity_detector 0.2.1; webrtc-vad 0.4.0; rubato 5.0.0; hound 3.5.1; tokio 1.53.1; tokio-tungstenite 0.30.0; async-openai 0.42.0; ort 2.0.0-rc.13; candle-core 0.11.0; embed_plist 1.2.2
  - source: `cargo search` against crates.io, 2026-09-18
- [verified] objc2 0.6.4 / objc2-foundation 0.3.2 / objc2-app-kit 0.3.2 / block2 0.6.2 are already in the local cargo cache (compatible with objc2-speech/objc2-avf-audio 0.3.2 which require objc2 >=0.6.2,<0.8)
  - source: ~/.cargo/registry/cache listing; docs.rs/objc2-speech/0.3.2
- [verified] cpal macOS backend: audio_unit_from_device sets kAudioOutputUnitProperty_CurrentDevice on the HALOutput unit; set_sample_rate changes kAudioDevicePropertyNominalSampleRate and waits (default 1 s timeout); DisconnectManager listens DeviceIsAlive→DeviceNotAvailable, NominalSampleRate change→StreamInvalidated, ProcessorOverload→xrun; does not set kAudioOutputUnitProperty_EnableIO explicitly
  - source: https://raw.githubusercontent.com/RustAudio/cpal/master/src/host/coreaudio/macos/{device.rs,mod.rs} fetched 2026-09-18
- [verified] coreaudio-rs 0.14.2 audio_unit module: AudioUnit::new/new_with_flags, IOType::{HalOutput, DefaultOutput, VoiceProcessingIO}, set_property/get_property, set_render_callback/set_input_callback, set_sample_rate; macos_helpers: get_default_device_id, audio_unit_from_device_id, get_device_name, find_matching_physical_format, set_device_sample_rate, get_supported_physical_stream_formats
  - source: https://docs.rs/coreaudio-rs/latest/coreaudio/audio_unit/index.html
- [verified] objc2-avf-audio 0.3.2 binds AVAudioEngine, AVAudioInputNode, AVAudioPCMBuffer, AVAudioFormat, AVAudioNode::installTapOnBus_bufferSize_format_block (features AVAudioBuffer, AVAudioFormat, AVAudioTime, AVAudioTypes, block2), AVSpeechSynthesizer::{speakUtterance, stopSpeakingAtBoundary, writeUtterance_toBufferCallback (features AVAudioBuffer, block2), outputChannels (feature AVAudioSessionRoute), setDelegate, usesApplicationAudioSession, mixToTelephonyUplink}, AVSpeechUtterance, AVSpeechSynthesisVoice
  - source: https://docs.rs/objc2-avf-audio/latest/objc2_avf_audio/ (struct.AVSpeechSynthesizer.html, struct.AVAudioNode.html)
- [verified] objc2-speech 0.3.2 binds SFSpeechRecognizer {initWithLocale, isAvailable, supportedLocales, supportsOnDeviceRecognition, requestAuthorization [block2], recognitionTaskWithRequest_resultHandler [block2+SFSpeechRecognitionRequest/Result/Task], recognitionTaskWithRequest_delegate, setQueue}, SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionTask; SpeechAnalyzer/SpeechTranscriber are not bound
  - source: https://docs.rs/objc2-speech/latest/objc2_speech/struct.SFSpeechRecognizer.html
- [verified] Apple: SFSpeechRecognizer macOS 10.15+, not deprecated; framework stops (server) tasks longer than one minute; per-device and per-app daily limits for the network service; supportsOnDeviceRecognition = can operate without network
  - source: https://developer.apple.com/tutorials/data/documentation/speech/sfspeechrecognizer.json
- [verified] Apple: requiresOnDeviceRecognition is honoured only if supportsOnDeviceRecognition is true; on-device requests 'won't be as accurate'
  - source: https://developer.apple.com/tutorials/data/documentation/speech/sfspeechrecognitionrequest/requiresondevicerecognition.json
- [verified] Apple: requestAuthorization prompts with the NSSpeechRecognitionUsageDescription text; 'If this key is not present, your app will crash when you call this method'
  - source: https://developer.apple.com/tutorials/data/documentation/speech/sfspeechrecognizer/requestauthorization(_:).json
- [verified] On this Mac: SFSpeechRecognizer en-US available=true supportsOnDeviceRecognition=true; zh-CN and en-GB on-device=false ('No Assistant asset for language'); 62 supported locales; SpeechTranscriber supportedLocales=45, installedLocales = en_NZ,en_AU,en_US,en_CA,en_IN,en_ZA,en_SG,en_IE,en_GB
  - source: local Swift probe scratchpad/speechcheck.swift run 2026-09-18 15:40
- [verified] SpeechAnalyzer (SpeechTranscriber, DictationTranscriber, AssetInventory, volatileRange) is macOS 26.0+ and Swift-only (interfaceLanguage swift, actor API, results via AsyncSequence)
  - source: https://developer.apple.com/tutorials/data/documentation/speech/speechanalyzer.json
- [likely] On-device SFSpeechRecognizer needs the dictation language asset downloaded; otherwise kAFAssistantErrorDomain 1101; `hear` notes only Siri-supported locales work with -d
  - source: Apple forums 731230/715516 (via search summary); https://github.com/sveinbjornt/hear README
- [likely] AVSpeechSynthesizer is macOS 10.14+ incl. write(_:toBufferCallback:), outputChannels, usesApplicationAudioSession, mixToTelephonyUplink; buffers from write are commonly 1 ch 22050 Hz and a zero-frameLength buffer marks the end; format may vary per voice
  - source: https://developer.apple.com/tutorials/data/documentation/avfaudio/avspeechsynthesizer.json; Apple forums 122690, 684419, 729218 (search summary)
- [likely] Siri voices are not exposed through AVSpeechSynthesizer (fallback voice substituted); premium/enhanced voices must be downloaded in System Settings (AVSpeechSynthesisVoiceQuality.premium)
  - source: Apple forum 682438; bendodson.com 2024-04-03 (search summary)
- [verified] `say` supports `-a ID|name --audio-device`, `-v voice`, `-o file`, `--data-format=LEI16@16000`, `--file-format=WAVE`; local `say -a ?` lists Codex Micro as ID 95; 185 voices installed; synthesizing a 2.7 s utterance to file took 0.85 s wall (2 runs); default output 1ch 22050 Hz 16-bit BE AIFF
  - source: `man say`, local runs 2026-09-18 (scratchpad/tts_test.aiff, tts_test16k.wav)
- [verified] whisper-rs 0.16.0 published 2026-03-12 (repo now https://codeberg.org/tazz4843/whisper-rs; GitHub mirror archived 2025-07-30); whisper-rs-sys 0.15; features raw-api, coreml, cuda, hipblas, openblas, metal, vulkan, openmp, intel-sycl, log_backend, tracing_backend; API WhisperContext/WhisperState/FullParams/SamplingStrategy, WhisperVadContext (Silero VAD); license Unlicense
  - source: https://crates.io/api/v1/crates/whisper-rs; codeberg raw Cargo.toml; docs.rs/whisper-rs 0.16.0
- [verified] whisper.cpp: Metal runs inference fully on GPU on Apple Silicon; Core ML encoder on ANE >3x faster than CPU but first run compiles the model (slow), macOS 14+ recommended; VAD via silero-v6.2.0 with --vad-threshold/--vad-min-speech-duration-ms/--vad-speech-pad-ms; RAM tiny ~273 MB, base ~388 MB, small ~852 MB, medium ~2.1 GB, large ~3.9 GB; stream example uses `--step 500 --length 5000` or VAD mode `--step 0 --length 30000 -vth 0.6` (sliding window, not true streaming)
  - source: https://github.com/ggml-org/whisper.cpp README and examples/stream/README.md
- [likely] mac-whisper-speedtest on MacBook Pro M4 24 GB, model 'large': fluidaudio-coreml 0.19 s, parakeet-mlx 0.50 s, mlx-whisper 1.02 s, whisper.cpp 1.23 s, whisperkit 2.22 s, faster-whisper 6.96 s per clip (clip length unstated)
  - source: https://github.com/anvanvan/mac-whisper-speedtest README
- [uncertain] large-v3-turbo on M2 Pro with Metal+flash attention transcribes 60 s of audio in ~2.8 s; ~9x RT on M3 Air; ~14-18x RT on M5
  - source: search summaries of fazm.ai / justvoice.ai / promptquorum.com blogs (secondary)
- [verified] fluidaudio-rs 0.14.1 (2026-05-11): C FFI bridge to FluidAudio Swift package compiled by build.rs; requires macOS 14+, Swift 5.10+, Rust 1.70+; streaming API init_streaming_asr/streaming_asr_start/feed/finish; VAD; first init downloads ~500 MB and takes 20-30 s ANE compile; FluidAudio runs Parakeet TDT 0.6B v2/v3 on CoreML, ~190x RT on M4 Pro, EOU 120M streaming (English), Kokoro TTS, Silero VAD; Apache-2.0/MIT
  - source: https://docs.rs/fluidaudio-rs/latest; https://github.com/FluidInference/FluidAudio
- [verified] parakeet-rs 0.3.x: Parakeet TDT (25 languages), EOU streaming 160 ms chunks, Nemotron 560 ms chunks; 'CoreML is unstable with this model' on Apple, use CPU/WebGPU EP; MIT OR Apache-2.0
  - source: https://github.com/altunenes/parakeet-rs
- [verified] Kyutai STT: stt-1b-en_fr 0.5 s delay with semantic VAD; stt-2.6b-en 2.5 s delay; Rust stt-rs with candle 0.9.1 and a `metal` feature; weights CC-BY-4.0, Rust code Apache
  - source: https://github.com/kyutai-labs/delayed-streams-modeling README and stt-rs/Cargo.toml
- [verified] OpenAI pricing: gpt-live-transcribe and gpt-realtime-whisper $0.017/min; gpt-4o-transcribe $2.50/$10 per 1M tokens (~$0.006/min); gpt-4o-mini-transcribe $1.25/$5 (~$0.003/min); gpt-transcribe $0.0045/min; whisper $0.006/min; gpt-4o-mini-tts $0.60/1M text in + $12/1M audio out; tts-1 $15/1M chars, tts-1-hd $30/1M chars; gpt-realtime-2.1 audio $32/$64 per 1M tokens, -mini $10/$20
  - source: https://developers.openai.com/api/docs/pricing fetched 2026-09-18
- [likely] OpenAI Realtime transcription: session type "transcription" with model gpt-live-transcribe over WebSocket (wss://api.openai.com/v1/realtime?intent=transcription), input audio/pcm rate 24000 via input_audio_buffer.append (base64 pcm16), partials `conversation.item.input_audio_transcription.delta`, finals `.completed`
  - source: https://developers.openai.com/api/docs/guides/realtime-transcription; URL form from community examples (search summary)
- [verified] Deepgram pricing (PAYG): Nova-3 mono streaming $0.0077/min list ($0.0048 promo), multilingual $0.0092 ($0.0058 promo), Nova-3 batch $0.0043/min, Flux English streaming $0.0077 list ($0.0065 promo), Aura-2 TTS $0.030/1k chars, $200 free credit
  - source: https://deepgram.com/pricing fetched 2026-09-18
- [verified] Deepgram streaming: interim_results=true gives is_final:false partials; endpointing (ms of silence, e.g. 300) finalizes; accepts encoding=linear16 channels=1 sample_rate=16000; Deepgram's guidance: ≤300 ms transcription latency, 200-500 ms total; Flux EOT 100-500 ms, median <300 ms (260 ms P50), p95 1.5 s
  - source: https://developers.deepgram.com/docs/interim-results; https://developers.deepgram.com/docs/measuring-streaming-latency; Deepgram Flux blog (search)
- [likely] ElevenLabs Flash v2.5: 75 ms model inference, ~135 ms end-to-end TTFB, ~$0.05 per 1k characters
  - source: elevenlabs.io docs/blog via search summary
- [likely] rodio 0.22.2: DeviceSinkBuilder::from_device(cpal::Device)/with_device, with_sample_rate/with_channels/with_buffer_size, open_stream; mixer::Mixer for parallel playback; docs indicate a cpal 0.17 dependency (potential mismatch with cpal 0.18.2)
  - source: https://docs.rs/rodio/latest/rodio/ and stream/struct.DeviceSinkBuilder.html
- [likely] AVAudioEngine on macOS: inputNode uses the system default input unless kAudioOutputUnitProperty_CurrentDevice is set on inputNode.audioUnit (or AUAudioUnit.deviceID)
  - source: Apple forums 71008, 133080, 775015; AudioKit issue 2130 (search summary)
- [likely] macOS community reports: while a Bluetooth headset mic is in use the device shows '1ch 8kHz/16kHz' (SCO) in Audio MIDI Setup; macOS sometimes fails to switch back to A2DP (Tahoe 26.1 thread, Nov 2025); fix is reconnect / `sudo killall bluetoothd`
  - source: https://discussions.apple.com/thread/256182176; gist dvf/3771e58085568559c429d05ccc339219
- [verified] macOS 26.1 fixed several CoreAudio capture bugs incl. captures failing when secondary output devices had a different sample rate than the default output, and low-sample-rate capture regressions
  - source: https://weblog.rogueamoeba.com/2025/11/04/macos-26-tahoe-includes-important-audio-related-bug-fixes/
- [verified] TCC attributes a bare LaunchAgent binary by executable path (row `kTCCServiceMicrophone | /usr/local/bin/na_audio_daemon`), and a denied client captures silence with no error; the Privacy pane shows the binary name
  - source: https://github.com/KJ5HST-LABS/naudio/issues/105
- [verified] Apple DTS (Quinn, Oct 2021): a launchd agent embedded in an app bundle at Contents/MacOS gets the parent app's name in the TCC alert, the grant is made to the app and extends to the nested agent (tested on macOS 11.6); do not put it in Contents/Resources
  - source: https://developer.apple.com/forums/thread/692758
- [likely] A LaunchDaemon (root, /Library/LaunchDaemons) always saw AVAuthorizationStatusDenied for audio on Mojave/Catalina; thread unanswered
  - source: https://developer.apple.com/forums/thread/123314 (2019)
- [likely] TCC prompts for child processes require a valid code signature on the parent bundle plus usage-description keys in its Info.plist
  - source: https://github.com/pingdotgg/t3code/issues/728
- [verified] `hear` (sveinbjornt/hear) is a bare command-line tool (productType tool) using SFSpeechRecognizer with CREATE_INFOPLIST_SECTION_IN_BINARY=YES, Info.plist containing NSMicrophoneUsageDescription and NSSpeechRecognitionUsageDescription, Developer ID signed and notarized, macOS 13+; README: an abort on launch is fixed by opening via Finder; server recognition has a ~500-character hard limit
  - source: https://github.com/sveinbjornt/hear (README, hear.xcodeproj/project.pbxproj, assets/Info.plist)
- [verified] embed_plist 1.2.2 embeds Info.plist into __TEXT,__info_plist (what Xcode's -sectcreate does); its issue #1 (2021) shows 'Abort trap: 6' when the usage description is not recognised, unresolved in the issue text
  - source: https://github.com/nvzqz/embed-plist-rs and issues/1
- [verified] ChatGPT.app and Claude.app are installed in /Applications; whisper-cli is not installed
  - source: local `ls /Applications`, `which whisper-cli`

## Options
### Capture A: cpal 0.18.2 pinned to the Codex Micro input UID, stream lifetime = PTT hold
- pros: Already CoreAudio-correct (sets kAudioOutputUnitProperty_CurrentDevice, alive/sample-rate listeners); pure Rust; f32 16 kHz mono callback feeds any STT engine; minimal code
- cons: Must request the device's own nominal config or cpal tries to change the nominal rate (Bluetooth cannot); no access to kAudioDevicePropertyDeviceIsRunningSomewhere without objc2-core-audio; device name matching must pick the input-capable 'Codex Micro' (two devices share the name) — use UID
- recommendation: v1 choice. Match by UID suffix ':input' via objc2-core-audio, open on `ptt act:1`, drop on release/abort/up/60 s.
### Capture B: coreaudio-rs 0.14.2 AUHAL directly (IOType::HalOutput + set_input_callback)
- pros: Full control (EnableIO, DeviceIsRunningSomewhere to detect the Codex app holding the mic, property listeners for SCO rate flip); one crate for both input and pinned output
- cons: More boilerplate and unsafe surface than cpal; you re-implement disconnect/rate-change handling
- recommendation: Use only if cpal's abstractions block a needed property; otherwise keep for the output unit if a custom mixer wants raw render callbacks.
### Capture C: AVAudioEngine (objc2-avf-audio) with inputNode.audioUnit re-targeted
- pros: Yields AVAudioPCMBuffer directly for SFSpeechRecognizer; Apple-blessed audio graph
- cons: On macOS inputNode follows the default input unless you reach into CoreAudio anyway; engine reconfigures on route changes; more ObjC lifetime handling
- recommendation: Not needed; wrap cpal's f32 frames into AVAudioPCMBuffer for Speech instead.
### STT A: Apple SFSpeechRecognizer on-device (objc2-speech 0.3.2)
- pros: Free, private, en-US on-device verified on this Mac, partial results for the live-transcript view, no models to ship; 60 s PTT cap matches Apple's 1-minute limit
- cons: Accuracy lower than cloud/Parakeet for code/CLI vocabulary; zh-CN on-device unavailable here; needs NSSpeechRecognitionUsageDescription in the (embedded) Info.plist or requestAuthorization crashes; TCC Speech Recognition grant required
- recommendation: v1 default engine.
### STT B: SpeechAnalyzer/SpeechTranscriber (macOS 26+) via a Swift @_cdecl helper
- pros: Apple's newest on-device model, 45 locales (9 English installed), volatile (partial) results, no network
- cons: Swift-only API → requires a Swift static lib or XPC helper in the build; macOS 26+ only; untested from Rust
- recommendation: v2 upgrade path if SFSpeechRecognizer accuracy is insufficient and the build can include Swift.
### STT C: NVIDIA Parakeet TDT 0.6B v3 via fluidaudio-rs 0.14.1 (CoreML/ANE) or parakeet-rs 0.3.7 (ONNX)
- pros: Fastest local engine measured (0.19 s vs whisper.cpp 1.23 s on M4), 25 languages, streaming + EOU model, permissive licenses
- cons: fluidaudio-rs compiles a Swift package in build.rs (macOS 14+, Swift 5.10+), ~500 MB model download and 20-30 s first ANE compile; parakeet-rs CoreML EP unstable (CPU/WebGPU); more binary size/complexity
- recommendation: v2 candidate for multilingual/accuracy; benchmark against Speech on the target vocabulary first.
### STT D: whisper.cpp via whisper-rs 0.16.0 (metal/coreml)
- pros: Mature, best-known accuracy per size, built-in Silero VAD, pure C++ deps
- cons: Not streaming — partials are sliding-window re-decodes (500 ms step) that flicker; large-v3-turbo needs ~1-6 GB RAM and ~1-3 s per window on M-series; slower than Parakeet on Apple Silicon
- recommendation: Not for v1 live transcript; acceptable as an offline 'final pass' if desired.
### STT E: Cloud streaming (OpenAI gpt-live-transcribe $0.017/min; Deepgram Nova-3 $0.0077/min list, Flux for turn-end)
- pros: Best accuracy, sub-300 ms partials (Deepgram guidance), Deepgram accepts 16 kHz linear16 natively; negligible cost at PTT volumes
- cons: Voice leaves the machine; API key management; network dependency; OpenAI wants 24 kHz PCM (resample with rubato)
- recommendation: Ship as an optional engine behind the same trait; default off.
### TTS A: AVSpeechSynthesizer writeUtterance_toBufferCallback → daemon mixer → HAL unit pinned to Codex Micro output
- pros: Free, offline, per-agent AVSpeechSynthesisVoice (185 voices installed, premium voices downloadable), buffers let the mixer do priority/ducking/barge-in and route only to the device without touching the Mac default output
- cons: Buffer format varies per voice (commonly 22050 Hz mono) → per-utterance conversion; Siri voices unavailable; some forum reports of glitchy buffers on older OS versions
- recommendation: v1 choice.
### TTS B: `say -a "Codex Micro"` / `say -o` subprocess
- pros: Zero code, device targeting by name, 0.85 s to render a 2.7 s line to 16 kHz WAV
- cons: Process spawn per utterance, no mixing/priority/ducking when playing directly, output goes straight to the device bypassing the mixer
- recommendation: Dev fallback and CLI diagnostics only.
### TTS C: Cloud (OpenAI gpt-4o-mini-tts ~$0.015/min, ElevenLabs Flash v2.5 ~135 ms TTFB, Deepgram Aura-2 $0.03/1k chars) or local Kokoro via FluidAudio
- pros: Much more natural voices, distinct per-agent personas
- cons: Network latency and cost for every announcement; privacy of agent output; extra streaming decode path
- recommendation: Optional plugin; not v1.
### Mixer A: custom f32 priority mixer in the daemon (question > ready > chatter + chime lane), output via cpal/coreaudio-rs unit pinned by UID
- pros: Exact spec semantics (priority preemption, TTL, ducking, PTT cut in one callback), handles the SCO-induced StreamInvalidated by re-opening the output after PTT; small code
- cons: You own resampling (rubato) and fades
- recommendation: v1 choice.
### Mixer B: rodio 0.22.2 DeviceSinkBuilder::from_device + mixer::Mixer (or kira 0.12.4)
- pros: Ready-made mixing, sinks, volume
- cons: No priority/preemption model; rodio appears to pin cpal 0.17 (version mismatch with cpal 0.18.2 device handles); still must handle stream invalidation
- recommendation: Only if the custom mixer grows beyond ~300 lines.
### Process/permissions A: daemon inside the gpui app bundle (Contents/MacOS), registered with SMAppService (macOS 13+); daemon owns capture/STT/TTS
- pros: Per Apple DTS, TCC alert shows the app name and the grant extends to the nested agent; one prompt for Microphone + Speech Recognition; GUI stays a pure client; headless operation after first grant
- cons: Requires the app bundle to exist and be signed; agent binary must live in Contents/MacOS; bundle-id changes invalidate grants
- recommendation: v1 shipping layout.
### Process/permissions B: bare LaunchAgent binary with embedded __info_plist (embed_plist 1.2.2) and Developer ID signature
- pros: Works without a GUI (precedent: `hear`), simple Homebrew/cargo install
- cons: TCC row keyed by path + signature (ad-hoc rebuilds likely re-prompt); denial yields silence; first prompt may need a UI launch (hear README); unverified on macOS 26/27
- recommendation: Dev/CLI mode only; document `codesign` step.
### Process/permissions C: GUI app owns audio, daemon delegates to it
- pros: Simplest TCC story
- cons: Breaks 'GUI is just a client' and headless use; audio dies when the GUI quits
- recommendation: Reject.

## Open questions
- Measure on macOS 27: latency from starting input IO on the HFP device to first mic samples (SCO open), and from stopping IO to the output device's nominal rate returning to 44100 Hz (A2DP resume) — these bound PTT start delay and post-PTT announcement delay.
- Does the link ever negotiate CVSD (8 kHz) instead of mSBC with this Mac? Only 16 kHz was observed on 2026-09-09; the STT path should still handle an 8 kHz nominal rate.
- Actual partial-result latency and accuracy of SFSpeechRecognizer on-device vs Parakeet v3 vs gpt-live-transcribe on agent/CLI vocabulary (agent names, file paths); consider SFSpeechLanguageModel custom vocabulary (macOS 14+) for agent names — not verified here.
- Does an ad-hoc-signed bare binary with an embedded Info.plist get a Microphone/Speech TCC prompt when launched by launchd on macOS 26/27, or only via Finder/`open`? (hear is Developer ID signed.)
- Exact cpal version rodio 0.22.2 depends on (docs.rs summary said 0.17) — determines whether rodio can share cpal 0.18.2 Device handles.
- Echo/feedback: while PTT is held the device speaker may still play via SCO; verify HFP-side echo cancellation on the SiFli SDK or mute the output lane during capture (native/README.md:256-257 says echo quality is unevaluated).
- Whether to build a Swift helper (SpeechTranscriber / fluidaudio-rs) into the daemon build — decide once the v1 SFSpeechRecognizer accuracy is measured.
- Codex app coexistence: if the Codex desktop app also opens the Codex Micro input on its own PTT (channel 2), both IO procs share the SCO link; confirm CoreAudio allows concurrent input clients and which process's start/stop toggles A2DP.
