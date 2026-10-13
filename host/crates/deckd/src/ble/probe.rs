//! `deckd ble probe` and `deckd ble scan`: the bench tools for the deck's
//! audio service, usable before anything else in deckd depends on it.
//!
//! The probe asks one of three questions, each the one a firmware engineer
//! has at that point of a bring-up:
//!
//! * **Does the microphone stream?** (no `--tone`) MIC mode 1 for N
//!   seconds, a line a second — frames, bytes, lost frames, RMS and peak —
//!   and a summary: frame rate, bit rate, loss, and the time from the MIC
//!   request to the first frame. It passes when audio arrived, under
//!   [`MAX_LOSS_PCT`] of it was lost, and the stream ended with an END.
//! * **Does the speaker take a stream?** (`--tone HZ`) START, an AUDIO frame
//!   every 20 ms paced on a monotonic clock with a [`LEAD`] of 60 ms (the
//!   deck buffers about that much before it starts its amplifier), END. It
//!   passes when every frame was taken; whether it *sounded* right is for
//!   the person at the bench to say.
//! * **Do both work at once?** (`--tone HZ --loopback`) MIC mode 2 keeps the
//!   speaker on during a capture, so the tone comes back through the deck's
//!   own microphone. Each second of it is put through a Goertzel filter at
//!   HZ and at an off-frequency neighbour; it passes when at least half of
//!   the seconds have the tone [`LOOPBACK_DB`] above the neighbour.
//!
//! Exit codes: [`EXIT_PASS`], [`EXIT_FAIL`], and [`EXIT_NO_DEVICE`] when the
//! link is not ready within the attach timeout (20 s from the CLI).
//!
//! `scan` only looks — the radio, the connected peripherals, the adverts —
//! and never connects, so it is safe next to a deck in any state, including
//! one whose bond the Mac no longer shares.
use super::codec::{self, AUDIO_HEADER, MAX_FRAME, RATE_HZ};
use super::link::{BleAudio, LinkState, SendError};
use super::signal::{fade, off_frequency, tone_dbfs, Level, Sine};
use super::{hex, Info, MicEvent};
use crossbeam_channel::RecvTimeoutError;
use std::collections::BTreeSet;
use std::io::Write;
use std::time::{Duration, Instant};

pub const EXIT_PASS: i32 = 0;
pub const EXIT_FAIL: i32 = 1;
pub const EXIT_NO_DEVICE: i32 = 2;

/// How long the CLI waits for the link to be ready.
pub const ATTACH_TIMEOUT: Duration = Duration::from_secs(20);
/// How far ahead of real time the speaker stream runs.
pub const LEAD: Duration = Duration::from_millis(60);
/// The tone's peak level: loud enough to hear and to come back through the
/// microphone, well clear of the amplifier clipping the r6 boards have at
/// full scale.
pub const TONE_DBFS: f64 = -20.0;
/// Fade the tone in and out over 10 ms, so it starts and stops without a
/// (broadband) click.
pub const TONE_RAMP: usize = RATE_HZ as usize / 100;
/// A loopback second passes when the tone is this far above its neighbour.
pub const LOOPBACK_DB: f64 = 15.0;
/// The microphone test fails at this much loss.
pub const MAX_LOSS_PCT: f64 = 5.0;
/// After the last frame should have arrived, how long to wait for END.
pub const GRACE: Duration = Duration::from_secs(3);

/// What `deckd ble probe` was asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Capture or play for this long (1 to 60).
    pub seconds: u8,
    /// Play a sine at this frequency instead of only capturing.
    pub tone: Option<u32>,
    /// With `tone`: capture at the same time and look for the tone.
    pub loopback: bool,
    /// With `tone`: stream digital silence instead, for listening to the
    /// speaker path's own noise floor. With `loopback` as well, the capture
    /// is that noise floor as the deck's own microphone hears it.
    pub silence: bool,
    /// The tone's peak level in dBFS ([`TONE_DBFS`] unless said).
    pub level: f64,
    /// Save what the microphone captured (16 kHz mono, 16-bit) here.
    pub wav: Option<std::path::PathBuf>,
    /// The deck's GAP name (normally [`super::DECK_NAME`]).
    pub name: String,
    pub attach_timeout: Duration,
}

macro_rules! say {
    ($out:expr, $($arg:tt)*) => {{
        let _ = writeln!($out, $($arg)*);
    }};
}

/// The probe, on a link someone else made (the CLI's
/// [`BleAudio::spawn_named`], or a test's fake). Returns the exit code.
pub fn run(ble: &BleAudio, opts: &Options, out: &mut dyn Write) -> i32 {
    let Some(max_write) = wait_ready(ble, opts.attach_timeout, out) else {
        return EXIT_NO_DEVICE;
    };
    let seconds = opts.seconds.clamp(1, 60);
    let wav = opts.wav.as_deref();
    let passed = match (opts.tone, opts.loopback) {
        (None, _) => mic_test(ble, seconds, wav, out),
        (Some(hz), false) => tone_test(ble, seconds, hz, opts, max_write, out),
        (Some(hz), true) => loopback_test(ble, seconds, hz, opts, max_write, out),
    };
    if passed {
        EXIT_PASS
    } else {
        EXIT_FAIL
    }
}

/// `deckd ble probe`: spawn the link and run the probe on stdout.
///
/// The bench link scans ([`super::Reach::Advertising`]): it is run on
/// purpose, by somebody looking at the deck. The daemon's link does not.
pub fn main(opts: &Options) -> i32 {
    let ble = BleAudio::spawn_named(&opts.name);
    let mut out = std::io::stdout().lock();
    run(&ble, opts, &mut out)
}

/// Wait for `Ready`, saying what happens on the way. Returns the write limit.
fn wait_ready(ble: &BleAudio, timeout: Duration, out: &mut dyn Write) -> Option<usize> {
    say!(
        out,
        "looking for the deck's audio service (up to {} s)",
        timeout.as_secs_f64()
    );
    let events = ble.link_events();
    let deadline = Instant::now() + timeout;
    let mut said = BTreeSet::new();
    let mut state = ble.state();
    loop {
        let line = match &state {
            LinkState::Ready { max_write, info } => {
                say!(out, "  ready: {}", describe_ready(*max_write, *info));
                if let Some(warning) = info.and_then(incompatible) {
                    say!(out, "  warning: {warning}");
                }
                return Some(*max_write);
            }
            LinkState::Attaching => match ble.peer() {
                Some(peer) => format!("  attaching to {peer}"),
                None => "  attaching".to_string(),
            },
            LinkState::Detached => match ble.problem() {
                Some(why) => format!("  not ready: {why}"),
                None => String::new(),
            },
        };
        if !line.is_empty() && said.insert(line.clone()) {
            say!(out, "{line}");
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        // The reason can change while the state does not (a retry that
        // fails differently), so look again now and then.
        match events.recv_timeout(left.min(Duration::from_millis(250))) {
            Ok(next) => state = next,
            Err(RecvTimeoutError::Timeout) => state = ble.state(),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let why = ble
        .problem()
        .unwrap_or_else(|| "nothing answered".to_string());
    say!(out, "no deck: {why}");
    None
}

/// `max write 182 B, info 01 01 10 14 03 (version 1, …)`.
#[must_use]
pub fn describe_ready(max_write: usize, info: Option<[u8; 5]>) -> String {
    let info = match info {
        Some(bytes) => format!("{} ({})", hex(&bytes), Info::from_bytes(bytes)),
        None => "unreadable".to_string(),
    };
    let per_frame = codec::samples_per_frame(max_write.min(MAX_FRAME));
    let frames = if per_frame == codec::FRAME_SAMPLES {
        String::new()
    } else {
        format!(" (frames carry {per_frame} samples at this limit)")
    };
    format!("max write {max_write} B{frames}, info {info}")
}

/// Why this host and the deck would not understand each other, if they
/// would not.
fn incompatible(bytes: [u8; 5]) -> Option<String> {
    let info = Info::from_bytes(bytes);
    (!info.compatible()).then(|| {
        format!(
            "this host speaks version {}, IMA ADPCM, {} kHz; the deck says {info}",
            super::INFO_VERSION,
            codec::RATE_KHZ
        )
    })
}

/// When frame `k` (from 0) of a stream is due, counted from its start: the
/// moment the audio already sent would run less than `lead` ahead of real
/// time without it — the frame's end, minus the lead. The first `lead`'s
/// worth goes at once.
#[must_use]
pub fn due(k: u64, samples_per_frame: usize, lead: Duration) -> Duration {
    let end_us = (k + 1) * samples_per_frame as u64 * 1_000_000 / u64::from(RATE_HZ);
    Duration::from_micros(end_us).saturating_sub(lead)
}

fn sleep_until(when: Instant) {
    if let Some(left) = when.checked_duration_since(Instant::now()) {
        std::thread::sleep(left);
    }
}

/// END reasons by name.
#[must_use]
pub fn end_reason(reason: u8) -> &'static str {
    match reason {
        codec::END_RELEASE => "release",
        codec::END_ABORT => "abort",
        codec::END_LIMIT => "limit",
        codec::END_ERROR => "error",
        _ => "unknown",
    }
}

fn drain_stale(mic: &crossbeam_channel::Receiver<MicEvent>) {
    while mic.try_recv().is_ok() {}
}

// ---------------------------------------------------------------------------
// The microphone
// ---------------------------------------------------------------------------

/// One second of the capture.
#[derive(Clone, Copy, Debug, Default)]
struct Second {
    frames: u64,
    bytes: u64,
    lost: u64,
    level: Level,
}

/// What the capture added up to: the per-second lines and the summary are
/// made from this, and it is pure so the arithmetic has tests.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    stream: Option<u8>,
    seconds: Vec<Second>,
    pub frames: u64,
    pub bytes: u64,
    pub lost: u64,
    /// Since the MIC request: the first and last AUDIO frame.
    pub first: Option<Duration>,
    pub last: Option<Duration>,
    last_seq: Option<u8>,
    samples_per_frame: usize,
    /// The END's reason, once it came.
    pub end: Option<u8>,
}

impl Tally {
    /// Fold one event in, `at` after the MIC request. Only the first stream
    /// that starts counts. `true` once the stream has ended.
    pub fn event(&mut self, at: Duration, event: &MicEvent) -> bool {
        match event {
            MicEvent::Start { stream } => {
                self.stream.get_or_insert(*stream);
            }
            MicEvent::Audio {
                stream,
                seq,
                lost_frames,
                samples,
            } => {
                if *self.stream.get_or_insert(*stream) != *stream {
                    return false;
                }
                let second = self.second(at);
                let bytes = (AUDIO_HEADER + samples.len() / 2) as u64;
                second.frames += 1;
                second.bytes += bytes;
                second.lost += u64::from(*lost_frames);
                second.level.push(samples);
                self.frames += 1;
                self.bytes += bytes;
                self.lost += u64::from(*lost_frames);
                self.first.get_or_insert(at);
                self.last = Some(at);
                self.last_seq = Some(*seq);
                if self.samples_per_frame == 0 {
                    self.samples_per_frame = samples.len();
                }
            }
            MicEvent::End {
                stream,
                next_seq,
                reason,
            } => {
                if self.stream != Some(*stream) {
                    return false;
                }
                let expected = self.last_seq.map_or(0, |s| s.wrapping_add(1));
                let tail = u64::from(next_seq.wrapping_sub(expected));
                self.lost += tail;
                self.second(at).lost += tail;
                self.end = Some(*reason);
                return true;
            }
        }
        false
    }

    fn second(&mut self, at: Duration) -> &mut Second {
        let index = at.as_secs() as usize;
        if self.seconds.len() <= index {
            self.seconds.resize(index + 1, Second::default());
        }
        &mut self.seconds[index]
    }

    /// The line for second `index` (from 0).
    #[must_use]
    pub fn line(&self, index: usize) -> String {
        let s = self.seconds.get(index).copied().unwrap_or_default();
        format!(
            "  {:>2} s  {:>3} frames  {:>6} B  {:>3} lost  rms {:>6.1} dBFS  peak {:>6.1} dBFS",
            index + 1,
            s.frames,
            s.bytes,
            s.lost,
            s.level.rms_dbfs(),
            s.level.peak_dbfs()
        )
    }

    /// Seconds with a line so far.
    #[must_use]
    pub fn seconds(&self) -> usize {
        self.seconds.len()
    }

    #[must_use]
    pub fn loss_pct(&self) -> f64 {
        let total = self.frames + self.lost;
        if total == 0 {
            0.0
        } else {
            100.0 * self.lost as f64 / total as f64
        }
    }

    /// From the first frame's arrival to the last one's, plus the last
    /// frame's own duration.
    #[must_use]
    pub fn span(&self) -> Duration {
        match (self.first, self.last) {
            (Some(first), Some(last)) => {
                let frame = Duration::from_micros(
                    self.samples_per_frame as u64 * 1_000_000 / u64::from(RATE_HZ),
                );
                last.saturating_sub(first) + frame
            }
            _ => Duration::ZERO,
        }
    }

    #[must_use]
    pub fn summary(&self) -> String {
        let span = self.span().as_secs_f64();
        let (fps, kbps) = if span > 0.0 {
            (
                self.frames as f64 / span,
                self.bytes as f64 * 8.0 / span / 1000.0,
            )
        } else {
            (0.0, 0.0)
        };
        let first = self.first.map_or_else(
            || "no frame".to_string(),
            |d| format!("first frame {} ms after MIC", d.as_millis()),
        );
        let end = self.end.map_or_else(
            || "no END".to_string(),
            |r| format!("END {}", end_reason(r)),
        );
        format!(
            "{} frames over {span:.2} s ({fps:.1} frames/s, {kbps:.1} kbit/s), {} lost ({:.1} %), \
             {first}, {end}",
            self.frames,
            self.lost,
            self.loss_pct()
        )
    }

    /// Why the capture fails, if it does.
    pub fn verdict(&self) -> Result<(), String> {
        let mut why = Vec::new();
        if self.frames == 0 {
            why.push("no audio arrived".to_string());
        }
        if self.loss_pct() >= MAX_LOSS_PCT {
            why.push(format!(
                "{:.1} % of the frames were lost (the limit is {MAX_LOSS_PCT} %)",
                self.loss_pct()
            ));
        }
        if self.end.is_none() {
            why.push("the stream never ended with an END".to_string());
        }
        if why.is_empty() {
            Ok(())
        } else {
            Err(why.join("; "))
        }
    }
}

/// Write `samples` as a 16 kHz mono 16-bit WAV, and say so.
fn save_wav(path: &std::path::Path, samples: &[i16], out: &mut dyn Write) {
    let data = samples.len() as u32 * 2;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE_HZ.to_le_bytes());
    bytes.extend_from_slice(&(RATE_HZ * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    match std::fs::write(path, bytes) {
        Ok(()) => say!(
            out,
            "saved {} samples ({:.2} s) to {}",
            samples.len(),
            samples.len() as f64 / f64::from(RATE_HZ),
            path.display()
        ),
        Err(e) => say!(out, "could not save {}: {e}", path.display()),
    }
}

fn mic_test(
    ble: &BleAudio,
    seconds: u8,
    wav: Option<&std::path::Path>,
    out: &mut dyn Write,
) -> bool {
    let mut captured: Vec<i16> = Vec::new();
    let mic = ble.mic_events();
    drain_stale(&mic);
    let t0 = Instant::now();
    if let Err(e) = ble.mic(codec::MIC_ON, seconds) {
        say!(out, "FAIL: the MIC request was refused: {e}");
        return false;
    }
    say!(out, "microphone on for {seconds} s (MIC mode 1)");
    let deadline = t0 + Duration::from_secs(u64::from(seconds)) + GRACE;
    let mut tally = Tally::default();
    let mut printed = 0usize;
    let mut ended = false;
    let mut dropped = false;
    loop {
        let now = Instant::now();
        let elapsed = now - t0;
        // Every second that is over gets its line, empty or not.
        while printed < elapsed.as_secs() as usize {
            say!(out, "{}", tally.line(printed));
            printed += 1;
        }
        if now >= deadline {
            break;
        }
        if !matches!(ble.state(), LinkState::Ready { .. }) {
            dropped = true;
            break;
        }
        let next_second = t0 + Duration::from_secs(elapsed.as_secs() + 1);
        match mic.recv_timeout(next_second.min(deadline) - now) {
            Ok(event) => {
                if wav.is_some() {
                    if let MicEvent::Audio { samples, .. } = &event {
                        captured.extend_from_slice(samples);
                    }
                }
                if tally.event(t0.elapsed(), &event) {
                    ended = true;
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    while printed < tally.seconds() {
        say!(out, "{}", tally.line(printed));
        printed += 1;
    }
    if !ended {
        // Stop the deck rather than leave it streaming to nobody; the frame
        // has to leave the outbox before this process does.
        let _ = ble.mic(codec::MIC_OFF, 1);
        settle(ble);
    }
    say!(out, "summary: {}", tally.summary());
    if let Some(path) = wav {
        save_wav(path, &captured, out);
    }
    if dropped {
        say!(out, "FAIL: the link dropped during the capture");
        return false;
    }
    match tally.verdict() {
        Ok(()) => {
            say!(out, "PASS");
            true
        }
        Err(why) => {
            say!(out, "FAIL: {why}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// The speaker
// ---------------------------------------------------------------------------

/// How a tone stream went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToneReport {
    /// AUDIO frames queued.
    pub frames: u64,
    /// AUDIO frames the queue refused (full).
    pub refused: u64,
    /// A refusal that ended the stream (the link dropped, or a frame was
    /// over the limit).
    pub error: Option<SendError>,
}

/// A stream id for the host side: any value, as long as consecutive runs
/// differ.
fn stream_id() -> u8 {
    (crate::core::new_sid() & 0xFF) as u8
}

/// Play `seconds` of a sine at `hz` as one speaker stream — START, AUDIO
/// frames of `samples_per_frame` paced at [`due`], END — calling
/// `each_second` after every second of audio is queued.
pub fn stream_tone(
    ble: &BleAudio,
    seconds: u8,
    hz: u32,
    peak_dbfs: f64,
    samples_per_frame: usize,
    mut each_second: impl FnMut(u64, &ToneReport),
) -> ToneReport {
    let mut report = ToneReport::default();
    let stream = stream_id();
    let mut buf = [0u8; MAX_FRAME];
    let n = codec::start_frame(stream, &mut buf);
    if let Err(e) = ble.send(&buf[..n]) {
        report.error = Some(e);
        return report;
    }
    let total = usize::from(seconds) * RATE_HZ as usize;
    let frames = total.div_ceil(samples_per_frame);
    let mut sine = Sine::new(f64::from(hz), peak_dbfs);
    let mut state = codec::State::default();
    let mut pcm = vec![0i16; samples_per_frame];
    let t0 = Instant::now();
    let mut next_seq = 0u8;
    for k in 0..frames {
        sleep_until(t0 + due(k as u64, samples_per_frame, LEAD));
        let start = k * samples_per_frame;
        let len = samples_per_frame.min(total - start);
        let chunk = &mut pcm[..len];
        sine.fill(chunk);
        for (i, sample) in chunk.iter_mut().enumerate() {
            *sample = (f64::from(*sample) * fade(start + i, total, TONE_RAMP)).round() as i16;
        }
        let n = codec::audio_frame(stream, next_seq, &mut state, chunk, &mut buf);
        next_seq = next_seq.wrapping_add(1);
        match ble.send(&buf[..n]) {
            Ok(()) => report.frames += 1,
            Err(SendError::Full) => report.refused += 1,
            Err(e) => {
                report.error = Some(e);
                return report;
            }
        }
        let rate = RATE_HZ as usize;
        if (start + len) / rate > start / rate {
            each_second(((start + len) / rate) as u64, &report);
        }
    }
    // END matters more than any one frame: give a full queue a moment.
    let n = codec::end_frame(stream, next_seq, codec::END_RELEASE, &mut buf);
    for _ in 0..50 {
        match ble.send(&buf[..n]) {
            Err(SendError::Full) => std::thread::sleep(Duration::from_millis(10)),
            Ok(()) => break,
            Err(e) => {
                report.error = Some(e);
                break;
            }
        }
    }
    report
}

/// Wait (briefly) for the outbox to empty, so "written" counts everything.
fn settle(ble: &BleAudio) {
    let until = Instant::now() + Duration::from_millis(500);
    while ble.stats().queued > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn samples_per_frame(max_write: usize, out: &mut dyn Write) -> Option<usize> {
    let n = codec::samples_per_frame(max_write.min(MAX_FRAME));
    if n == 0 {
        say!(
            out,
            "FAIL: a {max_write}-byte write limit has no room for audio"
        );
        None
    } else {
        Some(n)
    }
}

fn tone_test(
    ble: &BleAudio,
    seconds: u8,
    hz: u32,
    opts: &Options,
    max_write: usize,
    out: &mut dyn Write,
) -> bool {
    let Some(per_frame) = samples_per_frame(max_write, out) else {
        return false;
    };
    let silence = opts.silence;
    let level = opts.level;
    let peak = if silence { f64::NEG_INFINITY } else { level };
    if silence {
        say!(
            out,
            "speaker: digital silence for {seconds} s ({per_frame} samples a frame, {} ms lead)",
            LEAD.as_millis()
        );
    } else {
        say!(
            out,
            "speaker: {hz} Hz at {level} dBFS for {seconds} s ({per_frame} samples a frame, \
             {} ms lead)",
            LEAD.as_millis()
        );
    }
    let report = stream_tone(ble, seconds, hz, peak, per_frame, |second, so_far| {
        say!(
            out,
            "  {second:>2} s  {:>4} frames queued  {:>3} refused  {:>2} waiting",
            so_far.frames,
            so_far.refused,
            ble.stats().queued
        );
    });
    settle(ble);
    let stats = ble.stats();
    say!(
        out,
        "summary: {} frames queued, {} refused, {} written to the radio, {} waiting",
        report.frames,
        report.refused,
        stats.sent,
        stats.queued
    );
    match (report.error, report.refused) {
        (Some(e), _) => {
            say!(out, "FAIL: the stream stopped: {e}");
            false
        }
        (None, 0) => {
            say!(out, "PASS (did it sound like a clean {hz} Hz tone?)");
            true
        }
        (None, refused) => {
            say!(out, "FAIL: {refused} frames were refused by a full queue");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Loopback
// ---------------------------------------------------------------------------

/// One analysed second of the loopback capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub tone_dbfs: f64,
    pub off_dbfs: f64,
}

impl Window {
    #[must_use]
    pub fn of(samples: &[i16], hz: f64) -> Window {
        Window {
            tone_dbfs: tone_dbfs(samples, hz),
            off_dbfs: tone_dbfs(samples, off_frequency(hz)),
        }
    }

    #[must_use]
    pub fn ratio_db(&self) -> f64 {
        self.tone_dbfs - self.off_dbfs
    }

    #[must_use]
    pub fn passes(&self) -> bool {
        self.ratio_db() > LOOPBACK_DB
    }
}

/// At least half of the windows (and at least one) pass.
#[must_use]
pub fn loopback_verdict(windows: &[Window]) -> bool {
    let passing = windows.iter().filter(|w| w.passes()).count();
    !windows.is_empty() && passing * 2 >= windows.len()
}

fn loopback_test(
    ble: &BleAudio,
    seconds: u8,
    hz: u32,
    opts: &Options,
    max_write: usize,
    out: &mut dyn Write,
) -> bool {
    let Some(per_frame) = samples_per_frame(max_write, out) else {
        return false;
    };
    let silence = opts.silence;
    let peak = if silence {
        f64::NEG_INFINITY
    } else {
        opts.level
    };
    let mut captured: Vec<i16> = Vec::new();
    let mic = ble.mic_events();
    drain_stale(&mic);
    // A second longer than the tone: the probe stops it once the tone is out.
    let capture = seconds.saturating_add(1).min(60);
    if let Err(e) = ble.mic(codec::MIC_LOOPBACK, capture) {
        say!(out, "FAIL: the MIC request was refused: {e}");
        return false;
    }
    let off = off_frequency(f64::from(hz));
    if silence {
        say!(
            out,
            "loopback: digital silence for {seconds} s, captured with MIC mode 2 (the speaker \
             path's noise floor as the microphone hears it)"
        );
    } else {
        say!(
            out,
            "loopback: {hz} Hz at {peak} dBFS for {seconds} s, captured with MIC mode 2; each \
             second needs {hz} Hz {LOOPBACK_DB} dB above {off} Hz"
        );
    }
    let speaker = ble.clone();
    let tone =
        std::thread::spawn(move || stream_tone(&speaker, seconds, hz, peak, per_frame, |_, _| {}));

    let rate = RATE_HZ as usize;
    let deadline = Instant::now() + Duration::from_secs(u64::from(capture)) + GRACE;
    let mut stream = None;
    let mut buffer: Vec<i16> = Vec::with_capacity(2 * rate);
    let mut windows = Vec::new();
    let mut tone_done: Option<Instant> = None;
    let mut stopped = false;
    let analyse = |samples: &[i16], windows: &mut Vec<Window>, out: &mut dyn Write| {
        let w = Window::of(samples, f64::from(hz));
        windows.push(w);
        say!(
            out,
            "  {:>2} s  {hz} Hz {:>6.1} dBFS  {off} Hz {:>6.1} dBFS  ratio {:>5.1} dB  {}",
            windows.len(),
            w.tone_dbfs,
            w.off_dbfs,
            w.ratio_db(),
            if w.passes() { "ok" } else { "--" }
        );
    };
    loop {
        let now = Instant::now();
        if now >= deadline || !matches!(ble.state(), LinkState::Ready { .. }) {
            break;
        }
        // Once the tone is out (and its tail has had time to come back),
        // end the capture rather than analyse a second of silence.
        if tone_done.is_none() && tone.is_finished() {
            tone_done = Some(now);
        }
        if !stopped && tone_done.is_some_and(|t| now >= t + Duration::from_millis(250)) {
            let _ = ble.mic(codec::MIC_OFF, 1);
            stopped = true;
        }
        match mic.recv_timeout(Duration::from_millis(50)) {
            Ok(MicEvent::Start { stream: s }) => {
                stream.get_or_insert(s);
            }
            Ok(MicEvent::Audio {
                stream: s, samples, ..
            }) => {
                if *stream.get_or_insert(s) == s {
                    if opts.wav.is_some() {
                        captured.extend_from_slice(&samples);
                    }
                    buffer.extend_from_slice(&samples);
                    while buffer.len() >= rate {
                        analyse(&buffer[..rate], &mut windows, out);
                        buffer.drain(..rate);
                    }
                }
            }
            Ok(MicEvent::End { stream: s, .. }) => {
                if stream == Some(s) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // A tail of at least half a second is still a measurement.
    if buffer.len() >= rate / 2 {
        analyse(&buffer, &mut windows, out);
    }
    let report = tone.join().unwrap_or_default();
    if !stopped {
        let _ = ble.mic(codec::MIC_OFF, 1);
    }
    settle(ble);
    if report.refused > 0 || report.error.is_some() {
        say!(
            out,
            "note: the tone lost {} frames to a full queue{}",
            report.refused,
            report
                .error
                .map(|e| format!(" and stopped: {e}"))
                .unwrap_or_default()
        );
    }
    if let Some(path) = opts.wav.as_deref() {
        save_wav(path, &captured, out);
    }
    if silence {
        // Nothing to find in silence: it passes when the capture came back.
        if windows.is_empty() {
            say!(out, "FAIL: no microphone audio came back");
        } else {
            say!(
                out,
                "PASS: {} s captured over digital silence",
                windows.len()
            );
        }
        return !windows.is_empty();
    }
    let passing = windows.iter().filter(|w| w.passes()).count();
    if loopback_verdict(&windows) {
        let mut ratios: Vec<f64> = windows.iter().map(Window::ratio_db).collect();
        ratios.sort_by(f64::total_cmp);
        say!(
            out,
            "PASS: the tone came back in {passing} of {} s (median {:.1} dB above {off} Hz)",
            windows.len(),
            ratios[ratios.len() / 2]
        );
        true
    } else if windows.is_empty() {
        say!(out, "FAIL: no microphone audio came back");
        false
    } else {
        say!(
            out,
            "FAIL: the tone stood {LOOPBACK_DB} dB clear in only {passing} of {} s",
            windows.len()
        );
        false
    }
}

// ---------------------------------------------------------------------------
// scan
// ---------------------------------------------------------------------------

/// `deckd ble scan`: look for `window`, print what is there, connect to
/// nothing. 0 when a deck (by service or name) is there, 2 when not.
#[cfg(target_os = "macos")]
pub fn scan(window: Duration, name: &str, out: &mut dyn Write) -> i32 {
    use super::attach::Radio;
    say!(
        out,
        "looking without connecting (scan for {} s)",
        window.as_secs_f64()
    );
    let look = super::look(window, name);
    say!(
        out,
        "Bluetooth permission for this process: {}",
        permission_word(look.bluetooth)
    );
    if look.radio != Radio::On {
        say!(out, "no deck: {}", look.radio.why());
        return EXIT_NO_DEVICE;
    }
    say!(out, "Bluetooth is on");
    for s in &look.sightings {
        let name = s
            .name
            .as_deref()
            .map_or_else(|| "(no name)".to_string(), |n| format!("{n:?}"));
        let rssi = s.rssi.map(|r| format!(", {r} dBm")).unwrap_or_default();
        say!(out, "  {name} {}: {}{rssi}", s.id, s.how);
    }
    if look.sightings.is_empty() {
        say!(out, "  nothing connected with the audio or Battery service");
    }
    say!(
        out,
        "  {} other advertisers (not {name:?}) seen",
        look.other_advertisers
    );
    let deck = look
        .sightings
        .iter()
        .any(|s| s.how == "connected, with the audio service" || s.name.as_deref() == Some(name));
    if deck {
        EXIT_PASS
    } else {
        say!(
            out,
            "no deck: nothing offers the audio service or is named {name:?}"
        );
        EXIT_NO_DEVICE
    }
}

/// The wire spelling of a permission (`perm.status`'s), for the bench.
#[cfg(target_os = "macos")]
fn permission_word(permission: deck_model::Permission) -> &'static str {
    match permission {
        deck_model::Permission::Unknown => "unknown",
        deck_model::Permission::NotDetermined => "not_determined",
        deck_model::Permission::Granted => "granted",
        deck_model::Permission::Denied => "denied",
    }
}

/// `deckd ble scan` where there is no CoreBluetooth.
#[cfg(not(target_os = "macos"))]
pub fn scan(_window: Duration, _name: &str, out: &mut dyn Write) -> i32 {
    say!(out, "no deck: CoreBluetooth exists only on macOS");
    EXIT_NO_DEVICE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::codec::{audio_frame, end_frame, start_frame, State, FRAME_SAMPLES};
    use crate::ble::FakeDeck;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    const INFO: [u8; 5] = [1, 1, 16, 20, 3];

    fn options(seconds: u8, tone: Option<u32>, loopback: bool) -> Options {
        Options {
            seconds,
            tone,
            loopback,
            silence: false,
            level: TONE_DBFS,
            wav: None,
            name: crate::ble::DECK_NAME.into(),
            attach_timeout: Duration::from_millis(300),
        }
    }

    fn frame(build: impl FnOnce(&mut [u8]) -> usize) -> Vec<u8> {
        let mut buf = [0u8; MAX_FRAME];
        let n = build(&mut buf);
        buf[..n].to_vec()
    }

    /// A deck that answers MIC requests: mode 1 streams `seconds` of a
    /// 440 Hz tone at once (dropping one frame in ten when `lossy`); mode 2
    /// echoes every speaker frame back as microphone audio until MIC off.
    fn fake_deck(
        deck: FakeDeck,
        stop: Arc<AtomicBool>,
        lossy: bool,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut echo: Option<(u8, u8, State)> = None;
            while !stop.load(Ordering::Relaxed) {
                for value in deck.written(64) {
                    match codec::parse(&value) {
                        codec::Frame::Mic { mode: 1, seconds } => {
                            deck.notify(&frame(|b| start_frame(7, b)));
                            let mut sine = Sine::new(440.0, -12.0);
                            let mut state = State::default();
                            let frames = usize::from(seconds) * 50;
                            for seq in 0..frames {
                                let mut pcm = [0i16; FRAME_SAMPLES];
                                sine.fill(&mut pcm);
                                let value =
                                    frame(|b| audio_frame(7, seq as u8, &mut state, &pcm, b));
                                if !(lossy && seq % 10 == 3) {
                                    deck.notify(&value);
                                }
                            }
                            deck.notify(&frame(|b| {
                                end_frame(7, frames as u8, codec::END_LIMIT, b)
                            }));
                        }
                        codec::Frame::Mic { mode: 2, .. } => {
                            deck.notify(&frame(|b| start_frame(8, b)));
                            echo = Some((8, 0, State::default()));
                        }
                        codec::Frame::Mic { mode: 0, .. } => {
                            if let Some((stream, seq, _)) = echo.take() {
                                deck.notify(&frame(|b| {
                                    end_frame(stream, seq, codec::END_ABORT, b)
                                }));
                            }
                        }
                        codec::Frame::Audio {
                            mut state, data, ..
                        } => {
                            if let Some((stream, seq, mic)) = echo.as_mut() {
                                let mut pcm = [0i16; FRAME_SAMPLES];
                                let n = codec::decode(&mut state, data, &mut pcm);
                                let value =
                                    frame(|b| audio_frame(*stream, *seq, mic, &pcm[..n], b));
                                *seq = seq.wrapping_add(1);
                                deck.notify(&value);
                            }
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    }

    fn probe(opts: &Options, lossy: bool) -> (i32, String) {
        let (ble, deck) = BleAudio::with_fake();
        deck.attach(182, Some(INFO));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = fake_deck(deck, stop.clone(), lossy);
        let mut out = Vec::new();
        let code = run(&ble, opts, &mut out);
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn frames_are_due_at_their_end_minus_the_lead() {
        let lead = Duration::from_millis(60);
        assert_eq!(due(0, 320, lead), Duration::ZERO);
        assert_eq!(due(2, 320, lead), Duration::ZERO, "60 ms go at once");
        assert_eq!(due(3, 320, lead), Duration::from_millis(20));
        assert_eq!(due(49, 320, lead), Duration::from_millis(940));
        // 28-sample frames (a 20-byte limit) are 1.75 ms apart.
        assert_eq!(due(100, 28, lead), Duration::from_micros(101 * 1750) - lead);
    }

    #[test]
    fn the_tally_counts_frames_bytes_and_loss_including_the_tail() {
        let mut tally = Tally::default();
        let audio = |seq: u8, lost: u8| MicEvent::Audio {
            stream: 2,
            seq,
            lost_frames: lost,
            samples: vec![1000; FRAME_SAMPLES],
        };
        let ms = Duration::from_millis;
        assert!(!tally.event(ms(80), &MicEvent::Start { stream: 2 }));
        assert!(!tally.event(ms(100), &audio(0, 0)));
        assert!(!tally.event(ms(120), &audio(1, 0)));
        // Somebody else's stream does not count.
        assert!(!tally.event(
            ms(130),
            &MicEvent::Audio {
                stream: 3,
                seq: 0,
                lost_frames: 0,
                samples: vec![0; 4]
            }
        ));
        assert!(!tally.event(ms(1_180), &audio(4, 2)));
        assert!(tally.event(
            ms(1_200),
            &MicEvent::End {
                stream: 2,
                next_seq: 6,
                reason: codec::END_LIMIT
            }
        ));
        assert_eq!((tally.frames, tally.lost), (3, 3), "seq 2, 3 and 5");
        assert_eq!(tally.bytes, 3 * MAX_FRAME as u64);
        assert_eq!(tally.seconds(), 2);
        assert!(tally.line(0).contains("  2 frames"), "{}", tally.line(0));
        assert!(tally.line(1).contains("  3 lost"), "{}", tally.line(1));
        assert_eq!(tally.span(), ms(1_100));
        assert!((tally.loss_pct() - 50.0).abs() < 1e-9);
        assert_eq!(
            tally.summary(),
            "3 frames over 1.10 s (2.7 frames/s, 3.6 kbit/s), 3 lost (50.0 %), first frame 100 \
             ms after MIC, END limit"
        );
        assert!(tally
            .verdict()
            .unwrap_err()
            .contains("50.0 % of the frames were lost"));

        let empty = Tally::default();
        assert_eq!(
            empty.verdict(),
            Err("no audio arrived; the stream never ended with an END".into())
        );
    }

    #[test]
    fn loopback_passes_on_a_majority_of_clear_seconds() {
        let clear = Window {
            tone_dbfs: -30.0,
            off_dbfs: -60.0,
        };
        let murky = Window {
            tone_dbfs: -50.0,
            off_dbfs: -60.0,
        };
        assert!(clear.passes() && !murky.passes());
        assert!(loopback_verdict(&[clear]));
        assert!(loopback_verdict(&[clear, murky]));
        assert!(loopback_verdict(&[murky, clear, clear]));
        assert!(!loopback_verdict(&[murky, murky, clear]));
        assert!(!loopback_verdict(&[]));
        let window = Window::of(
            &{
                let mut pcm = vec![0i16; 16_000];
                Sine::new(1000.0, -20.0).fill(&mut pcm);
                pcm
            },
            1000.0,
        );
        assert!(window.passes(), "{window:?}");
    }

    #[test]
    fn ready_lines_describe_the_limit_and_the_info() {
        assert_eq!(
            describe_ready(182, Some(INFO)),
            "max write 182 B, info 01 01 10 14 03 (version 1, IMA ADPCM, 16 kHz, 20 ms, mic + \
             speaker)"
        );
        assert_eq!(
            describe_ready(20, None),
            "max write 20 B (frames carry 28 samples at this limit), info unreadable"
        );
        assert!(incompatible(INFO).is_none());
        assert!(incompatible([2, 1, 16, 20, 3])
            .unwrap()
            .contains("the deck says version 2"));
        assert_eq!(end_reason(codec::END_ABORT), "abort");
        assert_eq!(end_reason(9), "unknown");
    }

    #[test]
    fn no_deck_is_exit_2_with_the_reason() {
        let (ble, deck) = BleAudio::with_fake();
        deck.detach("OpenWalkieTalkie has no audio service (firmware without voice?)");
        let mut out = Vec::new();
        let code = run(&ble, &options(1, None, false), &mut out);
        let out = String::from_utf8(out).unwrap();
        assert_eq!(code, EXIT_NO_DEVICE, "{out}");
        assert!(
            out.ends_with(
                "no deck: OpenWalkieTalkie has no audio service (firmware without voice?)\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn the_microphone_probe_passes_a_clean_stream_and_fails_a_lossy_one() {
        let (code, out) = probe(&options(2, None, false), false);
        assert_eq!(code, EXIT_PASS, "{out}");
        assert!(out.contains("ready: max write 182 B"), "{out}");
        assert!(out.contains("summary: 100 frames"), "{out}");
        assert!(out.contains("0 lost (0.0 %)"), "{out}");
        assert!(out.contains("END limit"), "{out}");
        assert!(out.ends_with("PASS\n"), "{out}");

        let (code, out) = probe(&options(2, None, false), true);
        assert_eq!(code, EXIT_FAIL, "{out}");
        assert!(out.contains("summary: 90 frames"), "{out}");
        assert!(out.contains("10 lost (10.0 %)"), "{out}");
        assert!(
            out.ends_with("FAIL: 10.0 % of the frames were lost (the limit is 5 %)\n"),
            "{out}"
        );
    }

    #[test]
    fn the_tone_probe_streams_paced_frames_and_ends_the_stream() {
        let (ble, deck) = BleAudio::with_fake();
        deck.attach(182, Some(INFO));
        let stop = Arc::new(AtomicBool::new(false));
        // The radio: take whatever is queued, every few ms, and keep it.
        let radio = {
            let (deck, stop) = (deck.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut all = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    all.extend(deck.written(64));
                    std::thread::sleep(Duration::from_millis(2));
                }
                all.extend(deck.written(64));
                all
            })
        };
        let t0 = Instant::now();
        let mut out = Vec::new();
        let code = run(&ble, &options(1, Some(1000), false), &mut out);
        let took = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let written = radio.join().unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(code, EXIT_PASS, "{out}");
        // Paced: a second of audio takes about a second, minus the lead.
        assert!(took >= Duration::from_millis(900), "{took:?}");
        let kinds: Vec<u8> = written.iter().map(|f| f[0]).collect();
        assert_eq!(kinds.first(), Some(&codec::KIND_START));
        assert_eq!(kinds.last(), Some(&codec::KIND_END));
        assert_eq!(
            kinds.iter().filter(|&&k| k == codec::KIND_AUDIO).count(),
            50
        );
        // seq runs 0..50 and END says 50 is next.
        assert_eq!(written[1][2], 0);
        assert_eq!(written[50][2], 49);
        assert_eq!(written[51][2], 50);
        // The tone is in there: decode it back.
        let mut pcm = Vec::new();
        for value in &written[1..51] {
            let codec::Frame::Audio {
                mut state, data, ..
            } = codec::parse(value)
            else {
                panic!("audio");
            };
            let mut out = [0i16; FRAME_SAMPLES];
            let n = codec::decode(&mut state, data, &mut out);
            pcm.extend_from_slice(&out[..n]);
        }
        assert!((tone_dbfs(&pcm, 1000.0) - TONE_DBFS).abs() < 1.0);
        assert!(
            out.contains("summary: 50 frames queued, 0 refused, 52 written"),
            "{out}"
        );
    }

    #[test]
    fn the_loopback_probe_hears_its_own_tone_through_an_echoing_deck() {
        let (code, out) = probe(&options(1, Some(1000), true), false);
        assert_eq!(code, EXIT_PASS, "{out}");
        assert!(out.contains("PASS: the tone came back"), "{out}");
        assert!(out.contains("  1 s  1000 Hz"), "{out}");
    }
}
