//! `deckd-tts` — §2.3's synthesis thread: engine chunks → rubato → the lane
//! rings' producer halves, and the pre-rendered chimes of §7.3.
//!
//! **V13 rewrites §2.3's row for this thread.** The design draws it as
//! "AVSpeech `writeUtterance` buffer callbacks → rubato → lane rings", with
//! the callbacks landing here. They cannot: measured, a worker thread with
//! no run loop received **0 buffers**, a worker driving its own run loop
//! received **0**, and the main thread driving its run loop received
//! **300/300**. So the buffers land on main ([`crate::audio::mainloop`]),
//! which posts them down a channel, and this thread does the part that has
//! nothing to do with ObjC: resample and fill.
//!
//! It follows the `deckd-hid-write` precedent — a **blocking sink with one
//! item in flight and a stall-abandon**. Blocking is right here and would be
//! wrong on `deckd-audio`: a lane ring is 2 s deep and drains in real time,
//! so filling it *is* waiting for a loudspeaker, and §7.5.1's rule is that
//! `Capture::Open` must never queue behind that. It does not, because it is
//! on the other thread.
//!
//! **rubato 5.0.0 is a total break from 0.x** (V20): `Fft::new(…,
//! FixedSync::…)`, buffers through the separate `audioadapter` crates, and
//! `process_into_buffer` for the allocation-free path. Code written against
//! a 0.x example does not compile, and must not be "fixed" by loosening the
//! pin.
use super::lanes::{Lane, Lanes};
use super::traits::{AudioFormat, TtsChunk, TtsEngine, TtsRequest, TtsStop};
use crate::audio::LANE_RATE;
use crossbeam_channel::{Receiver, Sender};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// rubato's chunk size. V20 verified 22050→44100 mono at this size:
/// `input_frames_next = 1024`, `output_frames_next = 2048`,
/// `output_delay = 256`.
pub const CHUNK: usize = 1024;

/// How long the sink waits for room before it gives up on an item. It is
/// the lane's own TTL, because audio that could not be *queued* within its
/// TTL is exactly the audio the TTL exists to drop.
fn stall_budget(lane: Lane) -> Duration {
    lane.ttl()
}

/// How long it sleeps between attempts when a ring is full. The ring drains
/// in real time, so this is a pacing sleep and not a spin.
const SINK_POLL: Duration = Duration::from_millis(5);

/// §7.3's two chimes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chime {
    /// "two-tone rising" (`host-research/spec.md:114`).
    Question,
    /// "single soft knock".
    Ready,
}

impl Chime {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Chime> {
        match name {
            "question" => Some(Chime::Question),
            "ready" => Some(Chime::Ready),
            _ => None,
        }
    }
}

/// What `deckd-audio` asks this thread for.
#[derive(Clone, Debug, PartialEq)]
pub enum TtsCmd {
    Say {
        utterance: String,
        lane: Lane,
        text: String,
        voice: Option<String>,
    },
    Chime {
        which: Chime,
    },
}

/// What this thread tells `deckd-audio`, which turns it into B6's
/// `voice.utterance{phase}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TtsReport {
    /// The first sample of this utterance has been queued on its lane.
    Started {
        utterance: String,
        lane: Lane,
    },
    /// Every sample of it has been queued (or it was cut, or abandoned).
    Ended {
        utterance: String,
        lane: Lane,
        cut: bool,
    },
    Failed {
        utterance: String,
        code: String,
    },
}

/// The item currently being synthesised, so a barge-in can reach into it.
#[derive(Clone)]
struct Current {
    utterance: String,
    lane: Lane,
    cancel: Arc<AtomicBool>,
    stopper: Option<Arc<dyn TtsStop>>,
}

/// `deckd-audio`'s handle on `deckd-tts`.
pub struct TtsHandle {
    cmds: Option<Sender<TtsCmd>>,
    current: Arc<Mutex<Option<Current>>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for TtsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsHandle").finish_non_exhaustive()
    }
}

impl TtsHandle {
    /// Queue one command. `false` when the thread is gone.
    pub fn send(&self, cmd: TtsCmd) -> bool {
        self.cmds.as_ref().is_some_and(|tx| tx.send(cmd).is_ok())
    }

    /// Stop what is being synthesised **now**, not at the end of the chunk.
    ///
    /// This is the half of §7.7's barge-in that has to cross threads: the
    /// synth thread is blocked filling a ring, so `&mut self` on the job
    /// could not reach it (V16) — hence [`TtsStop`].
    pub fn cut(&self, all: bool, lane: Option<Lane>, utterance: Option<&str>) -> bool {
        let guard = self.current.lock().unwrap();
        let Some(current) = guard.as_ref() else {
            return false;
        };
        let names_it = all
            || lane.is_some_and(|l| l == current.lane)
            || utterance.is_some_and(|u| u == current.utterance);
        if names_it {
            current.cancel.store(true, Ordering::SeqCst);
            if let Some(stopper) = current.stopper.as_ref() {
                stopper.stop();
            }
        }
        names_it
    }

    /// The utterance being synthesised, if any.
    #[must_use]
    pub fn speaking(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.utterance.clone())
    }

    pub fn join(mut self) {
        drop(self.cmds.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Start `deckd-tts`.
pub fn spawn(
    engine: Arc<dyn TtsEngine>,
    lanes: Arc<Lanes>,
    reports: Sender<TtsReport>,
) -> TtsHandle {
    let (tx, rx) = crossbeam_channel::unbounded::<TtsCmd>();
    let current = Arc::new(Mutex::new(None));
    let thread_current = current.clone();
    let join = std::thread::Builder::new()
        .name("deckd-tts".into())
        .spawn(move || thread_main(&engine, &lanes, &rx, &reports, &thread_current))
        .ok();
    TtsHandle {
        cmds: Some(tx),
        current,
        join,
    }
}

fn thread_main(
    engine: &Arc<dyn TtsEngine>,
    lanes: &Arc<Lanes>,
    cmds: &Receiver<TtsCmd>,
    reports: &Sender<TtsReport>,
    current: &Arc<Mutex<Option<Current>>>,
) {
    // The producers, taken once (§7.2: `deckd-tts` owns the producer halves
    // for the daemon's whole life; only the consumers move).
    let mut producers: Vec<(Lane, rtrb::Producer<f32>)> = Lane::ALL
        .into_iter()
        .filter_map(|lane| lanes.take_producer(lane).map(|p| (lane, p)))
        .collect();
    let mut chimes = ChimeBank::default();

    while let Ok(cmd) = cmds.recv() {
        match cmd {
            TtsCmd::Say {
                utterance,
                lane,
                text,
                voice,
            } => {
                let cancel = Arc::new(AtomicBool::new(false));
                *current.lock().unwrap() = Some(Current {
                    utterance: utterance.clone(),
                    lane,
                    cancel: cancel.clone(),
                    stopper: None,
                });
                let outcome = say(
                    engine,
                    lanes,
                    &mut producers,
                    lane,
                    &utterance,
                    &text,
                    voice,
                    &cancel,
                    reports,
                    current,
                );
                *current.lock().unwrap() = None;
                lanes.set_active(lane, false);
                match outcome {
                    Ok(cut) => {
                        let _ = reports.send(TtsReport::Ended {
                            utterance,
                            lane,
                            cut,
                        });
                    }
                    Err(code) => {
                        let _ = reports.send(TtsReport::Failed { utterance, code });
                    }
                }
            }
            TtsCmd::Chime { which } => {
                let pcm = chimes.get(which);
                let lane = Lane::Chime;
                lanes.set_active(lane, true);
                let cancel = Arc::new(AtomicBool::new(false));
                if let Some((_, producer)) = producers.iter_mut().find(|(l, _)| *l == lane) {
                    sink(producer, pcm, lane, &cancel);
                }
                lanes.set_active(lane, false);
            }
        }
    }

    // Hand the producers back, so a restart finds them.
    for (lane, producer) in producers {
        lanes.put_producer(lane, producer);
    }
}

#[allow(clippy::too_many_arguments)]
fn say(
    engine: &Arc<dyn TtsEngine>,
    lanes: &Arc<Lanes>,
    producers: &mut [(Lane, rtrb::Producer<f32>)],
    lane: Lane,
    utterance: &str,
    text: &str,
    voice: Option<String>,
    cancel: &Arc<AtomicBool>,
    reports: &Sender<TtsReport>,
    current: &Arc<Mutex<Option<Current>>>,
) -> Result<bool, String> {
    let job = engine
        .synthesize(&TtsRequest {
            text: text.to_string(),
            voice,
            rate: 0.5,
        })
        .map_err(|e| e.code().to_string())?;
    // V16: the format is known **before** the first chunk, so the resampler
    // exists before the PCM does. AVSpeech is 22050/1/f32 for Samantha and
    // varies per voice, which is why this cannot be a constant.
    let format = job.format();
    if let Some(slot) = current.lock().unwrap().as_mut() {
        slot.stopper = Some(job.stopper());
    }
    let mut resample = Resample::new(format)?;
    let chunks = job.chunks();
    let Some((_, producer)) = producers.iter_mut().find(|(l, _)| *l == lane) else {
        return Err("no_lane".into());
    };
    lanes.set_active(lane, true);
    let mut started = false;
    let mut cut = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            cut = true;
            break;
        }
        match chunks.recv_timeout(Duration::from_millis(250)) {
            Ok(TtsChunk::Pcm(pcm)) => {
                let out = resample.push(&pcm);
                if !out.is_empty() {
                    if !started {
                        started = true;
                        let _ = reports.send(TtsReport::Started {
                            utterance: utterance.to_string(),
                            lane,
                        });
                    }
                    if !sink(producer, out, lane, cancel) {
                        cut = true;
                        break;
                    }
                }
            }
            Ok(TtsChunk::End) => {
                let tail = resample.flush();
                if !tail.is_empty() && !sink(producer, &tail, lane, cancel) {
                    cut = true;
                }
                break;
            }
            Ok(TtsChunk::Error(message)) => return Err(message),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(cut)
}

/// The blocking sink (`deckd-hid-write`'s shape): push what fits, wait for
/// room, and abandon the item if the room never comes.
///
/// Returns `false` when it was cut or abandoned.
fn sink(
    producer: &mut rtrb::Producer<f32>,
    mut pcm: &[f32],
    lane: Lane,
    cancel: &Arc<AtomicBool>,
) -> bool {
    let deadline = Instant::now() + stall_budget(lane);
    while !pcm.is_empty() {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        let room = producer.slots();
        if room == 0 {
            if Instant::now() >= deadline {
                // Nothing is draining this lane — no stream is open, or a
                // PTT hold has lasted longer than the audio is worth.
                return false;
            }
            std::thread::sleep(SINK_POLL);
            continue;
        }
        let take = room.min(pcm.len());
        let (head, rest) = pcm.split_at(take);
        match producer.write_chunk_uninit(take) {
            Ok(chunk) => {
                chunk.fill_from_iter(head.iter().copied());
            }
            Err(_) => continue,
        }
        pcm = rest;
    }
    true
}

/// One engine format → the lanes' interleaved stereo 44.1 kHz.
///
/// Allocation-free per chunk after construction: the buffers are sized once
/// and `process_into_buffer` writes through `audioadapter`'s borrowed views
/// (V20 — this is rubato 5's allocation-free path, and 0.x's
/// `process_into_buffer(&[Vec<f32>], &mut [Vec<f32>])` no longer exists).
pub struct Resample {
    format: AudioFormat,
    resampler: Option<Fft<f32>>,
    pending: Vec<f32>,
    scratch: Vec<f32>,
    out: Vec<f32>,
    in_frames: usize,
    out_frames: usize,
}

impl std::fmt::Debug for Resample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resample")
            .field("format", &self.format)
            .field("in_frames", &self.in_frames)
            .field("out_frames", &self.out_frames)
            .finish_non_exhaustive()
    }
}

impl Resample {
    /// # Errors
    /// When rubato cannot build a resampler for this pair of rates.
    pub fn new(format: AudioFormat) -> Result<Resample, String> {
        let channels = usize::from(format.channels).max(1);
        if format.rate == LANE_RATE {
            // The chimes are pre-rendered at the lane rate, and a voice that
            // happens to match needs no filter at all — running one would
            // only cost quality.
            return Ok(Resample {
                format,
                resampler: None,
                pending: Vec::new(),
                scratch: Vec::new(),
                out: Vec::new(),
                in_frames: 0,
                out_frames: 0,
            });
        }
        let resampler = Fft::<f32>::new(
            format.rate as usize,
            LANE_RATE as usize,
            CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|e| e.to_string())?;
        let in_frames = resampler.input_frames_next();
        let out_frames = resampler.output_frames_max();
        Ok(Resample {
            format,
            pending: Vec::with_capacity(in_frames * channels * 2),
            scratch: vec![0.0; in_frames * channels],
            out: vec![0.0; out_frames * channels],
            in_frames,
            out_frames,
            resampler: Some(resampler),
        })
    }

    /// rubato's own startup delay, in output frames (V20 measured 256 for
    /// 22050→44100). Nothing trims it: 5.8 ms of leading silence in front of
    /// an announcement is inaudible, and trimming would cost a copy.
    #[must_use]
    pub fn delay(&self) -> usize {
        self.resampler.as_ref().map_or(0, Resampler::output_delay)
    }

    #[must_use]
    pub fn input_frames(&self) -> usize {
        self.in_frames
    }

    #[must_use]
    pub fn output_frames(&self) -> usize {
        self.out_frames
    }

    /// Feed one engine chunk; get back whatever is ready, as **interleaved
    /// stereo at the lane rate**.
    pub fn push(&mut self, pcm: &[f32]) -> &[f32] {
        let channels = usize::from(self.format.channels).max(1);
        self.scratch.clear();
        if self.resampler.is_none() {
            upmix(pcm, channels, &mut self.scratch);
            return &self.scratch;
        }
        self.pending.extend_from_slice(pcm);
        let mut produced: Vec<f32> = Vec::new();
        let need = self.in_frames * channels;
        while self.pending.len() >= need {
            let Some(resampler) = self.resampler.as_mut() else {
                break;
            };
            let input = match InterleavedSlice::new(&self.pending[..need], channels, self.in_frames)
            {
                Ok(input) => input,
                Err(_) => break,
            };
            let mut output =
                match InterleavedSlice::new_mut(&mut self.out, channels, self.out_frames) {
                    Ok(output) => output,
                    Err(_) => break,
                };
            let Ok((consumed, frames)) = resampler.process_into_buffer(&input, &mut output, None)
            else {
                break;
            };
            produced.extend_from_slice(&self.out[..frames * channels]);
            self.pending.drain(..consumed * channels);
        }
        upmix(&produced, channels, &mut self.scratch);
        &self.scratch
    }

    /// The engine's zero-length buffer arrived: pad the remainder up to one
    /// chunk and take the last of it, so the end of a sentence is not lost.
    pub fn flush(&mut self) -> Vec<f32> {
        let channels = usize::from(self.format.channels).max(1);
        if self.resampler.is_none() || self.pending.is_empty() {
            self.pending.clear();
            return Vec::new();
        }
        let need = self.in_frames * channels;
        self.pending.resize(need, 0.0);
        let tail = std::mem::take(&mut self.pending);
        let out = self.push(&tail).to_vec();
        self.pending.clear();
        out
    }
}

/// Interleaved `channels` → interleaved stereo, into `out`.
fn upmix(pcm: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => {
            out.reserve(pcm.len() * 2);
            for s in pcm {
                out.push(*s);
                out.push(*s);
            }
        }
        2 => out.extend_from_slice(pcm),
        n => {
            // Anything else: take the first two channels. No AVSpeech voice
            // does this, and guessing a downmix matrix for one that did
            // would be worse than taking the front pair.
            out.reserve(pcm.len() / n * 2);
            for frame in pcm.chunks_exact(n) {
                out.push(frame[0]);
                out.push(frame[1]);
            }
        }
    }
}

/// §7.3's pre-rendered chimes, at the lane rate and at 16 k "in case
/// anything must play during SCO" (`voice-macos.md:4`).
#[derive(Default)]
struct ChimeBank {
    question: Option<Vec<f32>>,
    ready: Option<Vec<f32>>,
}

impl ChimeBank {
    fn get(&mut self, which: Chime) -> &[f32] {
        match which {
            Chime::Question => self
                .question
                .get_or_insert_with(|| chime_pcm(Chime::Question, LANE_RATE)),
            Chime::Ready => self
                .ready
                .get_or_insert_with(|| chime_pcm(Chime::Ready, LANE_RATE)),
        }
    }
}

/// Render a chime as interleaved stereo f32 at `rate`.
///
/// Quiet by construction (peak 0.2) and short (≤240 ms): §7's rule is that
/// nothing plays at full gain into a headset someone may be wearing, and a
/// chime is the one thing that plays without anybody asking for it.
#[must_use]
pub fn chime_pcm(which: Chime, rate: u32) -> Vec<f32> {
    let tones: &[(f32, f32)] = match which {
        // "two-tone rising": a fifth up, 90 ms each.
        Chime::Question => &[(660.0, 0.09), (990.0, 0.12)],
        // "single soft knock": one low, short thump.
        Chime::Ready => &[(220.0, 0.11)],
    };
    let mut pcm = Vec::new();
    for (freq, seconds) in tones {
        let frames = (f64::from(rate) * f64::from(*seconds)) as usize;
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / rate as f32;
            // A raised-cosine envelope over the whole tone: no click at
            // either end, and no need for a separate attack and release.
            #[allow(clippy::cast_precision_loss)]
            let phase = i as f32 / frames as f32;
            let env = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
            let s = 0.2 * env * (std::f32::consts::TAU * freq * t).sin();
            pcm.push(s);
            pcm.push(s);
        }
    }
    pcm
}

#[cfg(test)]
mod tests {
    use super::*;

    /// V20's measured numbers for the one conversion that actually happens
    /// on this machine, asserted against the library rather than trusted.
    #[test]
    fn the_resampler_matches_the_numbers_v20_measured() {
        let resample = Resample::new(AudioFormat::new(22_050, 1)).unwrap();
        assert_eq!(resample.input_frames(), 1024, "V20: input_frames_next");
        assert_eq!(resample.output_frames(), 2048, "V20: output_frames_next");
        assert_eq!(resample.delay(), 256, "V20: output_delay");
    }

    /// Samantha's 22050/1 becomes the lanes' 44100/2, at the same level —
    /// a resampler that halved or doubled the amplitude would be an
    /// announcement at the wrong volume for ever.
    #[test]
    fn samanthas_format_becomes_the_lane_format_at_the_same_level() {
        let mut resample = Resample::new(AudioFormat::new(22_050, 1)).unwrap();
        let mut out = Vec::new();
        for _ in 0..8 {
            out.extend_from_slice(resample.push(&[0.5f32; 1024]));
        }
        out.extend_from_slice(&resample.flush());
        assert!(!out.is_empty());
        assert_eq!(out.len() % 2, 0, "interleaved stereo");
        // Both channels carry the same mono signal.
        for pair in out.as_chunks::<2>().0 {
            assert!((pair[0] - pair[1]).abs() < 1e-6);
        }
        // Past the 256-frame startup delay the level is the input's.
        let settled = &out[1024..];
        let peak = settled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            (peak - 0.5).abs() < 0.02,
            "22050→44100 must not change the level: peak {peak}"
        );
        // Roughly twice as many frames out as in, minus the delay.
        let in_frames = 8 * 1024;
        let out_frames = out.len() / 2;
        assert!(
            out_frames > in_frames * 2 - 1024 && out_frames <= in_frames * 2 + 2048,
            "{out_frames} frames out of {in_frames} in"
        );
    }

    /// A voice already at the lane rate skips the filter entirely.
    #[test]
    fn a_voice_at_the_lane_rate_is_not_resampled_at_all() {
        let mut resample = Resample::new(AudioFormat::new(44_100, 1)).unwrap();
        assert_eq!(resample.delay(), 0);
        let out = resample.push(&[0.25, -0.25]).to_vec();
        assert_eq!(
            out,
            vec![0.25, 0.25, -0.25, -0.25],
            "mono duplicated, exact"
        );
        let mut stereo = Resample::new(AudioFormat::new(44_100, 2)).unwrap();
        assert_eq!(stereo.push(&[0.1, 0.2]), &[0.1, 0.2]);
    }

    /// The thread end to end, on the fakes: one item in flight, the lane
    /// ring filling, and a barge-in that reaches a synth blocked inside the
    /// sink — which is the case `&mut self` on the job could not have
    /// served (V16) and is why `TtsStop` exists.
    #[test]
    fn a_say_fills_its_lane_and_a_barge_in_cuts_it_mid_utterance() {
        use crate::audio::fakes::FakeTts;

        let lanes = Lanes::new();
        let (engine, fake) = FakeTts::new();
        // Far more audio than the 2 s ring holds, so the sink genuinely
        // blocks and the cut has something to interrupt.
        fake.set_frames_per_char(22_050);
        let (tx, reports) = crossbeam_channel::unbounded();
        let tts = spawn(Arc::new(engine), lanes.clone(), tx);

        assert!(tts.send(TtsCmd::Say {
            utterance: "ut_1".into(),
            lane: Lane::Question,
            text: "bot needs a decision".into(),
            voice: Some("fake.voice1".into()),
        }));
        let started = reports
            .recv_timeout(Duration::from_secs(5))
            .expect("the engine reports the first queued sample");
        assert_eq!(
            started,
            TtsReport::Started {
                utterance: "ut_1".into(),
                lane: Lane::Question
            },
            "B6: this is what becomes voice.utterance{{phase:\"started\"}}"
        );
        assert_eq!(fake.said()[0].voice.as_deref(), Some("fake.voice1"));

        // It is filling the right lane, and only that one.
        let mut waited = 0;
        while lanes.queued(Lane::Question) == 0 && waited < 200 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        assert!(lanes.queued(Lane::Question) > 0, "the ring is filling");
        assert_eq!(lanes.queued(Lane::Ready), 0);
        assert_eq!(tts.speaking().as_deref(), Some("ut_1"));

        // `ptt act:1` → `voice.stop all`, from another thread, while this
        // one is asleep inside the sink waiting for room.
        assert!(tts.cut(true, None, None));
        assert_eq!(fake.stops(), 1, "the engine is told, not just the loop");
        let ended = reports
            .recv_timeout(Duration::from_secs(5))
            .expect("and the cut ends the utterance");
        assert_eq!(
            ended,
            TtsReport::Ended {
                utterance: "ut_1".into(),
                lane: Lane::Question,
                cut: true
            }
        );
        assert!(!lanes.active(Lane::Question), "and the lane goes idle");
        assert!(tts.speaking().is_none());

        // A chime needs no engine at all: it is pre-rendered (§7.3).
        assert!(tts.send(TtsCmd::Chime {
            which: Chime::Ready
        }));
        let mut waited = 0;
        while lanes.queued(Lane::Chime) == 0 && waited < 200 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        assert!(lanes.queued(Lane::Chime) > 0);
        assert_eq!(fake.said().len(), 1, "no synthesis for a chime");
        tts.join();
    }

    /// §7's rule: nothing plays at full gain into a headset someone may be
    /// wearing, and a chime is the one thing that plays unasked.
    #[test]
    fn the_chimes_are_short_quiet_and_free_of_clicks() {
        for which in [Chime::Question, Chime::Ready] {
            let pcm = chime_pcm(which, LANE_RATE);
            let frames = pcm.len() / 2;
            let seconds = frames as f32 / LANE_RATE as f32;
            assert!(seconds > 0.05 && seconds <= 0.4, "{which:?}: {seconds} s");
            let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak <= 0.2001, "{which:?}: peak {peak}");
            assert!(pcm[0].abs() < 1e-6, "{which:?} starts at zero: no click");
            assert!(
                pcm[pcm.len() - 1].abs() < 1e-3,
                "{which:?} ends at zero: no click"
            );
        }
        // "two-tone rising": the second tone is higher than the first.
        let q = chime_pcm(Chime::Question, LANE_RATE);
        let r = chime_pcm(Chime::Ready, LANE_RATE);
        assert!(q.len() > r.len(), "the question chime is the two-tone one");
        // §7.3 pre-renders each chime at 44.1 k **and** at 16 k, "in case
        // anything must play during SCO": same sound, fewer samples.
        let sco = chime_pcm(Chime::Ready, 16_000);
        let ratio = sco.len() as f64 / r.len() as f64;
        assert!(
            (ratio - 16_000.0 / 44_100.0).abs() < 0.001,
            "the 16 k chime is the same duration: ratio {ratio}"
        );
        assert_eq!(Chime::from_name("question"), Some(Chime::Question));
        assert_eq!(Chime::from_name("boing"), None);
    }
}
