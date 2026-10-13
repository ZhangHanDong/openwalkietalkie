//! `Arc<Lanes>` — §7.2's consumer hand-off and §7.3's gains, states and TTLs.
//!
//! **Why the hand-off exists, corrected (V10).** §7.2 justifies it with
//! "cpal's coreaudio `Stream` is `!Send`". That is false for cpal 0.18.2:
//! `assert_stream_send!`/`assert_stream_sync!` are asserted in the backend
//! itself and a probe compiled them green. The hand-off stays anyway, and
//! the real reason is one line of `DeviceTrait::build_output_stream`:
//!
//! ```text
//! D: FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static
//! ```
//!
//! The consumers live **inside** that closure, the closure dies with the
//! stream, and §7.2 re-opens the stream on every PTT press and every rate
//! flip. A closure that owned its `rtrb::Consumer`s outright would destroy
//! them while `deckd-tts` still held the matching `Producer`s, and the next
//! stream would have nothing to read: the Question the operator has not
//! heard yet would be gone, once per press. So each consumer sits in a
//! `Mutex<Option<_>>` that the **next** closure takes at build time and a
//! `Drop` guard puts back.
//!
//! Every `Mutex` here is touched on `deckd-audio` at stream-build time and
//! **never in the callback** (§4).
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// §7.3's lane ring: 2 s of interleaved stereo f32 at 44.1 kHz.
pub const RING_FRAMES: usize = 2 * crate::audio::LANE_RATE as usize;
/// …in samples, which is what `rtrb` counts.
pub const RING_SAMPLES: usize = RING_FRAMES * 2;

/// The duck floor of §7.3: −18 dB.
pub const DUCK_GAIN: f32 = 0.125_892_54; // 10^(-18/20)
/// §7.3's cut fade. V18 measured the callback at 512 frames / 11.61 ms, so
/// this spans one callback and a bit — and §15's row 4 asks for −60 dB
/// within 35 ms, which is three.
pub const CUT_MS: f32 = 10.0;
/// §7.3's duck ramp.
pub const DUCK_MS: f32 = 120.0;

/// §7.3's lanes, **highest priority first**.
///
/// `Ptt` is not here: it is virtual (a flag on [`Lanes`]), because "while a
/// PTT session is live everything is cut" is a state and not a source of
/// audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    /// Pre-rendered chimes. Plays *over* the others and ducks them.
    Chime = 0,
    Question = 1,
    Ready = 2,
    Chatter = 3,
}

pub const LANE_COUNT: usize = 4;

impl Lane {
    pub const ALL: [Lane; LANE_COUNT] = [Lane::Chime, Lane::Question, Lane::Ready, Lane::Chatter];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Lane::Chime => "chime",
            Lane::Question => "question",
            Lane::Ready => "ready",
            Lane::Chatter => "chatter",
        }
    }

    /// The wire name (`voice.say{lane}`, `AudioCmd::Say{lane}`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Lane> {
        Lane::ALL.into_iter().find(|l| l.name() == name)
    }

    /// §7.3's TTLs: `Question`/`Ready` 60 s, `Chatter` 20 s, and a chime is
    /// ≤400 ms of audio that is never worth holding.
    #[must_use]
    pub const fn ttl(self) -> Duration {
        match self {
            Lane::Chime => Duration::from_millis(400),
            Lane::Question | Lane::Ready => Duration::from_secs(60),
            Lane::Chatter => Duration::from_secs(20),
        }
    }
}

/// §7.3's perceptual curve for `set speaker` and `set mic` (0..=10).
///
/// 0 is an exact mute and 10 is exact unity; the nine steps between are
/// equal in **decibels**, spanning 40 dB. A linear `v/10` would spend half
/// the knob's travel inside the top 6 dB, so the bottom half of the deck's
/// dial would feel dead — which is the complaint a "volume is just a
/// multiplier" mixer always earns.
#[must_use]
pub fn perceptual_gain(step: u16) -> f32 {
    match step {
        0 => 0.0,
        s if s >= 10 => 1.0,
        s => {
            let db = -40.0 * (10.0 - f32::from(s)) / 9.0;
            10f32.powf(db / 20.0)
        }
    }
}

fn store_f32(cell: &AtomicU32, value: f32) {
    cell.store(value.to_bits(), Ordering::Relaxed);
}

fn load_f32(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Ordering::Relaxed))
}

#[derive(Debug)]
struct LaneRing {
    /// Taken once by `deckd-tts`; never by the render callback.
    producer: Mutex<Option<rtrb::Producer<f32>>>,
    /// Taken by each render closure at build time, put back by its `Drop`.
    consumer: Mutex<Option<rtrb::Consumer<f32>>>,
    /// Per-lane gain, f32 bits.
    gain: AtomicU32,
    /// Set by the control loop, honoured (and cleared) by the callback:
    /// discard everything in the ring. A TTL expiry and `voice.stop{lane}`
    /// are the two things that set it.
    flush: AtomicBool,
    /// Millis after [`Lanes::base`] at which this lane's audio is stale, or
    /// 0 for "no deadline".
    deadline_ms: AtomicU64,
    /// Frames the mixer actually rendered from this lane. Metrics and tests.
    played: AtomicU64,
    /// `deckd-tts` is part-way through an item on this lane.
    ///
    /// Without it, "the highest non-empty lane wins" hands the floor to
    /// `Ready` the moment a `Question` momentarily underruns — the ring is
    /// 2 s deep and the synth refills it in bursts, so that is a routine
    /// event and not a pathological one. An active lane that has run dry
    /// renders silence and keeps the floor.
    active: AtomicBool,
}

impl LaneRing {
    fn new() -> Self {
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_SAMPLES);
        LaneRing {
            producer: Mutex::new(Some(producer)),
            consumer: Mutex::new(Some(consumer)),
            gain: AtomicU32::new(1.0f32.to_bits()),
            flush: AtomicBool::new(false),
            deadline_ms: AtomicU64::new(0),
            played: AtomicU64::new(0),
            active: AtomicBool::new(false),
        }
    }
}

/// The lanes, shared by `deckd-audio`, `deckd-tts` and whichever render
/// closure currently exists.
#[derive(Debug)]
pub struct Lanes {
    rings: [LaneRing; LANE_COUNT],
    /// `set speaker` on §7.3's curve.
    master: AtomicU32,
    /// `set mic` on the same curve; the capture path reads it (§7.1).
    mic: AtomicU32,
    /// §7.3's virtual `Ptt` lane: a session is live, everything is cut.
    ptt: AtomicBool,
    /// Zero for the deadlines, so they fit an `AtomicU64` of millis.
    base: Instant,
}

impl Lanes {
    #[must_use]
    pub fn new() -> Arc<Lanes> {
        Arc::new(Lanes {
            rings: [
                LaneRing::new(),
                LaneRing::new(),
                LaneRing::new(),
                LaneRing::new(),
            ],
            master: AtomicU32::new(1.0f32.to_bits()),
            mic: AtomicU32::new(1.0f32.to_bits()),
            ptt: AtomicBool::new(false),
            base: Instant::now(),
        })
    }

    /// `deckd-tts` takes each producer once, at start-up.
    pub fn take_producer(&self, lane: Lane) -> Option<rtrb::Producer<f32>> {
        self.rings[lane.index()].producer.lock().unwrap().take()
    }

    /// Hand the producer back (a `deckd-tts` restart).
    pub fn put_producer(&self, lane: Lane, producer: rtrb::Producer<f32>) {
        *self.rings[lane.index()].producer.lock().unwrap() = Some(producer);
    }

    /// Room in a lane's ring, in samples. `deckd-tts` paces on it.
    #[must_use]
    pub fn room(&self, lane: Lane) -> usize {
        self.rings[lane.index()]
            .producer
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, rtrb::Producer::slots)
    }

    /// Samples waiting in a lane. Not for the callback — it has the
    /// consumer — but for the control loop and for tests.
    #[must_use]
    pub fn queued(&self, lane: Lane) -> usize {
        self.rings[lane.index()]
            .consumer
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, rtrb::Consumer::slots)
    }

    /// Frames this lane has actually been rendered from.
    #[must_use]
    pub fn played(&self, lane: Lane) -> u64 {
        self.rings[lane.index()].played.load(Ordering::Relaxed)
    }

    /// `deckd-tts` is filling this lane (see [`LaneRing::active`]).
    pub fn set_active(&self, lane: Lane, active: bool) {
        self.rings[lane.index()]
            .active
            .store(active, Ordering::Relaxed);
    }

    #[must_use]
    pub fn active(&self, lane: Lane) -> bool {
        self.rings[lane.index()].active.load(Ordering::Relaxed)
    }

    pub fn set_gain(&self, lane: Lane, gain: f32) {
        store_f32(&self.rings[lane.index()].gain, gain);
    }

    #[must_use]
    pub fn gain(&self, lane: Lane) -> f32 {
        load_f32(&self.rings[lane.index()].gain)
    }

    /// `set speaker` (§6.4, B4) — the value straight off the deck's knob.
    pub fn set_master_step(&self, step: u16) {
        store_f32(&self.master, perceptual_gain(step));
    }

    #[must_use]
    pub fn master(&self) -> f32 {
        load_f32(&self.master)
    }

    /// `set mic` (§7.1): the capture path scales samples by this before the
    /// recogniser sees them.
    pub fn set_mic_step(&self, step: u16) {
        store_f32(&self.mic, perceptual_gain(step));
    }

    #[must_use]
    pub fn mic(&self) -> f32 {
        load_f32(&self.mic)
    }

    /// §7.3's virtual `Ptt` lane. Set on `ptt act:1` before anything else.
    pub fn set_ptt(&self, live: bool) {
        self.ptt.store(live, Ordering::Relaxed);
    }

    #[must_use]
    pub fn ptt(&self) -> bool {
        self.ptt.load(Ordering::Relaxed)
    }

    /// Throw away the audio this lane holds **now**.
    ///
    /// "Now" is the whole contract, and it used to be a lie. The flag is
    /// only ever cleared by a render callback, and there is not always one:
    /// `Store::start_listen`'s barge-in and `Engine::listen`'s close land in
    /// the same core turn, `voice.stop` arrives while the route is
    /// `ScoOurs`/`ScoExternal` and the output is shut, a `Chime` TTL expires
    /// during a PTT hold. A flag raised in any of those survived until the
    /// **next** stream — where the first callback threw away whatever had
    /// been queued since, which is by definition something else. Measured:
    /// with no render quantum between the `Stop` and the close, 44 100
    /// queued samples of a held Question were discarded and the operator
    /// heard a 10 ms blip; with one quantum in between the same audio
    /// survived. Same press, two outcomes, decided by an 11.61 ms race.
    ///
    /// So the flush is resolved at the moment it is raised whenever it can
    /// be — the consumer is in its `Mutex` exactly when no render closure
    /// holds it — and the flag is only for the case where a live callback
    /// is the one that has to do it. [`Mixer::drop`] closes the remaining
    /// window. Either way a flush can only ever discard audio that existed
    /// when it was raised.
    pub fn flush(&self, lane: Lane) {
        self.rings[lane.index()]
            .deadline_ms
            .store(0, Ordering::Relaxed);
        // Under the consumer's own `Mutex`, so this and `take_mixer` cannot
        // interleave: either we drain it here, or a mixer holds it and will.
        let mut guard = self.rings[lane.index()].consumer.lock().unwrap();
        if let Some(consumer) = guard.as_mut() {
            drain_ring(consumer);
            self.rings[lane.index()]
                .flush
                .store(false, Ordering::Relaxed);
            return;
        }
        self.rings[lane.index()]
            .flush
            .store(true, Ordering::Relaxed);
    }

    /// Drain a lane here and now, flag and all. Only valid while no render
    /// closure holds the consumer — it takes the `Mutex`, so it is a
    /// control-loop call.
    pub fn drain(&self, lane: Lane) {
        let mut guard = self.rings[lane.index()].consumer.lock().unwrap();
        if let Some(consumer) = guard.as_mut() {
            drain_ring(consumer);
            // The flag goes with the audio. Leaving it set was the bug
            // above wearing a different hat: the ring is empty, so the
            // only thing the flag can still reach is the next item.
            self.rings[lane.index()]
                .flush
                .store(false, Ordering::Relaxed);
        }
    }

    /// §7.3's TTL for whatever was just queued on `lane`.
    pub fn set_deadline(&self, lane: Lane, at: Instant) {
        let ms = at.saturating_duration_since(self.base).as_millis();
        self.rings[lane.index()].deadline_ms.store(
            u64::try_from(ms).unwrap_or(u64::MAX).max(1),
            Ordering::Relaxed,
        );
    }

    /// The default TTL of the lane, from now.
    pub fn arm_ttl(&self, lane: Lane, now: Instant) {
        self.set_deadline(lane, now + lane.ttl());
    }

    pub fn clear_deadline(&self, lane: Lane) {
        self.rings[lane.index()]
            .deadline_ms
            .store(0, Ordering::Relaxed);
    }

    /// Lanes whose TTL has run out, flushed. `deckd-audio` calls this on its
    /// 50 ms tick; the render callback does the discarding.
    pub fn expire(&self, now: Instant) -> Vec<Lane> {
        let now_ms =
            u64::try_from(now.saturating_duration_since(self.base).as_millis()).unwrap_or(u64::MAX);
        let mut expired = Vec::new();
        for lane in Lane::ALL {
            let deadline = self.rings[lane.index()].deadline_ms.load(Ordering::Relaxed);
            if deadline != 0 && now_ms >= deadline {
                self.flush(lane);
                expired.push(lane);
            }
        }
        expired
    }

    /// Take every consumer out of its `Mutex` for a new render closure
    /// (§7.2). On `deckd-audio`, at stream-build time, never in a callback.
    ///
    /// `None` when a live [`Mixer`] still holds them: the route SM has to
    /// drop the old stream before it builds the new one, and silently
    /// building a second closure with half the lanes would be far worse
    /// than refusing.
    #[must_use]
    pub fn take_mixer(self: &Arc<Self>) -> Option<Mixer> {
        self.take_mixer_at(crate::audio::LANE_RATE)
    }

    /// The same, for a stream that is open at `out_rate` (V3).
    ///
    /// `Reopening` opens the output at the device's **current** nominal
    /// rate, which during the SCO tail is 16000, because waiting for 44100
    /// costs 2591 ms against 48 ms on every post-PTT announcement. The
    /// lanes are always 44 100 ([`crate::audio::traits::AudioFormat::lane`])
    /// and nothing else resamples on that path — `Resample` targets the
    /// lane rate and `ChimeBank` renders at it — so the mixer is where the
    /// two rates meet. This is the half of V3 that was never written.
    #[must_use]
    pub fn take_mixer_at(self: &Arc<Self>, out_rate: u32) -> Option<Mixer> {
        let mut consumers: [Option<rtrb::Consumer<f32>>; LANE_COUNT] = [None, None, None, None];
        for lane in Lane::ALL {
            let taken = self.rings[lane.index()].consumer.lock().unwrap().take();
            match taken {
                Some(consumer) => consumers[lane.index()] = Some(consumer),
                None => {
                    // Put back whatever we already took: a half-taken set is
                    // the one state nothing else in here can recover from.
                    for other in Lane::ALL {
                        if let Some(c) = consumers[other.index()].take() {
                            *self.rings[other.index()].consumer.lock().unwrap() = Some(c);
                        }
                    }
                    return None;
                }
            }
        }
        Some(Mixer {
            lanes: self.clone(),
            consumers,
            // **Silence, not unity.** A brand-new closure has nothing in
            // flight to fade *from*: the previous stream's envelopes died
            // with it and the device was not being fed at all in between
            // (§7.2 closes the output for every press and every rate
            // flip). Starting at 1.0 fabricated a signal that was never
            // playing — measured, the first quantum of a fresh mixer with
            // a Question and a Ready queued peaked at **1.0494**, over
            // full scale, because the held `Ready` was drained and mixed
            // under its own fade-out; 880 frames of it were eaten, and
            // §7.3 says `Ready` is *held, not flushed*. From 0.0 the lane
            // that plays fades in over the same 10 ms (inaudible, and it
            // is the click a stream start otherwise makes), every held
            // lane stays silent and untouched, and a lane that owes a
            // discard is emptied on the first quantum with nothing audible
            // to clip.
            cut: [0.0; LANE_COUNT],
            duck: [1.0; LANE_COUNT],
            discarding: [false; LANE_COUNT],
            out_rate: out_rate.max(1),
            step: f64::from(crate::audio::LANE_RATE) / f64::from(out_rate.max(1)),
            lp_a: lowpass_coefficient(out_rate.max(1)),
            pos: 0.0,
            prev: (0.0, 0.0),
            cur: (0.0, 0.0),
            lp: (0.0, 0.0),
            primed: false,
        })
    }
}

/// The render callback's whole state.
///
/// **Nothing in [`Mixer::render`] allocates, locks or blocks** (§4, V19). It
/// reads atomics and `rtrb` consumers, and that is all; `tests/voice_mixer.rs`
/// proves the allocation half with a counting allocator rather than by
/// asking a reviewer to take it on trust.
pub struct Mixer {
    lanes: Arc<Lanes>,
    consumers: [Option<rtrb::Consumer<f32>>; LANE_COUNT],
    /// The 10 ms cut envelope per lane, 0.0..=1.0. A new closure starts
    /// every lane at **0.0** — see [`Lanes::take_mixer_at`].
    cut: [f32; LANE_COUNT],
    /// The 120 ms duck envelope per lane, [`DUCK_GAIN`]..=1.0.
    duck: [f32; LANE_COUNT],
    /// A flush was asked for and the lane has not been emptied yet. It
    /// outlives the `flush` flag on purpose: the flag is consumed in one
    /// callback, and the fade that has to run before the discard takes
    /// three.
    discarding: [bool; LANE_COUNT],

    // -- V3's rate bridge ------------------------------------------------
    /// The rate the **stream** is open at. 44 100 on A2DP, 16000 during the
    /// SCO tail and for the whole of `ScoStuck`.
    out_rate: u32,
    /// Lane frames per output frame: 2.75625 at 44 100 → 16000.
    step: f64,
    /// One-pole coefficient for the pre-decimation guard, 0 when none.
    lp_a: f32,
    /// Where the next output sample sits between `prev` and `cur`, in lane
    /// frames. Always in `[0, 1)` when an output sample is taken.
    pos: f64,
    /// The two lane-rate frames the current output sample sits between.
    prev: (f32, f32),
    cur: (f32, f32),
    /// The one-pole's state, per channel.
    lp: (f32, f32),
    /// `cur` holds a real frame rather than the initial silence.
    primed: bool,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mixer")
            .field("cut", &self.cut)
            .field("duck", &self.duck)
            .finish_non_exhaustive()
    }
}

/// What a lane is doing this quantum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Drain it into the mix.
    Play,
    /// Cut to silence and **keep** the audio: a PTT hold, or a lower lane
    /// waiting for a higher one (§7.3: "held, not flushed").
    Hold,
    /// Cut to silence and **throw the audio away**: `Chatter` under a
    /// `Question`, a TTL expiry, a `voice.stop`.
    Discard,
}

impl Mixer {
    /// The callback body, for `out` interleaved stereo frames.
    ///
    /// V18 measured the real one at 512 frames / 11.61 ms on a CoreAudio
    /// time-constraint thread, which is the quantum §14 and §15's row 4 use.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let master = self.lanes.master();
        let ptt = self.lanes.ptt();

        // One pass over the atomics, before the sample loop: the callback
        // must not re-read shared state per frame.
        let mut gain = [0.0f32; LANE_COUNT];
        let mut busy = [false; LANE_COUNT];
        for lane in Lane::ALL {
            let i = lane.index();
            gain[i] = load_f32(&self.lanes.rings[i].gain);
            let queued = self.consumers[i].as_ref().map_or(0, rtrb::Consumer::slots);
            busy[i] = queued > 0 || self.lanes.rings[i].active.load(Ordering::Relaxed);
            if self.lanes.rings[i].flush.swap(false, Ordering::Relaxed) {
                self.discarding[i] = true;
            }
        }

        // §7.3's priority: `Chime` plays over everything and ducks the rest;
        // among `Question`/`Ready`/`Chatter` only the highest busy one is
        // heard, and `Chatter` is *cut* by them rather than held.
        let chime = busy[Lane::Chime.index()] && !self.discarding[Lane::Chime.index()];
        let speaking = [Lane::Question, Lane::Ready, Lane::Chatter]
            .into_iter()
            .find(|l| busy[l.index()] && !self.discarding[l.index()]);

        let mut mode = [Mode::Hold; LANE_COUNT];
        for lane in Lane::ALL {
            let i = lane.index();
            mode[i] = if self.discarding[i] {
                Mode::Discard
            } else if ptt {
                // Everything is cut. `Question`/`Ready` are **held** across
                // the hold (§7.3) — the operator has not heard them yet.
                Mode::Hold
            } else if Some(lane) == speaking || (lane == Lane::Chime && chime) {
                Mode::Play
            } else if lane == Lane::Chatter && speaking.is_some() {
                Mode::Discard
            } else {
                Mode::Hold
            };
        }

        // A discard is the one bulk operation, and it is still allocation-
        // free: `read_chunk` borrows the ring's own storage. It runs only
        // once the 10 ms fade has finished, so nothing is ever cut off
        // mid-sample with a click.
        for lane in Lane::ALL {
            let i = lane.index();
            if mode[i] == Mode::Discard && self.cut[i] <= 0.0 {
                if let Some(consumer) = self.consumers[i].as_mut() {
                    drain_ring(consumer);
                }
                self.discarding[i] = false;
            }
        }

        let cut_step = 1.0 / (CUT_MS * crate::audio::LANE_RATE as f32 / 1000.0);
        let duck_step = (1.0 - DUCK_GAIN) / (DUCK_MS * crate::audio::LANE_RATE as f32 / 1000.0);

        let mut played = [0u64; LANE_COUNT];
        let env = Envelopes {
            mode,
            gain,
            chime,
            cut_step,
            duck_step,
        };
        if self.out_rate == crate::audio::LANE_RATE {
            // The A2DP path: the lanes are already at the stream's rate.
            for frame in 0..frames {
                let (l, r) = self.mix_frame(&env, &mut played);
                out[frame * 2] = l * master;
                out[frame * 2 + 1] = r * master;
            }
        } else {
            // V3's other half. The stream is open at whatever the device
            // reports — 16000 through the SCO tail and the whole of
            // `ScoStuck` — and the lanes are always 44 100, so the mix bus
            // is resampled here. Popping 1:1 into a 16 kHz stream played
            // every announcement at 0.363× speed and ~16 semitones flat.
            //
            // Linear interpolation, deliberately: it is branch-light,
            // allocation-free and stateless but for four floats, which is
            // what §4 and V19 require of this function. See `lp_a` for what
            // stands in for a decimation filter.
            for frame in 0..frames {
                if !self.primed {
                    self.cur = self.mix_frame(&env, &mut played);
                    self.prev = self.cur;
                    self.primed = true;
                }
                while self.pos >= 1.0 {
                    self.prev = self.cur;
                    self.cur = self.mix_frame(&env, &mut played);
                    self.pos -= 1.0;
                }
                #[allow(clippy::cast_possible_truncation)]
                let f = self.pos as f32;
                let l = self.prev.0 + (self.cur.0 - self.prev.0) * f;
                let r = self.prev.1 + (self.cur.1 - self.prev.1) * f;
                self.lp.0 += self.lp_a * (l - self.lp.0);
                self.lp.1 += self.lp_a * (r - self.lp.1);
                out[frame * 2] = self.lp.0 * master;
                out[frame * 2 + 1] = self.lp.1 * master;
                self.pos += self.step;
            }
        }

        for lane in Lane::ALL {
            let i = lane.index();
            if played[i] > 0 {
                self.lanes.rings[i]
                    .played
                    .fetch_add(played[i], Ordering::Relaxed);
            }
        }
    }

    /// One **lane-rate** frame of the mix: §7.3's two envelopes stepped,
    /// every playing lane popped, summed.
    ///
    /// Split out of [`Mixer::render`] so the resampled path can ask for
    /// input frames at its own pace without the envelopes noticing: a
    /// 10 ms cut is 10 ms of lane audio whatever rate the stream is open
    /// at, which is the only definition that survives a rate flip.
    #[inline]
    fn mix_frame(&mut self, env: &Envelopes, played: &mut [u64; LANE_COUNT]) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for lane in Lane::ALL {
            let i = lane.index();
            // The two envelopes of §7.3, stepped per frame so a 10 ms
            // fade is a 10 ms fade and not a per-callback staircase.
            let cut_target = if env.mode[i] == Mode::Play { 1.0 } else { 0.0 };
            self.cut[i] = step_toward(self.cut[i], cut_target, env.cut_step);
            let duck_target = if env.chime && lane != Lane::Chime {
                DUCK_GAIN
            } else {
                1.0
            };
            self.duck[i] = step_toward(self.duck[i], duck_target, env.duck_step);

            let gain = self.cut[i] * self.duck[i] * env.gain[i];
            // A lane that is fading out is still drained, so the audio
            // under the fade is the audio that was playing. A lane that
            // has reached zero and is held is not: that is the hold.
            let drain = env.mode[i] == Mode::Play || self.cut[i] > 0.0;
            if !drain {
                continue;
            }
            if let Some(consumer) = self.consumers[i].as_mut() {
                let l = consumer.pop().unwrap_or(0.0);
                let r = consumer.pop().unwrap_or(0.0);
                left += l * gain;
                right += r * gain;
                played[i] += 1;
            }
        }
        (left, right)
    }

    /// The lanes this mixer drains, for the stream that owns it.
    #[must_use]
    pub fn lanes(&self) -> &Arc<Lanes> {
        &self.lanes
    }

    /// The rate the stream this mixer feeds is open at.
    #[must_use]
    pub const fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// Samples still queued on a lane, **while this mixer holds the
    /// consumer**.
    ///
    /// [`Lanes::queued`] answers the same question for the control loop,
    /// but only while no stream is open: it reads through the `Mutex` the
    /// consumer was taken out of, so with a live closure it would answer 0
    /// for a full ring. Both exist because both moments exist.
    #[must_use]
    pub fn queued(&self, lane: Lane) -> usize {
        self.consumers[lane.index()]
            .as_ref()
            .map_or(0, rtrb::Consumer::slots)
    }
}

/// What every lane's per-frame envelope needs, read once per callback.
///
/// A struct rather than five arguments because it is copied out of the
/// atomics at the top of [`Mixer::render`] and must not be re-read per
/// frame — and because the resampled path calls `mix_frame` two or three
/// times per output frame.
struct Envelopes {
    mode: [Mode; LANE_COUNT],
    gain: [f32; LANE_COUNT],
    chime: bool,
    cut_step: f32,
    duck_step: f32,
}

/// A one-pole guard ahead of the decimation, or **1.0** — a pass-through —
/// when the stream's rate is at or above the lane rate and there is nothing
/// to fold.
///
/// **Not a brick wall, and it does not pretend to be.** A proper decimation
/// filter in a CoreAudio time-constraint callback would mean rubato's
/// fixed-chunk machinery inside the closure, and the chunk sizes do not
/// divide a 512-frame quantum at a ratio of 2.75625. What actually reaches
/// these lanes is AVSpeech at 22 050 Hz (band-limited to 11 kHz by its own
/// synthesiser) and chimes whose partials stop at 990 Hz, so the energy
/// that can fold past 8 kHz is sibilance and little else. One pole at
/// 0.42 × the output rate takes the top off that; the residue is an
/// accepted approximation, and it is named here so the next reader does not
/// have to measure it to find out.
#[allow(clippy::cast_precision_loss)]
fn lowpass_coefficient(out_rate: u32) -> f32 {
    if out_rate >= crate::audio::LANE_RATE {
        // `y += 1.0 * (x - y)` is `y = x`: no state, no colour.
        return 1.0;
    }
    let fc = 0.42 * out_rate as f32;
    1.0 - (-std::f32::consts::TAU * fc / crate::audio::LANE_RATE as f32).exp()
}

/// Empty an `rtrb` consumer. Allocation-free — `read_chunk` borrows the
/// ring's own storage — which is why the render callback may call it too.
#[inline]
fn drain_ring(consumer: &mut rtrb::Consumer<f32>) {
    let slots = consumer.slots();
    if slots > 0 {
        if let Ok(chunk) = consumer.read_chunk(slots) {
            chunk.commit_all();
        }
    }
}

#[inline]
fn step_toward(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else if current > target {
        (current - step).max(target)
    } else {
        target
    }
}

/// §7.2's `Drop` guard: the consumers go back into their `Mutex`es so the
/// **next** stream reads the same rings, and the Question the operator has
/// not heard yet survives the PTT hold that tore the stream down.
impl Drop for Mixer {
    fn drop(&mut self) {
        for lane in Lane::ALL {
            let i = lane.index();
            if let Some(mut consumer) = self.consumers[i].take() {
                // A flush this closure was asked for and never got to run —
                // the flag raised between two quanta, or a discard still
                // part-way through its 10 ms fade — is honoured here, at the
                // first moment anything else can. Carrying it over to the
                // next stream would apply it to the next item on the lane
                // instead of to the audio it was raised against.
                let pending = self.lanes.rings[i].flush.swap(false, Ordering::Relaxed);
                if pending || self.discarding[i] {
                    drain_ring(&mut consumer);
                    self.discarding[i] = false;
                }
                *self.lanes.rings[i].consumer.lock().unwrap() = Some(consumer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_master_curve_is_a_mute_a_unity_and_forty_decibels_between() {
        assert_eq!(perceptual_gain(0), 0.0, "0 is an exact mute, not −40 dB");
        assert_eq!(perceptual_gain(10), 1.0);
        assert_eq!(perceptual_gain(11), 1.0, "clamped, never louder than unity");
        let one = perceptual_gain(1);
        assert!(
            (20.0 * one.log10() + 40.0).abs() < 0.01,
            "step 1 is −40 dB, got {} dB",
            20.0 * one.log10()
        );
        // Monotonic, and equal in dB rather than in amplitude: the ratio
        // between neighbours is the same all the way up, which is what
        // makes the bottom half of the knob usable.
        let ratios: Vec<f32> = (1..10)
            .map(|s| perceptual_gain(s + 1) / perceptual_gain(s))
            .collect();
        for r in &ratios {
            assert!(
                (r - ratios[0]).abs() < 1e-4,
                "{ratios:?} are not equal steps"
            );
            assert!(*r > 1.0);
        }
    }

    #[test]
    fn a_lane_name_round_trips_and_ptt_is_not_one_of_them() {
        for lane in Lane::ALL {
            assert_eq!(Lane::from_name(lane.name()), Some(lane));
        }
        assert_eq!(
            Lane::from_name("ptt"),
            None,
            "§7.3's Ptt lane is virtual: a caller may not name it"
        );
        assert_eq!(Lane::Chime.ttl(), Duration::from_millis(400));
        assert_eq!(Lane::Chatter.ttl(), Duration::from_secs(20));
        assert_eq!(Lane::Question.ttl(), Duration::from_secs(60));
    }

    /// The half-taken set is the state nothing can recover from, so a
    /// refused take must leave every lane exactly where it was.
    #[test]
    fn a_second_mixer_is_refused_and_leaves_the_lanes_whole() {
        let lanes = Lanes::new();
        let first = lanes.take_mixer().expect("the first take succeeds");
        assert!(
            lanes.take_mixer().is_none(),
            "two live mixers would each hold half the lanes"
        );
        drop(first);
        let second = lanes.take_mixer();
        assert!(second.is_some(), "and the refusal put nothing at risk");
    }
}
