# USB sample-rate experiment

USB speaker playback accepts 16 or 48 kHz mono IMA ADPCM on channel 7.
The optional `audio-usb-rate-test` feature adds read-only integrity counters. The START rate byte selects 16 or 48;
codec 1, the 20 ms frame declaration and AUDIO packet format are unchanged.
USB packets still carry at most 110 samples. At 48 kHz this is about 436
audio reports/s (28 KB/s including HID framing), versus 145 reports/s at
16 kHz. The microphone and BLE remain at 16 kHz. The daemon selects 48 kHz for
USB firmware advertising INFO feature bit 2, falling back to 16 kHz otherwise.

The DAC stays at 48 kHz. For a 16 kHz stream a band-limited 3x FIR
interpolator runs; a 48 kHz stream passes decoded samples directly to the DAC.
High-rate playback requires an explicit USB START. BLE and unrelated stream
IDs cannot feed an active high-rate stream. Finishing or timing out restores
the default rate.

The diagnostic counters are opt-in; USB 48 kHz support is standard. Build it separately
so the ordinary image remains available for rollback:

```sh
cargo build --release --locked --offline --features hw-a,audio-usb-rate-test \
  --target-dir target/usb-audio-rate-test
python3 tools/check-xip-image.py \
  target/usb-audio-rate-test/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
python3 tools/flash-usb.py --serial SERIAL \
  target/usb-audio-rate-test/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
```

Use `hw-b` for Rev B. `sys.version` reports `audio_test: true` and
`usb_speaker_rates: [16000, 48000]`. The read-only `audio.test.status` RPC
returns cumulative stream, decoded-frame/sample, gap, underrun, queue-drop
and sequence-error counts, plus the last sample rate and whether playback
is active. No register-write or microphone diagnostic feature is enabled.

## Prepare a controlled comparison

Start with actual 48 kHz source audio. Upsampling an existing 16 kHz recording
cannot test the lost high frequencies. Apply one common gain and the speaker
EQ to the 48 kHz master, then derive the 16 kHz version with an anti-alias
filter. Keep both versions at the same level; do not normalize them separately.
Use the same passage, fades and duration, and keep the speaker volume fixed.

`tools/usb-audio-rate.rs` uses the production EQ and codec directly:

```sh
rustc --edition=2021 -O tools/usb-audio-rate.rs -o /tmp/usb-audio-rate
/tmp/usb-audio-rate eq source48.f32 eq48.raw
/tmp/usb-audio-rate encode 16000 a16.raw a16.hid 201
/tmp/usb-audio-rate encode 48000 b48.raw b48.hid 202
```

The EQ input is interleaved stereo little-endian f32 at 48 kHz; its output is
mono little-endian i16. It applies one source peak gain to 0.5, then the
production curve at 48 kHz (250 Hz high-pass, -5 dB at 1.1 kHz, -6 dB at
6.3 kHz, -3 dB preamp). Encoding takes mono i16 at the declared rate. It
also writes `.decoded.raw` and reports the codec's offline signal-to-noise
ratio. That metric measures digital encoding error, not acoustic quality.

Derive `a16.raw` by filtering and downsampling the EQ master; `b48.raw` uses
the same master directly. Include 300 ms of leading silence and 150 ms of
trailing silence in both clips. A manifest beside the reports selects the order:

```json
{"tracks":[{"label":"A — 16 kHz","file":"a16.hid"},{"label":"B — 48 kHz","file":"b48.hid"}]}
```

```sh
python3 tools/usb-audio-rate.py comparison.json --serial SERIAL \
  --deckd /path/to/deckd --out results.json
```

The player verifies the firmware capability, USB route and idle voice state,
temporarily suspends competing daemon audio with a separate resume watchdog,
paces reports at their actual sample rate, and resumes the service in a
`finally` block. Speaker volume is preserved. A two-second gap separates clips.
The player checks every decoded sample/frame arrived, END was processed, and
the deck recorded no gaps, underruns, queue drops or sequence errors. It stops
on failed integrity checks so transport glitches are not mistaken for a
sample-rate difference.

Interpret a listening improvement as evidence about the combined sample-rate,
ADPCM and interpolation path. It does not isolate the analog amplifier or
speaker, and higher-rate ADPCM can improve encoding error even within the
shared speech band. Further codec or EQ comparisons can isolate those effects.

Restore an ordinary matching-board release image with `tools/flash-usb.py`
when the experiment is finished. The bench image has the same rate negotiation as the ordinary firmware.

## First hardware run — 2026-10-22

Rev A over USB, using a fresh 48 kHz render of the same orchestral score.
The comparison used seconds 42–54, common EQ and gain, 50 ms fades, and
the silence padding above. The 16 kHz variant used a 511-tap Kaiser
anti-alias filter with a 7.5 kHz cutoff before downsampling by three.

| Measurement | A: 16 kHz | B: 48 kHz |
| --- | ---: | ---: |
| Duration including padding | 12.450625 s | 12.450625 s |
| Source RMS | -25.58 dBFS | -25.58 dBFS |
| Offline ADPCM SNR | 22.62 dB | 33.80 dB |
| Audio frames decoded | 1,811 | 5,433 |
| Samples decoded | 199,210 | 597,630 |
| Gaps / underruns / queue drops / sequence errors | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

Both streams completed normally at speaker volume 10/10. This establishes
that the USB path sustained 48 kHz for this passage. The higher-rate codec
had less digital encoding error, but the listener reported that the two
versions sounded identical, including after a replay. Both replay streams
also passed every integrity check. This result applies to this music passage;
the reported problem is unclear words in English TTS, so speech needs its own
comparison. Energy above 8 kHz was about -30.4 dB relative to total
master energy, so this passage is a limited test of high-frequency extension.

The local preparation, WAV references, encoded reports and device results
are in `/private/tmp/deck-audio-rate/`; `comparison.json` can be replayed with
the command above. These temporary artifacts are not part of the repository.

## English TTS comparison — 2026-10-22

The running service selected Kokoro (`kokoro-multi-lang-v1_1`). Export its
installed model through the production engine, including sentence leveling:

```sh
cargo run --manifest-path host/Cargo.toml -p deckd --example tts-clarity \
  --locked --offline --target aarch64-apple-darwin --target-dir target/host -- \
  /path/to/kokoro-multi-lang-v1_1 af_maple speech.txt speech24.f32 1.0
```

This writes mono little-endian f32 and a JSON sidecar recording the actual
sample rate. The exporter requires an already installed model, rejects a
fallback voice, and does not change service settings. Speed `1.0` matches
normal Kokoro playback. This run produced 24 kHz audio, so resampling it to
48 kHz preserves the source bandwidth; it does not create a native 48 kHz
voice or new high-frequency detail.

The source was resampled to 48 kHz and duplicated to stereo before
`usb-audio-rate eq-unity`, which applies the production EQ without additional
input normalization. Both variants share that EQ master. The 16 kHz version
uses the same anti-alias filter as the music experiment. Five-millisecond
outer fades and 300/150 ms silence padding preserve the spoken words.

| Measurement | A: 16 kHz | B: 48 kHz |
| --- | ---: | ---: |
| Source RMS | -25.563 dBFS | -25.475 dBFS |
| Offline ADPCM SNR | 23.95 dB | 32.81 dB |
| Audio frames decoded | 1,601 | 4,801 |
| Samples decoded, including packet padding | 176,110 | 528,110 |
| Gaps / underruns / queue drops / sequence errors | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

Both streams completed normally, with EQ enabled and speaker volume 10/10;
a requested replay also passed every integrity check. The listener reported
that B (48 kHz) was much better and that noise audible in A went away in B.
This supports the higher-rate ADPCM/direct-DAC path for this English speech,
but does not separate codec error from interpolation or source bandwidth.
Test artifacts, the exact text and the
preparation script are in `/private/tmp/deck-audio-rate/tts/`. This controlled
test bypasses live synthesis/mixing/streaming; if both variants sound clear,
compare against normal daemon playback before ruling out that part of the
pipeline.

During synthesis, sherpa-onnx warned about an unknown lexicon token and
skipped phonemes U+025A and U+0329. Their effect on the spoken passage is
unverified; do not attribute the clarity problem to them without a source
voice comparison.

The same original 24 kHz Kokoro PCM was also saved as `laptop-source.wav`
and played through the MacBook Pro's built-in speakers, before deck-specific
EQ and ADPCM. This reference checks source intelligibility. It is not a
level-matched test of speaker hardware alone because EQ, codec and acoustic
output differ between the laptop and deck.

## Follow-up implementation

USB 48 kHz is now negotiated through INFO feature bit 2, with 16 kHz fallback
for older firmware. The capture rate and BLE default remain 16 kHz. Host EQ,
20 ms wall-clock pacing and the 300 ms hangover follow the negotiated speaker
rate. USB/BLE switches and microphone release are covered by route tests.
The firmware queue is 128 frames, covering 293 ms at the highest USB rate.

The 16 kHz host encoder now searches eight ADPCM predictor paths and retains
the greedy result whenever search would increase a frame's error. With the
same English speech, codec SNR rose from 23.95 to 25.49 dB (about 30% less
squared encoding error). Bitrate and the decoder are unchanged. The firmware
also replaces linear interpolation with a 127-tap Q15 polyphase FIR: its
3 kHz test tone retains unity gain, while 13/19 kHz images are below -72 dBc.
This addresses measured artifacts; complete audible hiss removal has not
yet been confirmed by a listener for the revised 16 kHz path.

48 kHz mono ADPCM would require 192 kbit/s over BLE, or 199.2 kbit/s including
this protocol's 6-byte headers in 320-sample frames (150 frames/s), before
ATT/L2CAP/link overhead. Existing project measurements report LE 2M, data
length 251, MTU 247 and 15 ms active intervals with no peripheral latency.
That makes 48 kHz plausible, but it is not a sustained-throughput validation
on this hardware. BLE 48 kHz remains disabled pending such a test. General
throughput depends on packet sizes, event time and implementation, not PHY
rate alone; see [Nordic's throughput measurements](https://docs.nordicsemi.com/r/bundle/sds_s132/page/sds/s1xx/ble_data_throughput/ble_data_throughput.html).
