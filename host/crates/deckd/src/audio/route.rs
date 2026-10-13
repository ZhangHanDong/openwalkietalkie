//! §7.2's output route state machine, as a pure fold over [`RouteEvent`].
//!
//! It calls no CoreAudio and owns no stream (V17): events in, actions out,
//! which is what makes §15's row 7 a scripted test instead of a Bluetooth
//! ritual. The platform layer translates cpal's
//! `ErrorKind::{StreamInvalidated, DeviceNotAvailable, Xrun}` and a
//! standalone property listener into the events; this decides what to do.
//!
//! Three measurements shape it, and each one contradicts the design's text:
//!
//! * **V3 — `Reopening` opens at the device's CURRENT nominal rate, at
//!   once.** The design says "wait nominal 44100 ≤3 s". Waiting costs
//!   **2591 ms** to the first render callback; opening eagerly at whatever
//!   the device reports (16000 during the SCO tail) costs **48 ms**. That is
//!   54× on *every* post-PTT announcement, and the 2.3 s is a fixed macOS
//!   SCO-teardown timer that cannot be hurried. So the machine opens
//!   immediately and resamples into the live rate, and re-opens when the
//!   rate flips back.
//! * **V7 — `ScoExternal` is the routine path here, not an edge case.** The
//!   deck's input is this machine's **system default input device**, so any
//!   app that records — Zoom, Teams, dictation — opens SCO and flips the rate under us. It gets the same treatment as
//!   the happy path, and the same tests.
//! * **V19 — cpal pauses the unit *before* calling the error callback** on
//!   `StreamInvalidated`/`DeviceNotAvailable`. So a fault is not "something
//!   went wrong and the stream carries on": the stream is already stopped,
//!   and the machine must close and rebuild rather than hope.
//!
//! V4 is why it can trust its own events: an open output AUHAL survived both
//! rate flips with no CoreAudio error (51–59 render callbacks while nominal
//! was 16000, continuing after the flip), and the nominal-rate listener is
//! accurate to ~30–50 ms. Re-opening is cpal's contract, not the hardware's.
//!
//! **The settled rate is a parameter, not a fact about every deck.** Over
//! Classic Bluetooth the route is only settled at [`A2DP_RATE`], and every
//! other rate is the SCO tail or somebody else's capture. Over the deck's
//! BLE audio service (`[voice] transport = "ble"`, `audio::ble`) the speaker
//! runs at 16 kHz and stays there: there is no SCO, no tail and no other
//! app that can flip it, so the machine is told to settle at 16000
//! ([`RouteMachine::set_transport`]) and the rest of it — a capture of
//! ours closes the output, the release reopens it, a vanished device is
//! `None` — applies unchanged. Only the two states that name the transport
//! are spelled differently there: settled is [`Route::Ble`] rather than
//! [`Route::A2dp`], and our own capture [`Route::BleOurs`] rather than
//! [`Route::ScoOurs`].
use super::traits::{DeviceSide, Fault, RouteEvent};
use deck_model::{Route, VoiceTransport};
use std::time::{Duration, Instant};

/// The deck's A2DP rate: the only rate at which a Classic route is `A2dp`,
/// and the machine's settled rate unless it is told otherwise.
pub const A2DP_RATE: u32 = 44_100;

/// The BLE audio service's one rate (`codec::RATE_HZ`).
pub const BLE_RATE: u32 = crate::ble::codec::RATE_HZ;

/// §7.2's `Reopening(wait ≤3 s, retry ×2)`. The wait is now only for the
/// **rate flip back**, because V3 opens the stream before it (A2DP resume
/// is 2307 ms median, so this leaves ~580 ms of margin — another reason not
/// to spend any of it waiting to open).
pub const REOPEN_WAIT: Duration = Duration::from_secs(3);

/// §7.2's `retry ×2`, after which the route is [`Route::ScoStuck`].
pub const MAX_ATTEMPTS: u8 = 2;

/// What the machine asks the control loop to do. It never does it itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteAction {
    /// Build an output stream at exactly this rate. **V3: this is the
    /// device's current nominal rate, which during the SCO tail is 16000.**
    OpenOutput { rate: u32 },
    /// Drop the output stream. Also the right answer to a fault, because
    /// cpal has already paused the unit (V19).
    CloseOutput,
    /// Re-read `kAudioDevicePropertyNominalSampleRate`: a
    /// `StreamInvalidated` says the rate moved but not to what.
    ReadNominal,
    /// `VoiceIn::Route` for the core (§4.12's `voice.route`).
    Report { route: Route, stuck: bool },
}

/// §7.2, folded.
#[derive(Clone, Debug)]
pub struct RouteMachine {
    route: Route,
    /// The device's last known nominal rate. 0 until something says.
    nominal: u32,
    /// The rate the output stream is actually open at.
    open_at: Option<u32>,
    output_present: bool,
    input_present: bool,
    /// Someone else is holding the mic (V7).
    external: bool,
    /// *We* are holding the mic.
    ours: bool,
    attempts: u8,
    stuck: bool,
    /// When the current `Reopening` began, for [`REOPEN_WAIT`].
    reopening_since: Option<Instant>,
    /// §7.2's `fallback_output`: `"none"` (silent) or `"system_default"`.
    fallback_output: String,
    /// The rate at which an open output is settled rather than reopening:
    /// [`A2DP_RATE`] for the Classic deck, 16000 for the BLE audio service.
    settled: u32,
    /// What the settled state and our own capture are called on this
    /// transport.
    settled_route: Route,
    ours_route: Route,
}

impl RouteMachine {
    #[must_use]
    pub fn new(fallback_output: &str) -> Self {
        RouteMachine {
            route: Route::None,
            nominal: 0,
            open_at: None,
            output_present: false,
            input_present: false,
            external: false,
            ours: false,
            attempts: 0,
            stuck: false,
            reopening_since: None,
            fallback_output: fallback_output.to_string(),
            settled: A2DP_RATE,
            settled_route: Route::A2dp,
            ours_route: Route::ScoOurs,
        }
    }

    /// The transport the machine runs for: a Classic deck settles at
    /// [`A2DP_RATE`] as `A2dp`, the BLE audio service at its only rate
    /// (16000) as `Ble`. Set before the first event; the machine does not
    /// re-evaluate an output that is already open.
    pub fn set_transport(&mut self, transport: VoiceTransport) {
        (self.settled, self.settled_route, self.ours_route) = match transport {
            VoiceTransport::Ble => (BLE_RATE, Route::Ble, Route::BleOurs),
            VoiceTransport::Coreaudio => (A2DP_RATE, Route::A2dp, Route::ScoOurs),
        };
    }

    /// The rate the route settles at.
    #[must_use]
    pub fn settled_rate(&self) -> u32 {
        self.settled
    }

    #[must_use]
    pub fn route(&self) -> Route {
        self.route
    }

    #[must_use]
    pub fn stuck(&self) -> bool {
        self.stuck
    }

    /// The rate the output stream is open at, if it is.
    #[must_use]
    pub fn open_at(&self) -> Option<u32> {
        self.open_at
    }

    #[must_use]
    pub fn nominal(&self) -> u32 {
        self.nominal
    }

    pub fn set_fallback_output(&mut self, fallback: &str) {
        self.fallback_output = fallback.to_string();
    }

    /// One event. Returns what the control loop should do, in order.
    pub fn on(&mut self, event: RouteEvent, now: Instant) -> Vec<RouteAction> {
        let before = (self.route, self.stuck);
        let mut actions = Vec::new();
        match event {
            RouteEvent::DevicePresent { side, present } => match side {
                DeviceSide::Input => {
                    self.input_present = present;
                }
                DeviceSide::Output => {
                    self.output_present = present;
                    if present {
                        self.try_reopen(now, &mut actions);
                    } else {
                        self.close(&mut actions);
                        self.reopening_since = None;
                        self.attempts = 0;
                        self.stuck = false;
                        self.settle();
                    }
                }
            },
            RouteEvent::NominalRate { rate } => self.on_nominal(rate, now, &mut actions),
            RouteEvent::ExternalCapture { running } => {
                self.external = running;
                if running && !self.ours {
                    // V7: the routine path on this machine. The output is
                    // about to flip under us; close before it does.
                    self.close(&mut actions);
                    self.reopening_since = None;
                    self.route = Route::ScoExternal;
                } else {
                    self.settle();
                }
            }
            RouteEvent::OurCapture { open, .. } => {
                self.ours = open;
                if open {
                    // §7.2: hold the lanes, **close the output**, open the
                    // input. V5 makes that load-bearing rather than
                    // precautionary — the deck's speaker returns our own TTS
                    // to its own mic at ≈ +17 dB, and there is no AEC.
                    self.reopening_since = None;
                    self.route = self.ours_route;
                    self.close(&mut actions);
                } else {
                    // The release. V3 lives here.
                    self.try_reopen(now, &mut actions);
                }
            }
            RouteEvent::OutputOpened { rate } => {
                self.open_at = Some(rate);
                self.attempts = 0;
                if rate == self.settled {
                    self.reopening_since = None;
                    self.stuck = false;
                    self.settle();
                }
                // Opened at the tail rate instead: stay in `Reopening` and
                // play into it. The route is not `A2dp` until it is at the
                // settled rate (44.1 kHz on a Classic deck), and saying
                // otherwise would make `voice.route` a lie.
            }
            RouteEvent::OutputFailed => {
                self.open_at = None;
                self.attempts = self.attempts.saturating_add(1);
                if self.attempts >= MAX_ATTEMPTS {
                    self.give_up();
                } else if self.nominal != 0 && self.output_present && !self.ours {
                    // `!self.ours` is the guard every other path here has
                    // and this one did not. It is unreachable today — a
                    // failed open only follows a `try_reopen`, which
                    // already refuses while we hold the microphone — but
                    // "unreachable" is a fact about the callers, and the
                    // one thing this branch must never do is open the
                    // speaker during our own capture: V5 measured the
                    // deck's loopback at ≈ +17 dB with no AEC, so that is
                    // our own TTS going straight back into the recogniser.
                    actions.push(RouteAction::OpenOutput { rate: self.nominal });
                }
            }
            RouteEvent::Fault(fault) => self.on_fault(fault, now, &mut actions),
            RouteEvent::Tick => {
                if let Some(since) = self.reopening_since {
                    if now.duration_since(since) >= REOPEN_WAIT {
                        // §7.2's `wait ≤3 s, retry ×2`. Under V3 the wait is
                        // only for the **flip back** — the stream is already
                        // open and audible at the tail rate — so a timeout
                        // costs quality, not silence.
                        self.attempts = self.attempts.saturating_add(1);
                        if self.attempts >= MAX_ATTEMPTS {
                            self.give_up();
                        } else {
                            self.reopening_since = Some(now);
                            actions.push(RouteAction::ReadNominal);
                        }
                    }
                }
            }
        }
        if (self.route, self.stuck) != before {
            actions.push(RouteAction::Report {
                route: self.route,
                stuck: self.stuck,
            });
        }
        actions
    }

    fn on_nominal(&mut self, rate: u32, now: Instant, actions: &mut Vec<RouteAction>) {
        let moved = self.nominal != rate;
        self.nominal = rate;
        if !moved {
            return;
        }
        if self.stuck {
            // §7.2: "recovering on the next rate change". Any movement is a
            // fresh chance.
            self.stuck = false;
            self.attempts = 0;
            self.try_reopen(now, actions);
            return;
        }
        if self.ours {
            // Our own SCO. The output stays closed until the release (V5).
            return;
        }
        if rate != self.settled {
            // V7: somebody else opened the mic and took the link with them.
            self.close(actions);
            self.reopening_since = None;
            self.route = Route::ScoExternal;
            return;
        }
        // The A2DP resume — 2307 ms after the close, median (V1).
        self.try_reopen(now, actions);
    }

    fn on_fault(&mut self, fault: Fault, now: Instant, actions: &mut Vec<RouteAction>) {
        match fault {
            // cpal paused the unit **before** it called us (V19), so this is
            // not "something went wrong and playback carries on".
            Fault::Invalidated => {
                self.close(actions);
                actions.push(RouteAction::ReadNominal);
            }
            Fault::Gone => {
                self.output_present = false;
                self.close(actions);
                self.reopening_since = None;
                self.settle();
            }
            // §7.8: a denied grant is silence, not an error. Nothing to
            // re-open, and nothing to report as a route.
            Fault::Denied => self.close(actions),
            // An xrun is a glitch, not a state: it belongs to `metrics.get`.
            Fault::Xrun | Fault::Other => {
                let _ = now;
            }
        }
    }

    /// Open the output at the device's **current** nominal rate, now (V3).
    ///
    /// Not "wait for 44100, then open": that is 2591 ms to the first render
    /// callback against 48 ms for this, on every post-PTT announcement, and
    /// the 2.3 s is a fixed macOS SCO-teardown timer.
    fn try_reopen(&mut self, now: Instant, actions: &mut Vec<RouteAction>) {
        if self.ours || !self.output_present {
            self.settle();
            return;
        }
        if self.nominal == self.settled && self.open_at == Some(self.settled) {
            self.reopening_since = None;
            self.settle();
            return;
        }
        if self.open_at.is_some() {
            self.close(actions);
        }
        self.reopening_since = Some(now);
        self.route = Route::Reopening;
        if self.nominal != 0 {
            actions.push(RouteAction::OpenOutput { rate: self.nominal });
        }
    }

    /// Two goes and we are still not at 44.1 kHz (§7.2).
    fn give_up(&mut self) {
        self.stuck = true;
        self.reopening_since = None;
        self.route = Route::ScoStuck;
    }

    /// Where the machine belongs when nothing is in flight.
    fn settle(&mut self) {
        if self.ours || self.reopening_since.is_some() {
            return; // `ScoOurs`/`BleOurs` and `Reopening` own their own state
        }
        if !self.output_present {
            self.route = if self.fallback_output == "system_default" {
                Route::SystemDefault
            } else {
                Route::None
            };
            return;
        }
        if self.external || (self.nominal != 0 && self.nominal != self.settled) {
            self.route = Route::ScoExternal;
        } else if self.open_at == Some(self.settled) {
            self.route = self.settled_route;
        }
    }

    /// Idempotent: the control loop's close is, and asking twice in one
    /// event would only make the action list harder to read.
    fn close(&mut self, actions: &mut Vec<RouteAction>) {
        self.open_at = None;
        if actions.last() != Some(&RouteAction::CloseOutput) {
            actions.push(RouteAction::CloseOutput);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opens(actions: &[RouteAction]) -> Vec<u32> {
        actions
            .iter()
            .filter_map(|a| match a {
                RouteAction::OpenOutput { rate } => Some(*rate),
                _ => None,
            })
            .collect()
    }

    /// The tail of V3 in one assertion, kept here beside the machine as
    /// well as in `tests/voice_route.rs`: the release opens the output
    /// before the rate has come back, or it is 2.5 s late.
    #[test]
    fn a_release_opens_at_the_tail_rate_without_waiting() {
        let now = Instant::now();
        let mut sm = RouteMachine::new("none");
        sm.on(
            RouteEvent::DevicePresent {
                side: DeviceSide::Output,
                present: true,
            },
            now,
        );
        sm.on(RouteEvent::NominalRate { rate: 44_100 }, now);
        sm.on(RouteEvent::OutputOpened { rate: 44_100 }, now);
        assert_eq!(sm.route(), Route::A2dp);

        sm.on(
            RouteEvent::OurCapture {
                open: true,
                rate: 16_000,
            },
            now,
        );
        assert_eq!(sm.route(), Route::ScoOurs);
        // ~110 ms later the output's nominal follows the mic down.
        sm.on(RouteEvent::NominalRate { rate: 16_000 }, now);
        assert_eq!(sm.route(), Route::ScoOurs, "our own capture, not external");

        let released = sm.on(
            RouteEvent::OurCapture {
                open: false,
                rate: 0,
            },
            now,
        );
        assert_eq!(
            opens(&released),
            vec![16_000],
            "V3: open at the *current* rate at once, not at 44100 in 2.5 s"
        );
        assert_eq!(sm.route(), Route::Reopening);
    }

    /// One event the way `Engine::drive` handles it: an `OpenOutput` is
    /// answered at once with the open it asked for, and folded back in.
    fn drive(sm: &mut RouteMachine, event: RouteEvent, now: Instant) -> Vec<RouteAction> {
        let mut all = Vec::new();
        let mut next = vec![event];
        while let Some(event) = next.pop() {
            for action in sm.on(event, now) {
                if let RouteAction::OpenOutput { rate } = action {
                    next.push(RouteEvent::OutputOpened { rate });
                }
                all.push(action);
            }
        }
        all
    }

    fn present(side: DeviceSide, present: bool) -> RouteEvent {
        RouteEvent::DevicePresent { side, present }
    }

    fn routes(actions: &[RouteAction]) -> Vec<Route> {
        actions
            .iter()
            .filter_map(|a| match a {
                RouteAction::Report { route, .. } => Some(*route),
                _ => None,
            })
            .collect()
    }

    /// The BLE transport's route: the deck's speaker is a 16 kHz stream that
    /// nothing else can flip, so 16000 is where the machine settles. What
    /// §7.2 says about our own capture still holds — the output closes for
    /// it and comes back on the release — but there is no 2.3 s tail to sit
    /// out, no `ScoExternal` and nothing to get stuck on.
    #[test]
    fn a_ble_route_settles_at_16_khz_and_a_release_reopens_at_once() {
        let t0 = Instant::now();
        let mut sm = RouteMachine::new("none");
        sm.set_transport(VoiceTransport::Ble);
        assert_eq!(sm.settled_rate(), BLE_RATE);

        // The link comes up: `audio::ble::route_events` sends the rate, then
        // the two sides.
        assert!(drive(&mut sm, RouteEvent::NominalRate { rate: 16_000 }, t0).is_empty());
        drive(&mut sm, present(DeviceSide::Input, true), t0);
        let up = drive(&mut sm, present(DeviceSide::Output, true), t0);
        assert_eq!(opens(&up), vec![16_000]);
        assert_eq!(sm.route(), Route::Ble, "settled, not Reopening");
        assert_eq!(sm.open_at(), Some(16_000));
        assert!(!sm.stuck());
        assert_eq!(routes(&up).last(), Some(&Route::Ble));
        // Settled means settled: no wait is running, so time changes nothing.
        assert!(drive(&mut sm, RouteEvent::Tick, t0 + REOPEN_WAIT * 3).is_empty());
        assert_eq!(sm.route(), Route::Ble);

        // A press: the output closes for our own capture (the deck discards
        // speaker frames while its microphone streams, and its speaker
        // reaches its microphone at about +17 dB either way).
        let press = drive(
            &mut sm,
            RouteEvent::OurCapture {
                open: true,
                rate: 16_000,
            },
            t0,
        );
        assert!(press.contains(&RouteAction::CloseOutput));
        assert_eq!(sm.route(), Route::BleOurs);

        // The release reopens at once, at the rate it settles at, and is
        // settled in the same breath: nothing to wait for.
        let release = drive(
            &mut sm,
            RouteEvent::OurCapture {
                open: false,
                rate: 0,
            },
            t0,
        );
        assert_eq!(opens(&release), vec![16_000]);
        assert_eq!(routes(&release), vec![Route::Reopening, Route::Ble]);
        assert!(
            drive(&mut sm, RouteEvent::Tick, t0 + REOPEN_WAIT * 3).is_empty(),
            "no reopen wait left running to time out into ScoStuck"
        );
        assert!(!sm.stuck());

        // The link drops and comes back.
        let gone = drive(&mut sm, present(DeviceSide::Output, false), t0);
        assert!(gone.contains(&RouteAction::CloseOutput));
        assert_eq!(sm.route(), Route::None);
        let back = drive(&mut sm, present(DeviceSide::Output, true), t0);
        assert_eq!(opens(&back), vec![16_000]);
        assert_eq!(sm.route(), Route::Ble);
        // A repeated rate report is not news.
        assert!(drive(&mut sm, RouteEvent::NominalRate { rate: 16_000 }, t0).is_empty());

        // With `fallback_output = "system_default"` an absent deck is named
        // as such, exactly as on the Classic path.
        sm.set_fallback_output("system_default");
        drive(&mut sm, present(DeviceSide::Output, false), t0);
        assert_eq!(sm.route(), Route::SystemDefault);
    }

    /// The default is untouched: the Classic machine settles at 44.1 kHz
    /// only, so the same 16 kHz-only device reads as somebody else's SCO —
    /// which is why the BLE transport has to say what it settles at.
    #[test]
    fn the_default_settled_rate_is_still_a2dp() {
        let t0 = Instant::now();
        let mut sm = RouteMachine::new("none");
        assert_eq!(sm.settled_rate(), A2DP_RATE);
        drive(&mut sm, RouteEvent::NominalRate { rate: 16_000 }, t0);
        drive(&mut sm, present(DeviceSide::Output, true), t0);
        assert_eq!(sm.route(), Route::Reopening, "open at 16 k, waiting");
        let opened = drive(&mut sm, RouteEvent::NominalRate { rate: 44_100 }, t0);
        assert_eq!(opens(&opened), vec![44_100]);
        assert_eq!(sm.route(), Route::A2dp);
        drive(&mut sm, RouteEvent::NominalRate { rate: 16_000 }, t0);
        assert_eq!(sm.route(), Route::ScoExternal);
    }
}
