//! §15 week 4, exit rows 1 and 2 (HOST-CONTRACT-W4 §5): the whole §7.4 PTT
//! session machine, driven headlessly through `VoiceIn` on a simulated
//! clock — no microphone, no deck, no TCC grant.
//!
//! The fake here is a **capture source**, not a transcript: [`FakeMic`]
//! produces 512-frame buffers (the deck's real quantum, 11.61 ms, measured
//! from inside a live render callback) and turns them into exactly the
//! `VoiceIn` an engine would send — a `Level` per buffer, and `Live` on the
//! first sample that is not an exact digital zero, decided by the engine's
//! own [`deckd::audio::buffer_is_live`] rather than by the rig. That is the
//! only way to
//! write row 2 honestly, because row 2 is about the difference between
//! those two signals:
//!
//! > SCO bring-up measured **1031 ms median** (815–1246, n=16, 2026-09-21)
//! > from start-IO to the first non-zero sample, while the input callback
//! > starts at ~28 ms and `AudioUnitRender` returns `noErr` the whole time.
//! > Every sample in that first second is an exact zero.
//!
//! So a dead microphone and a warming one look **identical** for a second,
//! and §7.4's "1.5 s without samples → mic unavailable" keyed off buffers
//! could never fire on a dead mic at all: the buffers arrive. A fake that
//! only refused to send a transcript would pass a buffer-counting
//! implementation and prove nothing.

mod core_support;

use core_support::{agent_of, build, poll_register, uid, Parts};
use deck_model::{
    AgentId, CardId, CardKind, CardSet, Choice, Command, Deliver, DeviceLine, DeviceOp, Event,
    EventKind, Origin, Permission, Permissions, PttCtx, Request, SessionId, VoiceIn,
};
use deckd::audio::{AudioHandle, FromAudio};
use deckd::core::Core;
use std::time::{Duration, Instant};

/// The deck's own render quantum: 512 frames at 44.1 kHz, measured from
/// inside a live CoreAudio time-constraint callback (11.610 ms period).
const QUANTUM: Duration = Duration::from_micros(11_610);

/// …and its 512 frames.
const FRAMES: usize = 512;

/// A capture source that delivers buffers, which is not the same thing as
/// delivering audio.
struct FakeMic {
    /// How long the link stays up before the first non-zero sample, or
    /// `None` for a microphone that never produces one — a dead mic, a
    /// denied grant, a headset that dropped its SCO link.
    speech_after: Option<Duration>,
    open_at: Instant,
    /// **Not the rig's opinion.** `deckd::audio::Liveness` is the engine's
    /// own `Arming → Listening` edge, applied here to the samples this
    /// callback actually carries — so the rig can no longer paper over an
    /// engine that never sends `VoiceIn::Live` (B1: for a week, nothing
    /// but tests ever constructed one).
    liveness: deckd::audio::Liveness,
}

impl FakeMic {
    /// A mic that comes up the way the deck really does.
    fn warm(open_at: Instant) -> FakeMic {
        FakeMic {
            // The measured median, to the first non-zero sample.
            speech_after: Some(Duration::from_millis(1031)),
            open_at,
            liveness: deckd::audio::Liveness::new(),
        }
    }

    /// A mic that delivers callbacks full of digital silence for ever.
    fn dead(open_at: Instant) -> FakeMic {
        FakeMic {
            speech_after: None,
            open_at,
            liveness: deckd::audio::Liveness::new(),
        }
    }

    /// The samples one 512-frame callback carries at `now`.
    ///
    /// Both mics deliver a buffer every quantum and both report `noErr`;
    /// the only difference is whether any sample in it is non-zero.
    fn buffer(&self, now: Instant) -> Vec<f32> {
        let speaking = self
            .speech_after
            .is_some_and(|d| now.duration_since(self.open_at) >= d);
        vec![if speaking { 0.21 } else { 0.0 }; FRAMES]
    }

    /// The `VoiceIn` that buffer produces, by the **engine's** rules and
    /// not by the rig's: [`deckd::audio::buffer_is_live`] decides the
    /// liveness edge and the samples decide the meter.
    fn callback(&mut self, now: Instant) -> Vec<VoiceIn> {
        let buffer = self.buffer(now);
        let energy: f32 = buffer.iter().map(|s| s * s).sum();
        #[allow(clippy::cast_precision_loss)]
        let rms = (energy / buffer.len() as f32).sqrt();
        let mut out = vec![VoiceIn::Level { rms }];
        if self.liveness.observe(&buffer) {
            out.push(VoiceIn::Live);
        }
        out
    }
}

/// A detached core with the audio handle attached, so a test sees the
/// `AudioCmd`s the core sent and can feed `VoiceIn` back the way the engine
/// would.
struct Rig {
    core: Core,
    parts: Parts,
    now: Instant,
    events: Vec<Event>,
    _cmds: crossbeam_channel::Receiver<deck_model::AudioCmd>,
    _events: crossbeam_channel::Sender<FromAudio>,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let (mut core, parts) = build(name, 512);
        let (audio, cmds, events) = AudioHandle::detached();
        core.set_audio(audio);
        Rig {
            core,
            parts,
            now: Instant::now(),
            events: Vec::new(),
            _cmds: cmds,
            _events: events,
        }
    }

    fn agent(&mut self, name: &str) -> AgentId {
        let r = self.core.api_call(
            uid(),
            SessionId(1),
            true,
            Request::AgentRegister(poll_register(name)),
            self.now,
        );
        agent_of(&r)
    }

    fn card(&mut self, agent: &AgentId, kind: CardKind) -> CardId {
        let result = self.core.api_call(
            uid(),
            SessionId(1),
            true,
            Request::CardSet(CardSet {
                agent: agent.clone(),
                id: None,
                kind,
                text: "ship it?".into(),
                choices: vec![Choice {
                    label: "Yes".into(),
                    detail: None,
                }],
                rec: None,
                free_text: true,
                deliver: Deliver::Poll,
                timeout_s: None,
                origin: Origin::custom(),
            }),
            self.now,
        );
        core_support::card_of(&result)
    }

    fn ptt(&mut self, act: u8, ctx: PttCtx, card: Option<u16>, abort: bool) {
        self.events = self.core.on_device_line(
            DeviceLine::Ptt {
                act,
                slot: 1,
                ctx,
                card,
                abort, press: None,
            },
            self.now,
        );
    }

    /// One `VoiceIn`, through the same entry point the audio thread uses.
    fn voice(&mut self, v: VoiceIn) {
        self.events = self.core.step(Command::Voice(v), self.now);
    }

    /// Run the fake mic for `d`, a 512-frame callback at a time, ticking
    /// the core's timers as the clock moves — exactly what the daemon does.
    fn run(&mut self, mic: &mut FakeMic, d: Duration) {
        let until = self.now + d;
        while self.now < until {
            self.now += QUANTUM;
            for v in mic.callback(self.now) {
                self.core.on_from_audio(FromAudio::Voice(v), self.now);
            }
            self.core.tick(self.now);
        }
    }

    fn kinds(&self) -> Vec<EventKind> {
        self.events.iter().map(|e| e.kind).collect()
    }

    /// Every event the core raised since the rig was built, from the bus.
    fn all(&mut self) -> Vec<Event> {
        self.core.publish();
        self.core
            .bus
            .replay(0, 4096, None)
            .0
            .iter()
            .map(|e| (**e).clone())
            .collect()
    }

    fn cleanup(self) {
        let dir = self.parts.dir.clone();
        drop(self);
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn state_events(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.kind == EventKind::VoiceState)
        .filter_map(|e| e.data["state"].as_str().map(str::to_string))
        .collect()
}

/// Exit row 1: a press arms the capture, the first non-zero sample makes it
/// listen, and the release dispatches by the `ctx` the press carried.
///
/// The four `ctx` values do four unrelated things with the same sentence,
/// and §7.4 is the only place that says which — so this row walks all four
/// plus the empty final, on one core, through the real device lines.
#[test]
fn a_press_arms_captures_and_the_release_dispatches_by_ctx() {
    let mut rig = Rig::new("voice-ptt-dispatch");
    let agent = rig.agent("bot");
    let t0 = rig.now;

    // ── `talk` → `op.order{source:"ptt"}` ────────────────────────────────
    rig.ptt(1, PttCtx::Talk, None, false);
    assert_eq!(
        state_events(&rig.events),
        vec!["arming"],
        "§7.4: the press arms; nothing has been heard yet"
    );
    let mut mic = FakeMic::warm(rig.now);
    // Half the measured bring-up: buffers are arriving and every sample in
    // them is zero, so the session is still `Arming`.
    rig.run(&mut mic, Duration::from_millis(500));
    assert!(
        !rig.core.store.snapshot().voice.ptt.as_ref().unwrap().live,
        "500 ms of callbacks full of zeros is not a live microphone"
    );
    // Past the measured 1031 ms the first non-zero sample lands.
    rig.run(&mut mic, Duration::from_millis(700));
    let ptt = rig.core.store.snapshot().voice.ptt.clone().expect("live");
    assert!(ptt.live, "the first non-zero sample is what listens");
    let utterance = ptt.utterance.clone();

    rig.voice(VoiceIn::Partial {
        text: "run the".into(),
    });
    rig.ptt(0, PttCtx::Talk, None, false);
    assert_eq!(state_events(&rig.events), vec!["finalizing"]);
    rig.voice(VoiceIn::Final {
        text: "run the tests".into(),
    });
    let kinds = rig.kinds();
    assert!(kinds.contains(&EventKind::OrderDispatched), "{kinds:?}");
    let order = rig
        .events
        .iter()
        .find(|e| e.kind == EventKind::AgentOrder)
        .expect("agent.order for the owner");
    assert_eq!(order.data["text"], "run the tests");
    assert_eq!(order.data["source"], "ptt");
    assert_eq!(order.data["ctx"], "talk");
    assert_eq!(
        order.data["utterance"], utterance,
        "the order names the utterance it was dictated from"
    );
    assert_eq!(state_events(&rig.events), vec!["ended"]);

    // ── `answer` on a `q` card → `op.answer{text}` ───────────────────────
    let q = rig.card(&agent, CardKind::Q);
    rig.ptt(1, PttCtx::Answer, Some(q.0), false);
    rig.voice(VoiceIn::Live);
    rig.ptt(0, PttCtx::Answer, Some(q.0), false);
    rig.voice(VoiceIn::Final {
        text: "use the staging database".into(),
    });
    let answered = rig
        .events
        .iter()
        .find(|e| e.kind == EventKind::CardAnswered)
        .expect("card.answered");
    assert_eq!(answered.data["text"], "use the staging database");
    assert_eq!(answered.data["source"], "ptt");
    assert_eq!(
        answered.data["principal"], "device",
        "a deck press answers as the device (§4.9)"
    );
    assert_eq!(
        answered.data["choice"],
        serde_json::Value::Null,
        "free text, not a chosen option"
    );
    assert!(
        !rig.kinds().contains(&EventKind::OrderDispatched),
        "a question is answered, not ordered"
    );

    // ── `answer` on an `r` card → `op.order` ─────────────────────────────
    // The device latches the Ready id, and a release on it is a **step**:
    // §5.6 says an answer to a Ready card is an instruction to the agent.
    let r = rig.card(&agent, CardKind::R);
    rig.ptt(1, PttCtx::Answer, Some(r.0), false);
    rig.voice(VoiceIn::Live);
    rig.ptt(0, PttCtx::Answer, Some(r.0), false);
    rig.voice(VoiceIn::Final {
        text: "now deploy it".into(),
    });
    let kinds = rig.kinds();
    assert!(
        kinds.contains(&EventKind::OrderDispatched),
        "a Ready release is an order, not an answer: {kinds:?}"
    );
    assert!(
        !kinds.contains(&EventKind::CardAnswered),
        "…and not both: {kinds:?}"
    );
    let order = rig
        .events
        .iter()
        .find(|e| e.kind == EventKind::AgentOrder)
        .expect("agent.order");
    assert_eq!(order.data["ctx"], "step");
    assert_eq!(order.data["text"], "now deploy it");

    // ── `name` → `device.name`, cut to 16 B and title-cased ──────────────
    rig.parts.drain();
    rig.ptt(1, PttCtx::Name, None, false);
    rig.voice(VoiceIn::Live);
    rig.ptt(0, PttCtx::Name, None, false);
    rig.voice(VoiceIn::Final {
        text: "deploy staging now".into(),
    });
    let named = rig
        .parts
        .taken()
        .into_iter()
        .find_map(|op| match op {
            DeviceOp::Control(deck_model::Control::Name(n)) => Some(n),
            _ => None,
        })
        .expect("device.name reached the deck");
    assert_eq!(
        named, "Deploy Staging N",
        "cut to 16 bytes on a char boundary, title-cased, no ellipsis"
    );

    // ── an empty final → `voice.state{ended, empty:true}`, nothing sent ──
    rig.ptt(1, PttCtx::Talk, None, false);
    rig.voice(VoiceIn::Live);
    rig.ptt(0, PttCtx::Talk, None, false);
    rig.voice(VoiceIn::Final { text: "   ".into() });
    let ended = rig
        .events
        .iter()
        .find(|e| e.kind == EventKind::VoiceState && e.data["state"] == "ended")
        .expect("the session still ends");
    assert_eq!(
        ended.data["empty"], true,
        "pressing the button and saying nothing is normal, and says so"
    );
    let kinds = rig.kinds();
    assert!(!kinds.contains(&EventKind::OrderDispatched), "{kinds:?}");
    assert!(!kinds.contains(&EventKind::CardAnswered), "{kinds:?}");

    // Every session in this row went arming → listening → … → ended, in
    // that order and never out of it.
    let states = state_events(&rig.all());
    assert_eq!(
        states.iter().filter(|s| *s == "arming").count(),
        5,
        "{states:?}"
    );
    assert_eq!(states[0], "arming");
    assert_eq!(states[1], "listening");
    assert!(rig.now > t0);
    rig.cleanup();
}

/// Exit row 2: a microphone that delivers callbacks full of **digital
/// silence** still trips the dead-mic path.
///
/// This is the row the contract calls the subtlest correction in it, and it
/// is written to fail against the implementation §7.4 originally described.
/// A liveness check keyed on "a buffer arrived" sees 215 buffers in the
/// 2.5 s below, every one of them `noErr`, and concludes the microphone is
/// fine — which is what a genuinely dead mic looks like for its whole life,
/// and what a healthy one looks like for its first second.
#[test]
fn arming_without_samples_reports_a_dead_mic() {
    let mut rig = Rig::new("voice-ptt-dead-mic");
    let _agent = rig.agent("bot");

    // The engine reports a denied microphone grant (§7.8): silence, never
    // an error — but `perm.changed` so the GUI can say why.
    rig.voice(VoiceIn::Permissions(Permissions {
        microphone: Permission::Denied,
        speech: Permission::Granted,
        bluetooth: Permission::Unknown,
    }));
    assert_eq!(rig.kinds(), vec![EventKind::PermChanged]);
    assert_eq!(rig.events[0].data["microphone"], "denied");

    rig.parts.drain();
    rig.ptt(1, PttCtx::Talk, None, false);
    let mut mic = FakeMic::dead(rig.now);
    let before = rig.now;
    rig.run(&mut mic, Duration::from_millis(2_400));
    let buffers = (rig.now - before).as_micros() / QUANTUM.as_micros();
    assert!(
        buffers > 200,
        "{buffers} callbacks arrived, all of them noErr — a buffer count \
         cannot tell this mic from a working one"
    );
    assert!(
        rig.core.store.snapshot().voice.ptt.is_some(),
        "still arming: the deadline is above the measured worst-case \
         bring-up (1246 ms), so a slow-but-live mic is never called dead"
    );

    // Past the deadline the session ends and says why.
    rig.run(&mut mic, Duration::from_millis(400));
    assert!(
        rig.core.store.snapshot().voice.ptt.is_none(),
        "the session is over"
    );
    let all = rig.all();
    let failed = all
        .iter()
        .filter(|e| e.kind == EventKind::VoiceState)
        .find(|e| e.data["error"] == "mic unavailable")
        .expect("voice.state{error:\"mic unavailable\"}");
    assert_eq!(failed.data["state"], "aborted");
    assert_eq!(failed.data["ctx"], "talk");

    // The operator is holding the deck's button, so the deck is where the
    // answer has to appear: §7.4's `(no mic)` on the transcript line.
    let transcripts: Vec<String> = rig
        .parts
        .taken()
        .into_iter()
        .filter_map(|op| match op {
            DeviceOp::Control(deck_model::Control::Transcript { text, .. }) => Some(text),
            _ => None,
        })
        .collect();
    assert!(
        transcripts.contains(&"(no mic)".to_string()),
        "{transcripts:?}"
    );

    // A denied grant is silence and a `perm.changed`, never an error event
    // and never a panic (§7.8).
    assert!(
        all.iter().any(|e| e.kind == EventKind::PermChanged),
        "the grant state is on the bus for the Voice screen"
    );
    assert_eq!(
        rig.core.store.snapshot().voice.permissions.microphone,
        Permission::Denied
    );

    // And the contrast, on the same rig: a mic that is merely slow is not
    // a dead one. 1031 ms of digital silence, then audio — the measured
    // median — and the session lives to be dispatched.
    rig.ptt(1, PttCtx::Talk, None, false);
    let mut warm = FakeMic::warm(rig.now);
    rig.run(&mut warm, Duration::from_millis(2_400));
    let ptt = rig
        .core
        .store
        .snapshot()
        .voice
        .ptt
        .clone()
        .expect("a slow mic is not a dead one");
    assert!(ptt.live);
    rig.cleanup();
}

/// A slow final retains the last words, past the former 1.5 s cutoff.
#[test]
fn finalizing_waits_for_the_complete_transcript_instead_of_sending_a_partial() {
    let mut rig = Rig::new("voice-ptt-finalizing");
    let _agent = rig.agent("bot");
    rig.ptt(1, PttCtx::Talk, None, false);
    rig.voice(VoiceIn::Live);
    rig.voice(VoiceIn::Partial {
        text: "restart the".into(),
    });
    rig.voice(VoiceIn::Partial {
        text: "restart the build".into(),
    });
    rig.ptt(0, PttCtx::Talk, None, false);
    assert_eq!(state_events(&rig.events), vec!["finalizing"]);

    // Inside the budget, nothing is decided: the words really may still be
    // on their way.
    rig.now += Duration::from_millis(1_400);
    rig.core.tick(rig.now);
    let all = rig.all();
    assert!(
        !all.iter().any(|e| e.kind == EventKind::OrderDispatched),
        "the released recording waits for the recogniser's final"
    );

    // Past the old cutoff there is still no message to the agent.
    rig.now += Duration::from_secs(3);
    rig.core.tick(rig.now);
    assert!(!rig.all().iter().any(|e| matches!(e.kind, EventKind::OrderDispatched | EventKind::TranscriptFinal)));
    rig.voice(VoiceIn::Final {
        text: "restart the build with the cache disabled".into(),
    });
    let all = rig.all();
    let orders: Vec<_> = all.iter().filter(|e| e.kind == EventKind::OrderDispatched).collect();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].data["text"], "restart the build with the cache disabled");
    assert!(all.iter().any(|e| e.kind == EventKind::VoiceState && e.data["state"] == "ended"));
    rig.cleanup();
}
