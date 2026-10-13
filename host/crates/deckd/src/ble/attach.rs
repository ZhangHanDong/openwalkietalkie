//! The attach policy as a pure fold: what CoreBluetooth reported in, what
//! to ask of it out. It owns no peripheral and calls nothing (the engine in
//! `central` does both), which is what lets the retry, timeout and recovery
//! rules be tests instead of a Bluetooth ritual.
//!
//! The policy, in order:
//!
//! 1. Nothing happens until the radio is powered on (and the process is
//!    allowed to use it); every other radio state is "detached, and why".
//! 2. **Find**: a deck the system already holds for HID is the normal case.
//!    The engine asks for connected peripherals with the audio service, then
//!    for connected peripherals with the Battery service named
//!    [`DECK_NAME`] (the system's GATT cache only knows the audio service if
//!    something discovered it before), and answers `Found` or `NotFound`.
//! 3. Not found: **scan** for a peripheral advertising that name, and keep
//!    looking with Find on every tick while the scan runs — only when the
//!    link was made with [`Reach::Advertising`] (the bench probe). With
//!    [`Reach::Connected`] (`deckd run`) the machine only keeps looking:
//!    connecting to an advertising deck brings in macOS's HID service,
//!    which starts pairing and puts a pairing request on screen
//!    (`docs/deck-protocol.md`, "Pairing"), and a daemon must not do that
//!    on its own every two seconds — least of all to a neighbour's deck
//!    that happens to share the name. A paired deck is one the system keeps
//!    connected for HID, which is exactly what Find finds.
//! 4. A candidate: **connect** (for a system-held deck that is only our
//!    claim on a link that already exists), **discover** the audio service
//!    and its three characteristics, then **read** info and **subscribe** to
//!    the mic, in either order of completion. Both done is `Ready`.
//! 5. Any failure — connect, discovery, subscription, a disconnect, or a
//!    step (connect, services, characteristics, subscription) that makes no
//!    progress for [`ATTACH_TIMEOUT`] — releases the candidate and goes back
//!    to step 2 after [`RETRY`]; each further failure in a row doubles that
//!    wait, up to [`MAX_BACKOFF`], so a deck that cannot attach (firmware
//!    without the audio service, say) is not connected, rediscovered and
//!    released every two seconds for as long as the daemon runs. Attaching,
//!    the radio coming back, or the deck dropping out of the connected list
//!    starts the count again. A deck that changes its services while
//!    attached is rediscovered in place.
//!
//! Every input that does not fit the current phase is a late callback about
//! something the machine already gave up on, and is ignored.
use super::DECK_NAME;
use std::time::{Duration, Instant};

/// How often a link that is not ready looks again (and checks its timeout).
pub const RETRY: Duration = Duration::from_secs(2);
/// How long one attach step (connect, service discovery, characteristic
/// discovery, subscription) may go without an answer. Per step, not for the
/// whole attach: on the bench (r6, 2026-10-04) the first discovery after the
/// system had just connected the deck sat for over 10 s while the retry
/// finished in under 7, so slow-but-moving must not count as stuck.
pub const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest wait between attempts at a deck that keeps failing to attach.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Which decks a link may attach to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    /// Only a deck this Mac already holds connected — the system keeps a
    /// paired deck connected for its HID side. Never connects to an
    /// advertising peripheral, so it never brings up a pairing request and
    /// never touches somebody else's deck. What `deckd run` uses.
    #[default]
    Connected,
    /// Also scan for a deck advertising its name and connect to it. On an
    /// unpaired deck that is a pairing request on this Mac's screen, so it
    /// is for `deckd ble probe`, at a bench, on purpose.
    Advertising,
}

/// `CBManagerState`, as far as the link cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radio {
    Unknown,
    Resetting,
    Unsupported,
    Unauthorized,
    Off,
    On,
}

impl Radio {
    /// `CBManagerState` (CBManager.h): unknown 0, resetting 1, unsupported
    /// 2, unauthorized 3, powered off 4, powered on 5.
    #[must_use]
    pub fn from_raw(state: isize) -> Radio {
        match state {
            1 => Radio::Resetting,
            2 => Radio::Unsupported,
            3 => Radio::Unauthorized,
            4 => Radio::Off,
            5 => Radio::On,
            _ => Radio::Unknown,
        }
    }

    /// Why nothing can attach in this state.
    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Radio::Unknown => "Bluetooth has not reported its state yet",
            Radio::Resetting => "Bluetooth is resetting",
            Radio::Unsupported => "this Mac has no Bluetooth LE",
            Radio::Unauthorized => {
                "this process may not use Bluetooth (System Settings > Privacy & Security > \
                 Bluetooth)"
            }
            Radio::Off => "Bluetooth is off",
            Radio::On => "Bluetooth is on",
        }
    }
}

/// What CoreBluetooth said, translated by the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Radio(Radio),
    /// The timer [`Action::Arm`] asked for.
    Tick,
    /// The engine holds a candidate peripheral (from Find or the scan).
    Found,
    /// Find came back empty.
    NotFound,
    Connected,
    ConnectFailed(String),
    Disconnected(String),
    /// The audio service is there, or why not.
    Services(Result<(), String>),
    /// Mic, speaker and info are all there, or why not.
    Characteristics(Result<(), String>),
    /// The info read finished: the five bytes, or `None` if it failed.
    Info(Option<[u8; 5]>),
    /// Mic notifications are on, or why not (or why they stopped).
    Notifying(Result<(), String>),
    /// The deck invalidated the audio service while attached.
    ServicesChanged,
}

/// What the engine should do. It never does anything else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Look for the deck among connected peripherals; answer `Found`
    /// (keeping the peripheral) or `NotFound`.
    Find,
    /// Scan for a peripheral with the deck's name; a match is `Found`.
    Scan,
    StopScan,
    /// Connect to the candidate.
    Connect,
    /// Let go of the candidate: cancel our connection if it has one.
    Release,
    DiscoverServices,
    DiscoverCharacteristics,
    ReadInfo,
    Subscribe,
    /// Tell the handle (deduplicated: only changes are reported).
    Report(Report),
    /// Why the link is not ready, for the log and the probe.
    Problem(String),
    /// Deliver one [`Input::Tick`] after [`RETRY`]. Never asked twice
    /// before the tick arrives.
    Arm,
}

/// The link state as the machine knows it; the engine adds what only the
/// peripheral knows (the write limit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Detached,
    Attaching,
    Ready { info: Option<[u8; 5]> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The radio is not usable.
    Down,
    /// Nothing in hand; the next tick looks again.
    Idle,
    Scanning,
    Connecting,
    /// Services, then characteristics.
    Discovering,
    /// Info read and mic subscription in flight.
    Subscribing,
    Ready,
}

#[derive(Clone, Debug)]
pub struct Attach {
    /// The GAP name a deck goes by ([`DECK_NAME`] unless the bench says).
    name: String,
    /// Whether a deck that is not connected may be scanned for and
    /// connected to.
    reach: Reach,
    phase: Phase,
    /// When the attach in progress gives up.
    deadline: Option<Instant>,
    /// A tick is on its way.
    armed: bool,
    reported: Report,
    /// While subscribing: the info read's result once it is in.
    info: Option<Option<[u8; 5]>>,
    notifying: bool,
    /// Failed attaches in a row, and when the next may start.
    failures: u32,
    hold_until: Option<Instant>,
}

impl Default for Attach {
    fn default() -> Self {
        Attach::new()
    }
}

impl Attach {
    /// The bench policy for a deck named [`DECK_NAME`]: it scans
    /// ([`Reach::Advertising`]).
    #[must_use]
    pub fn new() -> Attach {
        Attach::named(DECK_NAME)
    }

    /// The bench policy for a deck that goes by `name`: it scans
    /// ([`Reach::Advertising`]).
    #[must_use]
    pub fn named(name: &str) -> Attach {
        Attach::reaching(name, Reach::Advertising)
    }

    /// The policy for a deck that goes by `name`, attaching only as far as
    /// `reach` allows.
    #[must_use]
    pub fn reaching(name: &str, reach: Reach) -> Attach {
        Attach {
            name: name.to_string(),
            reach,
            phase: Phase::Down,
            deadline: None,
            armed: false,
            reported: Report::Detached,
            info: None,
            notifying: false,
            failures: 0,
            hold_until: None,
        }
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// A new candidate is welcome: the engine may replace the one it holds
    /// (it holds none) and answer `Found`.
    #[must_use]
    pub fn wants_candidate(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Scanning)
    }

    /// Fold one input in.
    pub fn step(&mut self, now: Instant, input: Input) -> Vec<Action> {
        let mut out = Vec::new();
        match input {
            Input::Radio(Radio::On) => {
                if self.phase == Phase::Down {
                    self.enter(Phase::Idle);
                    self.forgive();
                    out.push(Action::Find);
                }
            }
            Input::Radio(radio) => {
                // Whatever CoreBluetooth had went with the radio.
                if self.holds_candidate() {
                    out.push(Action::Release);
                }
                self.enter(Phase::Down);
                out.push(Action::Problem(radio.why().into()));
                self.report(&mut out, Report::Detached);
            }
            Input::Tick => {
                self.armed = false;
                match self.phase {
                    // Backing off after failures: only the timer runs.
                    Phase::Idle if self.hold_until.is_some_and(|t| now < t) => {}
                    Phase::Idle | Phase::Scanning => out.push(Action::Find),
                    Phase::Connecting | Phase::Discovering | Phase::Subscribing => {
                        if self.deadline.is_some_and(|d| now >= d) {
                            let what = match self.phase {
                                Phase::Connecting => "connecting",
                                Phase::Discovering => "discovering the audio service",
                                _ => "subscribing to the microphone",
                            };
                            self.fail(now, &mut out, format!("timed out {what}"));
                        }
                    }
                    Phase::Down | Phase::Ready => {}
                }
            }
            Input::Found => {
                if self.wants_candidate() {
                    if self.phase == Phase::Scanning {
                        out.push(Action::StopScan);
                    }
                    out.push(Action::Connect);
                    self.enter(Phase::Connecting);
                    self.deadline = Some(now + ATTACH_TIMEOUT);
                    self.report(&mut out, Report::Attaching);
                }
            }
            Input::NotFound => {
                if self.phase == Phase::Idle {
                    // Whatever kept failing is gone; when a deck comes back
                    // (reflashed, re-paired), it is tried at once.
                    self.forgive();
                    match self.reach {
                        Reach::Advertising => {
                            out.push(Action::Scan);
                            self.enter(Phase::Scanning);
                            out.push(Action::Problem(format!(
                                "no {} is connected or advertising",
                                self.name
                            )));
                        }
                        // Stay idle: the armed tick looks again. The
                        // engine logs a repeated problem once.
                        Reach::Connected => out.push(Action::Problem(format!(
                            "no {} is connected to this Mac (pair it in System Settings > \
                             Bluetooth; deckd never connects to an advertising deck by itself)",
                            self.name
                        ))),
                    }
                }
            }
            Input::Connected => {
                if self.phase == Phase::Connecting {
                    out.push(Action::DiscoverServices);
                    self.phase = Phase::Discovering;
                    self.deadline = Some(now + ATTACH_TIMEOUT);
                }
            }
            Input::ConnectFailed(why) => {
                if self.phase == Phase::Connecting {
                    self.fail(now, &mut out, why);
                }
            }
            Input::Disconnected(why) => {
                if self.holds_candidate() {
                    self.fail(now, &mut out, why);
                }
            }
            Input::Services(Ok(())) => {
                if self.phase == Phase::Discovering {
                    out.push(Action::DiscoverCharacteristics);
                    self.deadline = Some(now + ATTACH_TIMEOUT);
                }
            }
            Input::Services(Err(why)) | Input::Characteristics(Err(why)) => {
                if self.phase == Phase::Discovering {
                    self.fail(now, &mut out, why);
                }
            }
            Input::Characteristics(Ok(())) => {
                if self.phase == Phase::Discovering {
                    out.push(Action::ReadInfo);
                    out.push(Action::Subscribe);
                    self.enter(Phase::Subscribing);
                    self.deadline = Some(now + ATTACH_TIMEOUT);
                }
            }
            Input::Info(info) => {
                if self.phase == Phase::Subscribing {
                    self.info = Some(info);
                    self.maybe_ready(&mut out);
                }
            }
            Input::Notifying(Ok(())) => {
                if self.phase == Phase::Subscribing {
                    self.notifying = true;
                    self.maybe_ready(&mut out);
                }
            }
            Input::Notifying(Err(why)) => {
                if matches!(self.phase, Phase::Subscribing | Phase::Ready) {
                    self.fail(now, &mut out, why);
                }
            }
            Input::ServicesChanged => {
                if matches!(
                    self.phase,
                    Phase::Discovering | Phase::Subscribing | Phase::Ready
                ) {
                    out.push(Action::DiscoverServices);
                    self.enter(Phase::Discovering);
                    self.deadline = Some(now + ATTACH_TIMEOUT);
                    self.report(&mut out, Report::Attaching);
                }
            }
        }
        if self.wants_tick() && !self.armed {
            self.armed = true;
            out.push(Action::Arm);
        }
        out
    }

    /// The engine holds a peripheral we connected (or are connecting) to.
    fn holds_candidate(&self) -> bool {
        matches!(
            self.phase,
            Phase::Connecting | Phase::Discovering | Phase::Subscribing | Phase::Ready
        )
    }

    fn wants_tick(&self) -> bool {
        !matches!(self.phase, Phase::Down | Phase::Ready)
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        if !matches!(
            phase,
            Phase::Connecting | Phase::Discovering | Phase::Subscribing
        ) {
            self.deadline = None;
        }
        if phase == Phase::Subscribing {
            self.info = None;
            self.notifying = false;
        }
    }

    fn maybe_ready(&mut self, out: &mut Vec<Action>) {
        if let (Some(info), true) = (self.info, self.notifying) {
            self.enter(Phase::Ready);
            self.forgive();
            self.report(out, Report::Ready { info });
        }
    }

    fn fail(&mut self, now: Instant, out: &mut Vec<Action>, why: String) {
        out.push(Action::Release);
        self.enter(Phase::Idle);
        out.push(Action::Problem(why));
        self.report(out, Report::Detached);
        // RETRY, then twice that, and so on up to MAX_BACKOFF.
        let doublings = self.failures.min(8);
        self.failures = self.failures.saturating_add(1);
        let wait = RETRY.saturating_mul(1 << doublings).min(MAX_BACKOFF);
        self.hold_until = Some(now + wait);
    }

    /// Start the failure count again.
    fn forgive(&mut self) {
        self.failures = 0;
        self.hold_until = None;
    }

    fn report(&mut self, out: &mut Vec<Action>, report: Report) {
        if self.reported != report {
            self.reported = report.clone();
            out.push(Action::Report(report));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    const INFO: [u8; 5] = [1, 1, 16, 20, 3];

    /// A machine with the radio on and a candidate found, attaching.
    fn connecting(t0: Instant) -> Attach {
        let mut m = Attach::new();
        assert_eq!(m.step(t0, Input::Radio(Radio::On)), [Find, Arm]);
        assert_eq!(
            m.step(t0, Input::Found),
            [Connect, Report(super::Report::Attaching)]
        );
        m
    }

    fn ready(t0: Instant) -> Attach {
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0, Input::Services(Ok(())));
        m.step(t0, Input::Characteristics(Ok(())));
        m.step(t0, Input::Info(Some(INFO)));
        m.step(t0, Input::Notifying(Ok(())));
        assert_eq!(m.phase(), Phase::Ready);
        // The tick armed at power-on lands while ready and is not renewed.
        assert_eq!(m.step(t0 + RETRY, Input::Tick), []);
        m
    }

    #[test]
    fn the_happy_path_finds_connects_discovers_subscribes_and_is_ready() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        assert_eq!(m.phase(), Phase::Connecting);
        assert!(!m.wants_candidate());
        assert_eq!(m.step(t0, Input::Connected), [DiscoverServices]);
        assert_eq!(
            m.step(t0, Input::Services(Ok(()))),
            [DiscoverCharacteristics]
        );
        assert_eq!(
            m.step(t0, Input::Characteristics(Ok(()))),
            [ReadInfo, Subscribe]
        );
        assert_eq!(m.step(t0, Input::Info(Some(INFO))), []);
        assert_eq!(
            m.step(t0, Input::Notifying(Ok(()))),
            [Report(super::Report::Ready { info: Some(INFO) })]
        );
        // The tick armed at power-on arrives: nothing to do, nothing re-armed.
        assert_eq!(m.step(t0 + RETRY, Input::Tick), []);
    }

    #[test]
    fn info_and_subscription_complete_in_either_order_and_a_failed_read_is_no_info() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0, Input::Services(Ok(())));
        m.step(t0, Input::Characteristics(Ok(())));
        assert_eq!(m.step(t0, Input::Notifying(Ok(()))), []);
        assert_eq!(
            m.step(t0, Input::Info(None)),
            [Report(super::Report::Ready { info: None })]
        );
    }

    #[test]
    fn not_found_scans_and_every_tick_looks_again_until_something_turns_up() {
        let t0 = Instant::now();
        let mut m = Attach::new();
        assert_eq!(m.step(t0, Input::Radio(Radio::On)), [Find, Arm]);
        assert_eq!(
            m.step(t0, Input::NotFound),
            [
                Scan,
                Problem("no OpenWalkieTalkie is connected or advertising".into())
            ]
        );
        assert_eq!(m.phase(), Phase::Scanning);
        assert!(m.wants_candidate());
        // Two seconds on: look again (still scanning), and arm again.
        assert_eq!(m.step(t0 + RETRY, Input::Tick), [Find, Arm]);
        assert_eq!(m.step(t0 + RETRY, Input::NotFound), []);
        // The scan (or the next Find) turns it up.
        assert_eq!(
            m.step(t0 + RETRY, Input::Found),
            [StopScan, Connect, Report(super::Report::Attaching)]
        );
        // The tick that was armed checks the deadline, and re-arms.
        assert_eq!(m.step(t0 + 2 * RETRY, Input::Tick), [Arm]);
    }

    /// `deckd run`'s link only ever attaches to a deck the system already
    /// holds: it never scans, so it never connects to an advertising deck
    /// and never puts a pairing request on the operator's screen. It keeps
    /// looking on every tick, and attaches the moment the system connects
    /// the paired deck.
    #[test]
    fn a_daemon_link_never_scans_and_attaches_when_the_system_connects_the_deck() {
        let t0 = Instant::now();
        assert_eq!(Reach::default(), Reach::Connected, "the safe one");
        let mut m = Attach::reaching(DECK_NAME, Reach::Connected);
        assert_eq!(m.step(t0, Input::Radio(Radio::On)), [Find, Arm]);
        let why = |actions: &[Action]| match actions {
            [Problem(why)] => why.clone(),
            other => panic!("one problem, no scan: {other:?}"),
        };
        let said = why(&m.step(t0, Input::NotFound));
        assert!(said.contains("is connected to this Mac"), "{said}");
        assert!(
            said.contains("never connects to an advertising deck"),
            "{said}"
        );
        assert_eq!(m.phase(), Phase::Idle);
        for n in 1..=3 {
            assert_eq!(m.step(t0 + RETRY * n, Input::Tick), [Find, Arm]);
            assert_eq!(why(&m.step(t0 + RETRY * n, Input::NotFound)), said);
            assert_eq!(m.phase(), Phase::Idle, "looking, not scanning");
        }
        // The system connects the paired deck; the next Find has it.
        assert_eq!(
            m.step(t0 + RETRY * 4, Input::Found),
            [Connect, Report(super::Report::Attaching)],
            "no StopScan: nothing was scanning"
        );
        m.step(t0 + RETRY * 4, Input::Connected);
        m.step(t0 + RETRY * 4, Input::Services(Ok(())));
        m.step(t0 + RETRY * 4, Input::Characteristics(Ok(())));
        m.step(t0 + RETRY * 4, Input::Info(Some(INFO)));
        assert_eq!(
            m.step(t0 + RETRY * 4, Input::Notifying(Ok(()))),
            [Report(super::Report::Ready { info: Some(INFO) })]
        );
        // A drop goes back to looking among connected decks, still without
        // a scan.
        let dropped = m.step(t0 + RETRY * 5, Input::Disconnected("gone".into()));
        assert!(!dropped.contains(&Scan), "{dropped:?}");
        assert_eq!(m.step(t0 + RETRY * 6, Input::Tick), [Find, Arm]);
        assert!(!m.step(t0 + RETRY * 6, Input::NotFound).contains(&Scan));
    }

    #[test]
    fn a_deck_without_the_audio_service_is_released_and_retried() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        let why = "OpenWalkieTalkie has no audio service".to_string();
        assert_eq!(
            m.step(t0, Input::Services(Err(why.clone()))),
            [
                Release,
                Problem(why.clone()),
                Report(super::Report::Detached)
            ],
            "the power-on tick is still armed, so no second Arm"
        );
        assert_eq!(m.phase(), Phase::Idle);
        assert_eq!(m.step(t0 + RETRY, Input::Tick), [Find, Arm]);
        // Same answer again: same failure, reported once.
        m.step(t0 + RETRY, Input::Found);
        m.step(t0 + RETRY, Input::Connected);
        assert_eq!(
            m.step(t0 + RETRY, Input::Services(Err(why.clone()))),
            [
                Release,
                Problem(why.clone()),
                Report(super::Report::Detached)
            ]
        );
        let missing = "the audio service has no mic".to_string();
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0, Input::Services(Ok(())));
        assert_eq!(
            m.step(t0, Input::Characteristics(Err(missing.clone()))),
            [Release, Problem(missing), Report(super::Report::Detached)]
        );
    }

    /// Connect and fail service discovery at `now`, from a machine that is
    /// idle and has just been told the deck is there.
    fn fail_once(m: &mut Attach, now: Instant) {
        m.step(now, Input::Found);
        m.step(now, Input::Connected);
        m.step(now, Input::Services(Err("no audio service".into())));
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn a_deck_that_keeps_failing_is_tried_less_and_less_often() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0, Input::Services(Err("no audio service".into())));
        // RETRY after the first failure, as before.
        let mut at = t0 + RETRY;
        assert_eq!(m.step(at, Input::Tick), [Find, Arm]);
        let mut wait = RETRY;
        for _ in 0..8 {
            fail_once(&mut m, at);
            wait = (wait * 2).min(MAX_BACKOFF);
            // Ticks keep coming, but nothing is tried until the wait is up.
            let mut t = at + RETRY;
            while t < at + wait {
                assert_eq!(
                    m.step(t, Input::Tick),
                    [Arm],
                    "{:?} into a {:?} wait",
                    t - at,
                    wait
                );
                t += RETRY;
            }
            at += wait;
            assert_eq!(m.step(at, Input::Tick), [Find, Arm]);
        }
        assert_eq!(wait, MAX_BACKOFF);
    }

    #[test]
    fn attaching_or_losing_the_deck_starts_the_count_again() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0, Input::Services(Err("no audio service".into())));
        m.step(t0 + RETRY, Input::Tick);
        fail_once(&mut m, t0 + RETRY);
        // Backing off now (4 s); the deck drops out of the connected list.
        let at = t0 + 2 * RETRY + RETRY * 2;
        assert_eq!(m.step(at, Input::Tick), [Find, Arm]);
        m.step(at, Input::NotFound);
        // It comes back: tried at once, and a failure waits RETRY again.
        let at = at + RETRY;
        assert_eq!(m.step(at, Input::Tick), [Find, Arm]);
        fail_once(&mut m, at);
        assert_eq!(m.step(at + RETRY, Input::Tick), [Find, Arm]);
        // Attaching forgives too: a later disconnect retries after RETRY.
        let at = at + RETRY;
        m.step(at, Input::Found);
        m.step(at, Input::Connected);
        m.step(at, Input::Services(Ok(())));
        m.step(at, Input::Characteristics(Ok(())));
        m.step(at, Input::Info(Some(INFO)));
        m.step(at, Input::Notifying(Ok(())));
        assert_eq!(m.phase(), Phase::Ready);
        m.step(at, Input::Disconnected("gone".into()));
        assert_eq!(m.step(at + RETRY, Input::Tick), [Find, Arm]);
    }

    #[test]
    fn an_attach_that_never_finishes_times_out() {
        let t0 = Instant::now();
        let mut m = connecting(t0);
        // The power-on tick: not yet.
        assert_eq!(m.step(t0 + RETRY, Input::Tick), [Arm]);
        assert_eq!(m.phase(), Phase::Connecting);
        assert_eq!(
            m.step(t0 + ATTACH_TIMEOUT, Input::Tick),
            [
                Release,
                Problem("timed out connecting".into()),
                Report(super::Report::Detached),
                Arm
            ]
        );
        // A late answer to the abandoned connect changes nothing.
        assert_eq!(m.step(t0 + ATTACH_TIMEOUT, Input::Connected), []);
        assert_eq!(m.phase(), Phase::Idle);

        // Every step has the same allowance, counted from its own start:
        // progress is not a stall, silence is.
        let mut m = connecting(t0);
        let step = |n: u32| t0 + RETRY * n;
        m.step(step(4), Input::Connected); // 8 s: just in time
        assert_eq!(
            m.step(step(5), Input::Tick),
            [Arm],
            "connect took 8 s, fine"
        );
        m.step(step(8), Input::Services(Ok(()))); // 8 s more
        assert_eq!(m.step(step(9), Input::Tick), [Arm]);
        m.step(step(12), Input::Characteristics(Ok(()))); // and 8 more
        assert_eq!(m.step(step(13), Input::Tick), [Arm]);
        assert_eq!(m.phase(), Phase::Subscribing);
        let actions = m.step(step(12) + ATTACH_TIMEOUT, Input::Tick);
        assert_eq!(
            actions[..2],
            [
                Release,
                Problem("timed out subscribing to the microphone".into())
            ]
        );

        let mut m = connecting(t0);
        m.step(t0, Input::Connected);
        m.step(t0 + RETRY, Input::Tick);
        let actions = m.step(t0 + ATTACH_TIMEOUT, Input::Tick);
        assert_eq!(
            actions[..2],
            [
                Release,
                Problem("timed out discovering the audio service".into())
            ]
        );
    }

    #[test]
    fn a_disconnect_or_a_lost_subscription_goes_back_to_looking() {
        let t0 = Instant::now();
        let mut m = ready(t0);
        assert_eq!(
            m.step(t0, Input::Disconnected("the deck went away".into())),
            [
                Release,
                Problem("the deck went away".into()),
                Report(super::Report::Detached),
                Arm
            ]
        );
        assert_eq!(m.step(t0 + RETRY, Input::Tick), [Find, Arm]);
        // A second disconnect callback for the same link is stale.
        assert_eq!(m.step(t0, Input::Disconnected("again".into())), []);

        let mut m = ready(t0);
        assert_eq!(
            m.step(t0, Input::Notifying(Err("notifications stopped".into())))[..3],
            [
                Release,
                Problem("notifications stopped".into()),
                Report(super::Report::Detached)
            ]
        );
        let mut m = connecting(t0);
        assert_eq!(
            m.step(t0, Input::ConnectFailed("peer refused".into()))[..3],
            [
                Release,
                Problem("peer refused".into()),
                Report(super::Report::Detached)
            ]
        );
    }

    #[test]
    fn the_radio_going_off_releases_and_coming_back_looks_again() {
        let t0 = Instant::now();
        let mut m = ready(t0);
        assert_eq!(
            m.step(t0, Input::Radio(Radio::Off)),
            [
                Release,
                Problem("Bluetooth is off".into()),
                Report(super::Report::Detached)
            ]
        );
        assert_eq!(m.phase(), Phase::Down);
        // No ticks while down; a tick already in flight does nothing.
        assert_eq!(m.step(t0 + RETRY, Input::Tick), []);
        assert_eq!(m.step(t0, Input::Radio(Radio::On)), [Find, Arm]);
        // A second power-on report is not a second Find.
        assert_eq!(m.step(t0, Input::Radio(Radio::On)), []);

        let mut m = Attach::new();
        assert_eq!(
            m.step(t0, Input::Radio(Radio::Unauthorized)),
            [Problem(Radio::Unauthorized.why().into())]
        );
        assert_eq!(Radio::from_raw(5), Radio::On);
        assert_eq!(Radio::from_raw(3), Radio::Unauthorized);
        assert_eq!(Radio::from_raw(42), Radio::Unknown);
    }

    #[test]
    fn changed_services_are_rediscovered_in_place() {
        let t0 = Instant::now();
        let mut m = ready(t0);
        assert_eq!(
            m.step(t0, Input::ServicesChanged),
            [DiscoverServices, Report(super::Report::Attaching), Arm]
        );
        m.step(t0, Input::Services(Ok(())));
        m.step(t0, Input::Characteristics(Ok(())));
        m.step(t0, Input::Notifying(Ok(())));
        assert_eq!(
            m.step(t0, Input::Info(Some(INFO))),
            [Report(super::Report::Ready { info: Some(INFO) })]
        );
        // Not attached: nothing to rediscover.
        let mut m = Attach::new();
        assert_eq!(m.step(t0, Input::ServicesChanged), []);
    }

    #[test]
    fn a_bench_deck_under_another_name_is_looked_for_by_that_name() {
        let t0 = Instant::now();
        let mut m = Attach::named("OpenWalkieTalkie r6");
        m.step(t0, Input::Radio(Radio::On));
        assert_eq!(
            m.step(t0, Input::NotFound),
            [
                Scan,
                Problem("no OpenWalkieTalkie r6 is connected or advertising".into())
            ]
        );
    }
}
