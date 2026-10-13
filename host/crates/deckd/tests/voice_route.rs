//! §15's week-4 row 7: the route state machine against scripted nominal-rate
//! events.
//!
//! The script is the real one. Every number below was read off the deck on
//! 2026-09-21 (`$S/w4/probes/runA.txt`, `runB_eager.txt`, `runC_held.txt`):
//! the output's nominal follows our capture down to 16000 after ~110–170 ms,
//! the first non-zero mic sample arrives at ~1030 ms, and the flip back to
//! 44100 lands 2281–2447 ms after the close. What the machine is asked here
//! is whether it does the right thing at each of those instants.
use deck_model::Route;
use deckd::audio::route::{RouteAction, RouteMachine, A2DP_RATE, MAX_ATTEMPTS, REOPEN_WAIT};
use deckd::audio::traits::{DeviceSide, Fault, RouteEvent};
use std::time::{Duration, Instant};

fn opens(actions: &[RouteAction]) -> Vec<u32> {
    actions
        .iter()
        .filter_map(|a| match a {
            RouteAction::OpenOutput { rate } => Some(*rate),
            _ => None,
        })
        .collect()
}

fn reports(actions: &[RouteAction]) -> Vec<(Route, bool)> {
    actions
        .iter()
        .filter_map(|a| match a {
            RouteAction::Report { route, stuck } => Some((*route, *stuck)),
            _ => None,
        })
        .collect()
}

/// Bring a machine up the way the daemon does: the deck appears, its output
/// is at 44.1 kHz, the stream opens.
fn a2dp(now: Instant) -> RouteMachine {
    let mut sm = RouteMachine::new("none");
    sm.on(
        RouteEvent::DevicePresent {
            side: DeviceSide::Input,
            present: true,
        },
        now,
    );
    sm.on(
        RouteEvent::DevicePresent {
            side: DeviceSide::Output,
            present: true,
        },
        now,
    );
    sm.on(RouteEvent::NominalRate { rate: A2DP_RATE }, now);
    sm.on(RouteEvent::OutputOpened { rate: A2DP_RATE }, now);
    assert_eq!(sm.route(), Route::A2dp);
    sm
}

#[test]
fn the_route_machine_follows_the_rate_flips() {
    let t0 = Instant::now();

    // ---- A2DP → our PTT → ScoOurs -------------------------------------
    let mut sm = a2dp(t0);
    let pressed = sm.on(
        RouteEvent::OurCapture {
            open: true,
            rate: 16_000,
        },
        t0,
    );
    assert_eq!(sm.route(), Route::ScoOurs);
    assert!(
        pressed.contains(&RouteAction::CloseOutput),
        "§7.2 closes the output for our own capture, and V5 says why: the \
         deck returns our TTS to its own mic at ≈ +17 dB with no AEC"
    );
    assert_eq!(reports(&pressed), vec![(Route::ScoOurs, false)]);

    // ~110 ms later the output's own nominal follows the mic down. That is
    // **not** an external capture: it is ours.
    let flipped = sm.on(
        RouteEvent::NominalRate { rate: 16_000 },
        t0 + Duration::from_millis(114),
    );
    assert_eq!(sm.route(), Route::ScoOurs, "ours, not ScoExternal");
    assert!(opens(&flipped).is_empty(), "nothing opens while we capture");

    // ---- release → Reopening, at the CURRENT rate, at once (V3) --------
    let t_release = t0 + Duration::from_millis(1100);
    let released = sm.on(
        RouteEvent::OurCapture {
            open: false,
            rate: 0,
        },
        t_release,
    );
    assert_eq!(sm.route(), Route::Reopening);
    assert_eq!(
        opens(&released),
        vec![16_000],
        "V3: `Reopening` opens at the device's CURRENT nominal rate. \
         Waiting for 44100 costs 2591 ms to the first render callback \
         against 48 ms for this — 54× on every post-PTT announcement."
    );
    assert!(
        !opens(&released).contains(&A2DP_RATE),
        "and it does not wait for 44100"
    );
    sm.on(
        RouteEvent::OutputOpened { rate: 16_000 },
        t_release + Duration::from_millis(48),
    );
    assert_eq!(
        sm.route(),
        Route::Reopening,
        "open at the tail rate is not `A2dp`: `voice.route` must not claim \
         44.1 kHz while it is playing at 16 k"
    );
    assert_eq!(sm.open_at(), Some(16_000));

    // ---- → A2DP, 2.3 s later (V1: 2307 ms median) ----------------------
    let t_resume = t_release + Duration::from_millis(2288);
    let resumed = sm.on(RouteEvent::NominalRate { rate: A2DP_RATE }, t_resume);
    assert!(
        resumed.contains(&RouteAction::CloseOutput),
        "the 16 k stream is at the wrong rate now"
    );
    assert_eq!(opens(&resumed), vec![A2DP_RATE]);
    sm.on(RouteEvent::OutputOpened { rate: A2DP_RATE }, t_resume);
    assert_eq!(sm.route(), Route::A2dp);
    assert!(!sm.stuck());
}

/// V7: this is the *routine* path on this machine — the deck's input is the
/// system default input device (`list_audio`: `default_input_id=99` =
/// `C2-2D-81-AB-88-81:input`), so Zoom, Teams or dictation flips the route under us with no PTT of ours. It gets the same
/// coverage as the happy path because it happens as often.
#[test]
fn an_external_capture_takes_the_route_and_gives_it_back() {
    let t0 = Instant::now();
    let mut sm = a2dp(t0);

    let taken = sm.on(RouteEvent::NominalRate { rate: 16_000 }, t0);
    assert_eq!(sm.route(), Route::ScoExternal);
    assert!(taken.contains(&RouteAction::CloseOutput));
    assert!(
        opens(&taken).is_empty(),
        "we did not open this SCO and cannot end it: there is nothing to do \
         but wait it out"
    );
    assert_eq!(reports(&taken), vec![(Route::ScoExternal, false)]);

    // The property listener can also say so directly, while no stream of
    // ours is open (`kAudioDevicePropertyDeviceIsRunningSomewhere`).
    let mut sm2 = a2dp(t0);
    let running = sm2.on(RouteEvent::ExternalCapture { running: true }, t0);
    assert_eq!(sm2.route(), Route::ScoExternal);
    assert!(running.contains(&RouteAction::CloseOutput));

    // And back.
    let given_back = sm.on(
        RouteEvent::NominalRate { rate: A2DP_RATE },
        t0 + Duration::from_secs(30),
    );
    assert_eq!(opens(&given_back), vec![A2DP_RATE]);
    assert_eq!(sm.route(), Route::Reopening);
    sm.on(
        RouteEvent::OutputOpened { rate: A2DP_RATE },
        t0 + Duration::from_secs(30),
    );
    assert_eq!(sm.route(), Route::A2dp);
}

/// §7.2's `fail ×2 → ScoStuck … recovering on the next rate change`, by
/// both roads: two opens that fail, and two `REOPEN_WAIT`s in which the
/// rate never came back.
#[test]
fn two_failed_reopens_are_stuck_and_the_next_rate_change_recovers() {
    let t0 = Instant::now();
    let mut sm = a2dp(t0);
    sm.on(
        RouteEvent::OurCapture {
            open: true,
            rate: 16_000,
        },
        t0,
    );
    sm.on(RouteEvent::NominalRate { rate: 16_000 }, t0);
    let released = sm.on(
        RouteEvent::OurCapture {
            open: false,
            rate: 0,
        },
        t0,
    );
    assert_eq!(opens(&released), vec![16_000]);

    // First failure: one more go.
    let first = sm.on(RouteEvent::OutputFailed, t0);
    assert_eq!(opens(&first), vec![16_000], "§7.2's retry ×2");
    assert!(!sm.stuck());
    assert_eq!(MAX_ATTEMPTS, 2);

    // Second: stuck, and it says so on the wire.
    let second = sm.on(RouteEvent::OutputFailed, t0);
    assert_eq!(sm.route(), Route::ScoStuck);
    assert!(sm.stuck());
    assert_eq!(
        reports(&second),
        vec![(Route::ScoStuck, true)],
        "§7.2: voice.route{{stuck:true}}"
    );

    // "recovering on the next rate change" — any movement at all.
    let recovered = sm.on(
        RouteEvent::NominalRate { rate: A2DP_RATE },
        t0 + Duration::from_secs(5),
    );
    assert!(!sm.stuck());
    assert_eq!(opens(&recovered), vec![A2DP_RATE]);
    assert_eq!(sm.route(), Route::Reopening);

    // The other road: the flip back never comes. Under V3 that is a quality
    // problem and not silence — the stream is already open and audible at
    // the tail rate — which is why two 3 s waits are affordable.
    let mut sm = a2dp(t0);
    sm.on(
        RouteEvent::OurCapture {
            open: true,
            rate: 16_000,
        },
        t0,
    );
    sm.on(RouteEvent::NominalRate { rate: 16_000 }, t0);
    sm.on(
        RouteEvent::OurCapture {
            open: false,
            rate: 0,
        },
        t0,
    );
    sm.on(RouteEvent::OutputOpened { rate: 16_000 }, t0);
    assert!(sm.on(RouteEvent::Tick, t0 + REOPEN_WAIT / 2).is_empty());
    let first = sm.on(RouteEvent::Tick, t0 + REOPEN_WAIT);
    assert_eq!(
        first,
        vec![RouteAction::ReadNominal],
        "ask the device again before giving up on it"
    );
    let second = sm.on(RouteEvent::Tick, t0 + REOPEN_WAIT * 2 + REOPEN_WAIT);
    assert_eq!(sm.route(), Route::ScoStuck);
    assert_eq!(reports(&second), vec![(Route::ScoStuck, true)]);
    assert_eq!(
        sm.open_at(),
        Some(16_000),
        "§7.2's `ScoStuck`: the output is open at the wrong rate and \
         resampled — stuck is not silent"
    );
}

/// V19: on `StreamInvalidated`/`DeviceNotAvailable` cpal **pauses the unit
/// before** calling the error callback. So a fault is not "a warning while
/// playback continues": whatever we thought was playing has stopped.
#[test]
fn a_fault_is_a_stream_that_has_already_stopped() {
    let t0 = Instant::now();
    let mut sm = a2dp(t0);
    assert_eq!(sm.open_at(), Some(A2DP_RATE));

    let invalidated = sm.on(RouteEvent::Fault(Fault::Invalidated), t0);
    assert_eq!(
        sm.open_at(),
        None,
        "cpal already paused it; pretending otherwise renders into a \
         stopped unit"
    );
    assert!(invalidated.contains(&RouteAction::CloseOutput));
    assert!(
        invalidated.contains(&RouteAction::ReadNominal),
        "the fault says the rate moved, never to what"
    );

    // An xrun is a glitch, not a state: `metrics.get` counts it and the
    // route does not move.
    let mut sm = a2dp(t0);
    let xrun = sm.on(RouteEvent::Fault(Fault::Xrun), t0);
    assert!(xrun.is_empty());
    assert_eq!(sm.route(), Route::A2dp);
    assert_eq!(sm.open_at(), Some(A2DP_RATE));

    // The link dropped.
    let gone = sm.on(RouteEvent::Fault(Fault::Gone), t0);
    assert_eq!(sm.route(), Route::None);
    assert!(gone.contains(&RouteAction::CloseOutput));
    assert_eq!(reports(&gone), vec![(Route::None, false)]);

    // …and with `fallback_output = "system_default"` the same drop says
    // something different (§7.2).
    let mut sm = a2dp(t0);
    sm.set_fallback_output("system_default");
    sm.on(RouteEvent::Fault(Fault::Gone), t0);
    assert_eq!(sm.route(), Route::SystemDefault);
}

/// V5, on the one branch that did not check it.
///
/// Every path that can open the output asks `self.ours` first, because §7.2
/// closes the output for our own capture and V5 measured why: the deck
/// returns our own TTS to its own mic at ≈ +17 dB with no AEC, so a speaker
/// opened during capture feeds the recogniser its own voice.
/// `RouteEvent::OutputFailed` was the exception — it retried the open with
/// no such guard. No caller can reach it today (a failed open only follows
/// a `try_reopen`, which already refuses while we hold the microphone), and
/// "unreachable" is a fact about today's callers rather than about this
/// branch. A state machine that is a pure fold is exactly the thing to test
/// on its own terms.
#[test]
fn a_failed_open_never_retries_while_we_hold_the_microphone() {
    let t0 = Instant::now();
    let mut sm = a2dp(t0);
    // Our press: the output closes and the route is ours.
    sm.on(
        RouteEvent::OurCapture {
            open: true,
            rate: 16_000,
        },
        t0,
    );
    assert_eq!(sm.route(), Route::ScoOurs);

    let retried = sm.on(RouteEvent::OutputFailed, t0);
    assert!(
        opens(&retried).is_empty(),
        "V5: never the speaker during our own capture, {retried:?}"
    );

    // …and the guard is about `ours`, not about refusing to retry: the
    // same failure after the release gets its retry as usual (a fresh
    // machine, because `MAX_ATTEMPTS` is 2 and the one above spent one).
    let mut sm = a2dp(t0);
    sm.on(
        RouteEvent::OurCapture {
            open: true,
            rate: 16_000,
        },
        t0,
    );
    sm.on(
        RouteEvent::OurCapture {
            open: false,
            rate: 0,
        },
        t0,
    );
    let after = sm.on(RouteEvent::OutputFailed, t0);
    assert_eq!(opens(&after), vec![A2DP_RATE], "{after:?}");
}
