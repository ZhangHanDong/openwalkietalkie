//! `deckd::desktop` (design §5.5, §2.3; HOST-CONTRACT-W5 §3): the Ghostty
//! resolver, the focus-out and the guarded injector — and the seam the core
//! reaches them through.
//!
//! The seam is the one `deckd::hid` and `deckd::audio` already have, for
//! the same reason and with the same asymmetry: a **bounded** channel out
//! of the core that is only ever `try_send`, an **unbounded** one back, and
//! a handle the core owns. The numbers make the case here more plainly than
//! anywhere else in the daemon. M12 measured **~118 ms per AppleScript
//! round trip**; a resolve-and-pin is two of them and an order is two more.
//! A core that ran `Effect::Desktop` inline would stop answering every
//! agent, every hook and every device line for half a second because
//! somebody dictated a sentence — and an `-1712` Ghostty, woken from a long
//! idle, has no published bound at all.
//!
//! So `Effect::Desktop` becomes a [`DesktopJob`] on a queue, a std thread
//! named `deckd-desktop` runs it, and the outcome comes back as
//! [`FromDesktop`] through the core's `select!`. That is also what makes
//! `order.failed{reason}` possible: `order.dispatched` goes out **before**
//! the typing (§5.5), so the refusal has to arrive later, as its own
//! command, or the reducer could never publish it.
//!
//! **There may be no desktop at all.** `--no-device`, a non-macOS build and
//! every unit test that does not want one get [`DesktopHandle::none`], and
//! a daemon holding one behaves exactly as it did before week 5: the job is
//! logged and forgotten, `op.order` still resolves to `via:"inject"`, and
//! the store still writes the order history. "The queue refused it" and
//! "there is nothing to refuse it" stay different states, because only the
//! first is worth telling the operator about.
mod beacon;
pub mod errors;
pub mod fakes;
pub mod fg;
pub mod ghostty;
pub mod inject;
pub mod keys;
pub mod resolve;
pub mod script;

pub use errors::{DesktopError, Fault, ScriptError};
pub use fg::{Foreground, Verdict};
pub use inject::Target;
pub use resolve::{Pinned, Resolution, Resolver, CACHE_TTL};
pub use script::Scripting;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use deck_model::AgentId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// Capacity of the core → desktop channel.
///
/// Small, like the audio one and for the same reason: every job in the
/// queue is already late by 118 ms times however many are in front of it,
/// and a hundred queued orders are not better than a counter saying a
/// hundred were dropped. `op.order` is rate-limited to 2/s per external
/// principal (§11), so a full queue here means Ghostty is wedged.
pub const CMDS_CAPACITY: usize = 32;

type RefreshBeacon = Arc<dyn Fn(&Target) -> Result<String, DesktopError> + Send + Sync>;

/// What the core asks the desktop side to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopJob {
    /// `op.order` resolving to `via:"inject"`. Guards 1 (partly: the
    /// `Target` exists, so provenance passed) and 2's card half already
    /// ran in the core; guard 2's roster half and both guard-3 checks run
    /// out here.
    Inject {
        agent: AgentId,
        target: Target,
        text: String,
        /// The card half of guard 2, as the reducer saw it. `true` means
        /// the store already refused and this job must not exist; it is
        /// carried so the worker can assert rather than assume.
        dialog_open: bool,
    },
    /// `op.focus` / `agent.focus` / a device `channel`.
    Focus { agent: AgentId, target: Target },
    /// A session ended: unpin its tab and forget it (§5.5 step 7).
    Release { key: String },
    /// Shutdown and `integrations uninstall`: unpin everything (§6).
    ReleaseAll,
    /// A Macropad key's keystroke, posted as it is (`keys::press`).
    Keystroke { keys: String },
    /// `perm.request{which:"accessibility"}`: read the grant, with macOS's
    /// prompt where prompts are allowed (once per run).
    Accessibility,
}

/// What the desktop side tells the core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromDesktop {
    /// The injector chain finished. `Ok` means the text and the Return both
    /// went to the resolved terminal; `Err` is the `order.failed{reason}`
    /// the reducer publishes.
    Order {
        agent: AgentId,
        text: String,
        result: Result<(), DesktopError>,
    },
    /// The focus attempt finished. Silent on the device (the `focus{ack}`
    /// already went out, §6.4) and visible in the GUI.
    Focus {
        agent: AgentId,
        selected: bool,
        reason: Option<String>,
    },
    /// Automation TCC's verdict changed (`-1743`, or a call that worked
    /// after one). `perm.status.automation.ghostty` and
    /// `integrations.status` read it; §5.5's "denied → the injector chain
    /// skips Ghostty" is this flag.
    Automation { denied: bool },
    /// The Accessibility grant, as the Macropad keys last learned it: a
    /// keystroke went out (`granted`) or macOS refused one (`denied`), or
    /// `perm.request` asked. `perm.status.accessibility` reads it.
    Accessibility { granted: bool },
    /// `terminal_map` changed and `state.json` should be written (§5.5
    /// step 4).
    Pins(BTreeMap<String, serde_json::Value>),
}

/// What `deckd run` needs to start the desktop side.
#[derive(Clone, Debug)]
pub struct DesktopConfig {
    /// `[desktop] inject` — the terminal programs injection is enabled
    /// for.
    pub inject: Vec<String>,
    /// `[desktop] focus`.
    pub focus: bool,
    /// `[liveness] claude_bin`, for guard 2's roster half.
    pub claude_bin: Option<String>,
    /// Run AppleScript in process on the main thread (M12) rather than
    /// spawning `osascript`. `false` in every test binary, because libtest
    /// owns `main` and never pumps a run loop.
    pub in_process: bool,
    /// May put macOS's own permission dialogs on the operator's screen (the
    /// Accessibility prompt, once per run, the first time a Macropad key
    /// is refused). `false` in every test binary.
    pub prompts: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        DesktopConfig {
            inject: vec![ghostty::APP.to_lowercase()],
            focus: true,
            claude_bin: None,
            in_process: false,
            prompts: false,
        }
    }
}

impl DesktopConfig {
    /// From the daemon's own config.
    #[must_use]
    pub fn from_config(cfg: &crate::config::Config) -> DesktopConfig {
        DesktopConfig {
            inject: cfg.desktop.inject.clone(),
            focus: cfg.desktop.focus,
            claude_bin: cfg
                .liveness
                .enabled
                .then(|| cfg.liveness.claude_bin.clone()),
            in_process: true,
            prompts: true,
        }
    }
}

/// The desktop side itself: one runner, one foreground checker, one
/// resolver, and the policy that joins them.
///
/// It is a plain struct with no thread of its own so that every test drives
/// it directly — `on_job` in, `FromDesktop` out, on a clock the test owns.
/// [`run`] is the loop that turns it into `deckd-desktop`.
pub struct Desktop {
    script: Arc<dyn Scripting>,
    fg: Arc<dyn Foreground>,
    resolver: Resolver,
    cfg: DesktopConfig,
    roster: Option<crate::liveness::Probe>,
    /// Whether the last Apple Event was refused `-1743`. Sticky until one
    /// succeeds: §5.5's chain "skips Ghostty" while it is set, because
    /// re-asking a denied grant only trains the operator to click Deny.
    denied: bool,
    /// The Accessibility grant as last learned (`None` until a key was
    /// pressed or `perm.request` asked); a change is reported to the core.
    accessibility: Option<bool>,
    /// The Accessibility prompt has been put up this run: once is a
    /// request, twice is nagging.
    prompted: bool,
    /// How a keystroke is posted: [`keys::press`], or a test's stand-in
    /// (a real press on the test machine would land in whatever has focus).
    press: fn(&str) -> Result<(), keys::KeyError>,
    /// Tty output is enabled only by the real constructor; scripted
    /// desktops never touch the operator's terminals.
    refresh_beacon: Option<RefreshBeacon>,
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desktop")
            .field("script", &self.script.name())
            .field("pins", &self.resolver.pins().len())
            .field("denied", &self.denied)
            .finish()
    }
}

impl Desktop {
    /// The daemon's own: the configured runner and the real
    /// `proc_pidinfo` check.
    #[must_use]
    pub fn new(cfg: DesktopConfig) -> Desktop {
        let script = script::default_runner(cfg.in_process);
        let mut desktop = Desktop::with(script, Arc::new(fg::Procs), cfg);
        desktop.refresh_beacon = Some(Arc::new(beacon::refresh));
        desktop
    }

    /// A desktop over any runner and any foreground checker: what every
    /// test builds, and what the hardware step builds with the real pair.
    #[must_use]
    pub fn with(
        script: Arc<dyn Scripting>,
        fg: Arc<dyn Foreground>,
        cfg: DesktopConfig,
    ) -> Desktop {
        let roster = cfg.claude_bin.as_deref().map(crate::liveness::Probe::new);
        Desktop {
            script,
            fg,
            resolver: Resolver::new(),
            cfg,
            roster,
            denied: false,
            accessibility: None,
            prompted: false,
            press: keys::press,
            refresh_beacon: None,
        }
    }

    /// Post keystrokes through `press` instead of the desktop (tests).
    #[cfg(test)]
    fn set_presser(&mut self, press: fn(&str) -> Result<(), keys::KeyError>) {
        self.press = press;
    }

    /// Note what the Accessibility grant turned out to be; `true` when
    /// that is news for the core.
    fn learned_accessibility(&mut self, granted: bool) -> bool {
        let changed = self.accessibility != Some(granted);
        self.accessibility = Some(granted);
        changed
    }

    /// Ask macOS for the Accessibility grant: its own dialog, at most once
    /// per run and only where prompts are allowed; otherwise a plain read.
    fn ask_accessibility(&mut self) -> Option<bool> {
        let prompt = self.cfg.prompts && !self.prompted;
        if prompt {
            self.prompted = true;
            tracing::info!("asking macOS for the Accessibility grant (System Settings > Privacy & Security > Accessibility)");
        }
        keys::trusted(prompt)
    }

    /// Use `probe` for guard 2's roster half — the tests' hook, so a fake
    /// `claude` on a temp `PATH` answers instead of the operator's own.
    pub fn set_roster(&mut self, probe: Option<crate::liveness::Probe>) {
        self.roster = probe;
    }

    /// Seed `terminal_map` from `state.json` at start-up.
    pub fn restore(&mut self, stored: &BTreeMap<String, serde_json::Value>) {
        self.resolver.restore(stored);
    }

    #[must_use]
    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }

    /// Whether Automation is currently denied (`perm.status`).
    #[must_use]
    pub const fn denied(&self) -> bool {
        self.denied
    }

    /// Run one job. `Vec` rather than `Option` because a job can both
    /// finish and change the pins, and `state.json` must not wait for the
    /// next one.
    pub fn on_job(&mut self, job: DesktopJob, now: Instant) -> Vec<FromDesktop> {
        let pins_before = self.resolver.pins();
        let denied_before = self.denied;
        let mut out = match job {
            DesktopJob::Inject {
                agent,
                target,
                text,
                dialog_open,
            } => {
                let result = self.order(&target, &text, dialog_open, now);
                vec![FromDesktop::Order {
                    agent,
                    text,
                    result,
                }]
            }
            DesktopJob::Focus { agent, target } => {
                let (selected, reason) = match self.focus(&target, now) {
                    Ok(selected) => (selected, None),
                    Err(e) => (false, Some(e.reason().to_string())),
                };
                vec![FromDesktop::Focus {
                    agent,
                    selected,
                    reason,
                }]
            }
            DesktopJob::Release { key } => {
                if let Err(e) = self.resolver.release(&*self.script, &key) {
                    self.note(&e);
                    tracing::debug!(%key, error = %e, "unpin on session end failed");
                }
                Vec::new()
            }
            DesktopJob::ReleaseAll => {
                let cleared = self.resolver.release_all(&*self.script);
                tracing::info!(cleared, "unpinned every tab this daemon pinned");
                Vec::new()
            }
            DesktopJob::Keystroke { keys } => {
                let learned = match (self.press)(&keys) {
                    Ok(()) => {
                        tracing::debug!(%keys, "macro keystroke posted");
                        Some(true)
                    }
                    Err(keys::KeyError::NotTrusted) => {
                        // The refusal is the moment to ask: macOS lists the
                        // process under Accessibility and the operator
                        // switches it on; the next press goes through.
                        let _ = self.ask_accessibility();
                        tracing::warn!(%keys, error = %keys::KeyError::NotTrusted, "macro keystroke not posted");
                        Some(false)
                    }
                    Err(e) => {
                        tracing::warn!(%keys, error = %e, "macro keystroke not posted");
                        None
                    }
                };
                let mut out = Vec::new();
                if let Some(granted) = learned {
                    if self.learned_accessibility(granted) {
                        out.push(FromDesktop::Accessibility { granted });
                    }
                }
                out
            }
            DesktopJob::Accessibility => {
                let mut out = Vec::new();
                if let Some(granted) = self.ask_accessibility() {
                    if self.learned_accessibility(granted) {
                        out.push(FromDesktop::Accessibility { granted });
                    }
                }
                out
            }
        };
        if self.denied != denied_before {
            out.push(FromDesktop::Automation {
                denied: self.denied,
            });
        }
        if self.resolver.pins() != pins_before {
            out.push(FromDesktop::Pins(self.resolver.terminal_map()));
        }
        out
    }

    /// The injector chain: `[desktop] inject`, guard 2's roster half, the
    /// resolution, then [`inject::type_into`] with guard 3 on both sides of
    /// the two verbs.
    fn order(
        &mut self,
        target: &Target,
        text: &str,
        dialog_open: bool,
        now: Instant,
    ) -> Result<(), DesktopError> {
        if !inject::enabled_for(&target.program, &self.cfg.inject) {
            return Err(DesktopError::Disabled);
        }
        if self.denied {
            // §5.5: denied → the chain skips Ghostty. The order is kept for
            // the GUI with Copy, exactly as `dialog_open` is.
            return Err(DesktopError::Script(ScriptError::Osa {
                number: errors::ERR_NOT_PERMITTED,
                message: "Automation is denied for Ghostty".into(),
            }));
        }
        if dialog_open {
            // The reducer refuses this before `order.dispatched` is ever
            // written, so a job that carries it is a routing bug. Refusing
            // again is the cheap half of not having to trust that.
            return Err(DesktopError::DialogOpen);
        }
        if let (Some(probe), Some(session)) =
            (self.roster.as_ref(), inject::claude_session_of(&target.key))
        {
            // One process spawn (0.12–0.17 s measured with six sessions),
            // uncached on purpose: a cached "not waiting" is exactly the
            // stale fact guard 2 exists to prevent, and `op.order` is
            // rate-limited to 2/s per principal (§11).
            //
            // An `Err` is **inconclusive and not a refusal**: `liveness`'s
            // rule 1 is that absence of evidence is not evidence, and a
            // missing `claude` binary must not make every order fail. The
            // card half of guard 2 has already run either way.
            match probe.rows() {
                Ok(rows) => {
                    if let Some(why) = inject::roster_waiting(&rows, session) {
                        tracing::info!(%session, why, "guard 2 refused: the CLI is waiting");
                        return Err(DesktopError::DialogOpen);
                    }
                }
                Err(e) => tracing::debug!(error = %e, "the roster probe was inconclusive"),
            }
        }
        let resolution = self.resolve(target, now)?;
        let typed = inject::type_into(&*self.script, &*self.fg, target, &resolution.terminal, text);
        if let Err(e) = &typed {
            self.note(e);
            if e.invalidates_resolution() {
                self.resolver.invalidate(&target.key);
            }
        }
        typed
    }

    /// `activate` + `goto_tab`, with the `NSWorkspace` fallback §5.5 names.
    fn focus(&mut self, target: &Target, now: Instant) -> Result<bool, DesktopError> {
        if !self.cfg.focus {
            return Err(DesktopError::Disabled);
        }
        let resolution = self.resolve(target, now)?;
        match self.wake_and_retry(|s| ghostty::focus(s, &resolution.terminal)) {
            Ok(selected) => Ok(selected),
            Err(e) => {
                self.note(&DesktopError::Script(e.clone()));
                if errors::ScriptError::fault(&e) == Fault::Stale {
                    self.resolver.invalidate(&target.key);
                }
                // The app could not be told to come forward; raise it by
                // bundle id instead. It cannot select a tab, which is why
                // it is the fallback — but a Ghostty the operator can see
                // is better than a silent nothing.
                ghostty::activate_by_bundle_id().map(|()| false)
            }
        }
    }

    /// Resolve, with §5.5 step 6's single beacon retry spent here rather
    /// than left to the caller.
    fn resolve(&mut self, target: &Target, now: Instant) -> Result<Resolution, DesktopError> {
        let session = resolve::Session {
            agent: &target.agent,
            key: &target.key,
            nonce: target.nonce.as_deref(),
            id4: &target.id4,
            tty: Some(&target.tty),
            pid: Some(target.pid),
        };
        let attempt = |me: &mut Self, nonce: Option<&str>| {
            // **`-1712` belongs here and nowhere else in this chain.** §5.5's
            // story is an instance that "refused 4/4 element queries with its
            // main thread provably idle", and an element query is exactly
            // what `enumerate` is — so the wake-and-retry wraps the
            // resolution's two calls. It deliberately does **not** wrap
            // `input text`/`send key`: a timeout says nothing about whether
            // the event was delivered, so retrying those risks typing the
            // operator's sentence twice, or sending a second Return.
            let waking = WakeOnce(Arc::clone(&me.script));
            me.resolver
                .resolve(&waking, &resolve::Session { nonce, ..session }, now)
        };
        match attempt(self, session.nonce) {
            Ok(resolution) => Ok(resolution),
            Err(first) => {
                self.note(&first);
                // One retry, and only for an ambiguity: a `-1743` will not
                // become an `Ok` by being asked twice.
                if !matches!(first, DesktopError::Ambiguous(_))
                    || !self.resolver.may_retry(&target.key)
                {
                    return Err(first);
                }
                // SessionStart's title can be overwritten before the first
                // order. Refresh it through the verified tty's output, then
                // locate that exact surface; never guess by title or cwd.
                let nonce = if let Some(refresh) = &self.refresh_beacon {
                    if !self.fg.check(target.pid, &target.tty).allows() {
                        return Err(DesktopError::NotForeground);
                    }
                    Some(refresh(target)?)
                } else {
                    None
                };
                let again = attempt(self, nonce.as_deref().or(session.nonce));
                if let Err(e) = &again {
                    self.note(e);
                }
                again
            }
        }
    }

    /// Run `f`; on `-1712` activate Ghostty and run it **once** more
    /// (§5.5: "`-1712` is not 'no Ghostty' … activating it fixed the
    /// session permanently").
    ///
    /// Bounded by construction: one match arm, no loop and no recursion, so
    /// a Ghostty that answers `-1712` for ever costs two attempts and one
    /// `activate` per job and never spins.
    fn wake_and_retry<T>(
        &self,
        f: impl Fn(&dyn Scripting) -> Result<T, ScriptError>,
    ) -> Result<T, ScriptError> {
        match f(&*self.script) {
            Err(e) if e.wake_and_retry() => {
                tracing::info!("ghostty answered -1712; activating and retrying once");
                let _ = self.script.run(&format!(
                    "tell application \"{}\" to activate\n",
                    ghostty::APP
                ));
                f(&*self.script)
            }
            other => other,
        }
    }

    /// Fold one outcome into the sticky Automation verdict.
    fn note(&mut self, e: &DesktopError) {
        if let DesktopError::Script(s) = e {
            if s.denied() && !self.denied {
                tracing::warn!("Automation is denied for Ghostty; the injector chain will skip it");
                self.denied = true;
            }
        }
    }

    /// A call worked: whatever `-1743` we remembered is stale.
    ///
    /// Called by the worker loop's idle probe rather than by every success,
    /// because "the grant came back" is a rare edge and a per-call clear
    /// would have the flag flapping across a partly-denied chain.
    pub fn clear_denial(&mut self) {
        self.denied = false;
    }
}

/// A runner that answers `-1712` the way §5.5 says to: `activate`, then
/// **one** more attempt.
///
/// A wrapper rather than a branch inside [`Resolver::resolve`] because the
/// rule is per *call* and the resolver makes two of them, and because it is
/// the only shape that makes the bound structural: the retry goes to the
/// inner runner, never back through `self`, so no sequence of `-1712`s can
/// make this recurse. Worst case per call is three scripts — the attempt,
/// the `activate`, the retry.
#[derive(Debug)]
struct WakeOnce(Arc<dyn Scripting>);

impl Scripting for WakeOnce {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        match self.0.run(src) {
            Err(e) if e.wake_and_retry() => {
                tracing::info!("ghostty answered -1712; activating and retrying once");
                let _ = self.0.run(&format!(
                    "tell application \"{}\" to activate\n",
                    ghostty::APP
                ));
                self.0.run(src)
            }
            other => other,
        }
    }

    fn name(&self) -> &'static str {
        self.0.name()
    }
}

/// The core's handle on the desktop side.
pub struct DesktopHandle {
    /// Bounded; `try_send` only. `None` when this build has no desktop.
    cmds: Option<Sender<DesktopJob>>,
    /// Unbounded: an outcome per order, and orders are rate-limited.
    pub events: Receiver<FromDesktop>,
    /// A sender kept for the handle's whole life so `events` can never
    /// become `Disconnected` — a disconnected `select!` arm is always
    /// ready, and the core would spin at 100% CPU for the rest of the
    /// daemon's life (the same trap `AudioHandle` documents).
    _idle: Sender<FromDesktop>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for DesktopHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopHandle")
            .field("engine", &self.engine())
            .field("queued", &self.queued())
            .finish()
    }
}

impl DesktopHandle {
    /// The core's outcome channel, for a worker that is not the desktop
    /// thread: the inbox poster (`crate::inbox::post_async`) reports an
    /// order's result through the same [`FromDesktop::Order`] the injector
    /// uses, so `order.failed` has one path into the reducer. Exists on
    /// every handle, `none()` included — posting into a session's socket
    /// needs no AppleScript engine and must work in a `--no-device` daemon.
    #[must_use]
    pub fn outcomes(&self) -> Sender<FromDesktop> {
        self._idle.clone()
    }

    /// No desktop: every job is logged and forgotten.
    #[must_use]
    pub fn none() -> DesktopHandle {
        let (idle, events) = crossbeam_channel::unbounded::<FromDesktop>();
        DesktopHandle {
            cmds: None,
            events,
            _idle: idle,
            join: None,
        }
    }

    /// A handle with no thread behind it: the caller holds the other ends,
    /// sees every job the core sends and can answer them. The bounded
    /// capacity is the real one, so a test can fill it.
    #[must_use]
    pub fn detached() -> (DesktopHandle, Receiver<DesktopJob>, Sender<FromDesktop>) {
        let (cmds_tx, cmds_rx) = crossbeam_channel::bounded::<DesktopJob>(CMDS_CAPACITY);
        let (events_tx, events_rx) = crossbeam_channel::unbounded::<FromDesktop>();
        (
            DesktopHandle {
                cmds: Some(cmds_tx),
                events: events_rx,
                _idle: events_tx.clone(),
                join: None,
            },
            cmds_rx,
            events_tx,
        )
    }

    #[must_use]
    pub fn engine(&self) -> bool {
        self.cmds.is_some()
    }

    #[must_use]
    pub fn queued(&self) -> usize {
        self.cmds.as_ref().map_or(0, crossbeam_channel::Sender::len)
    }

    /// `try_send`, never `send`: `None` when the desktop side took it, and
    /// **the job back** when it did not, so the caller can apply its own
    /// policy. A disconnected worker is a full one as far as that policy is
    /// concerned.
    #[must_use]
    pub fn send_or_return(&self, job: DesktopJob) -> Option<DesktopJob> {
        match self.cmds.as_ref() {
            None => Some(job),
            Some(tx) => match tx.try_send(job) {
                Ok(()) => None,
                Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => Some(job),
            },
        }
    }

    /// Dropping the job sender ends the thread; join it.
    pub fn join(mut self) {
        drop(self.cmds.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Default for DesktopHandle {
    fn default() -> Self {
        DesktopHandle::none()
    }
}

/// Start `deckd-desktop`.
///
/// A std thread and not a tokio task, for the reason §2.3 gives the audio
/// control loop: everything it does is blocking (a child `osascript`, a
/// main-thread hop, a `proc_pidinfo`), and a blocking job on a tokio worker
/// stalls every door behind it. It is a *thread* rather than
/// `spawn_blocking` because it owns the resolver's cache and map, which
/// have to outlive one job.
#[must_use]
pub fn spawn(cfg: DesktopConfig, stored: &BTreeMap<String, serde_json::Value>) -> DesktopHandle {
    let (cmds_tx, cmds_rx) = crossbeam_channel::bounded::<DesktopJob>(CMDS_CAPACITY);
    let (events_tx, events_rx) = crossbeam_channel::unbounded::<FromDesktop>();
    let mut desktop = Desktop::new(cfg);
    desktop.restore(stored);
    let idle = events_tx.clone();
    let join = std::thread::Builder::new()
        .name("deckd-desktop".into())
        .spawn(move || run(desktop, &cmds_rx, &events_tx))
        .ok();
    DesktopHandle {
        cmds: Some(cmds_tx),
        events: events_rx,
        _idle: idle,
        join,
    }
}

/// The `deckd-desktop` loop: one job at a time, in arrival order.
///
/// Serial on purpose. Two concurrent AppleScripts against one Ghostty is
/// not something M12 measured and not something the dictionary promises,
/// and the resolver's cache is the reason there is rarely more than one job
/// in flight anyway.
pub fn run(mut desktop: Desktop, jobs: &Receiver<DesktopJob>, out: &Sender<FromDesktop>) {
    while let Ok(job) = jobs.recv() {
        for event in desktop.on_job(job, Instant::now()) {
            if out.send(event).is_err() {
                return; // the core is gone
            }
        }
    }
    // The core dropped the sender: shutdown. §6's rule is to leave the
    // operator's Ghostty as we found it, and this is the last chance.
    let cleared = desktop.resolver.release_all(&*desktop.script);
    if cleared > 0 {
        tracing::info!(cleared, "unpinned on shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::fakes::{FakeForeground, FakeGhostty};

    fn target() -> Target {
        Target {
            agent: "ag_cd34".into(),
            key: "claude:sess-1".into(),
            nonce: Some("n0nce".into()),
            id4: "cd34".into(),
            tty: "/dev/ttys004".into(),
            pid: 4321,
            program: "ghostty".into(),
        }
    }

    fn agent() -> AgentId {
        AgentId("ag_cd34".into())
    }

    fn rig(fg: FakeForeground) -> (FakeGhostty, Desktop) {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-MINE", "n0nce")
            .with_beacon("UUID-OTHER", "zzzz");
        let handle = gh.handle();
        let desktop = Desktop::with(
            Arc::new(handle),
            Arc::new(fg),
            DesktopConfig {
                // No roster probe: guard 2's roster half is the operator's
                // own `claude`, and a unit test does not run it.
                claude_bin: None,
                ..DesktopConfig::default()
            },
        );
        (gh, desktop)
    }

    #[test]
    fn overwritten_titles_are_resolved_by_a_fresh_tty_beacon() {
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        gh.osc2("UUID-MINE", "Claude is working");
        let writer = gh.handle();
        desktop.refresh_beacon = Some(Arc::new(move |target| {
            assert_eq!(target.tty, "/dev/ttys004");
            writer.osc2("UUID-MINE", &ghostty::beacon_name("fresh-nonce"));
            Ok("fresh-nonce".into())
        }));
        let mut target = target();
        target.nonce = None; // A session hooked after SessionStart also works.
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target,
                text: "Please continue.".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        assert!(
            matches!(&out[0], FromDesktop::Order { result: Ok(()), .. }),
            "{out:?}"
        );
        assert_eq!(
            gh.typed(),
            vec![("UUID-MINE".into(), "Please continue.".into())]
        );
        assert_eq!(gh.keys(), vec![("UUID-MINE".into(), "enter".into())]);
    }

    #[test]
    fn refreshing_a_title_cannot_bypass_foreground_or_dialog_guards() {
        for (verdict, dialog_open, refusal) in [
            (Verdict::Dead, false, DesktopError::NotForeground),
            (Verdict::OtherTty, false, DesktopError::NotForeground),
            (Verdict::Foreground, true, DesktopError::DialogOpen),
        ] {
            let (gh, mut desktop) = rig(FakeForeground::always(verdict));
            gh.osc2("UUID-MINE", "Claude is working");
            desktop.refresh_beacon = Some(Arc::new(|_| panic!("must not touch the tty")));
            let out = desktop.on_job(
                DesktopJob::Inject {
                    agent: agent(),
                    target: target(),
                    text: "Please continue.".into(),
                    dialog_open,
                },
                Instant::now(),
            );
            assert!(
                matches!(&out[0], FromDesktop::Order { result: Err(e), .. } if *e == refusal),
                "{out:?}"
            );
            assert!(gh.typed().is_empty());
            assert!(gh.keys().is_empty());
        }
    }

    #[test]
    fn a_refreshed_beacon_still_requires_an_exact_surface_match() {
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        gh.osc2("UUID-MINE", "Claude is working");
        desktop.refresh_beacon = Some(Arc::new(|_| Ok("not-visible-in-this-instance".into())));
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "Please continue.".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        assert!(matches!(
            &out[0],
            FromDesktop::Order {
                result: Err(DesktopError::Ambiguous(0)),
                ..
            }
        ));
        assert!(gh.typed().is_empty());
        assert!(gh.keys().is_empty());
    }

    /// The whole happy chain, as one job: resolve, pin, guard 3, text,
    /// guard 3, Return — and the pins reported for `state.json`.
    #[test]
    fn an_order_resolves_pins_guards_and_types_into_the_aimed_terminal() {
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "deploy the api".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        assert_eq!(
            out[0],
            FromDesktop::Order {
                agent: agent(),
                text: "deploy the api".into(),
                result: Ok(()),
            }
        );
        assert_eq!(gh.typed_into("UUID-MINE"), "deploy the api");
        assert_eq!(gh.typed_into("UUID-OTHER"), "");
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("deck:cd34"));
        // …and the map change is reported, so `state.json` is written now
        // and not at the next order.
        let pins = out
            .iter()
            .find_map(|e| match e {
                FromDesktop::Pins(map) => Some(map.clone()),
                _ => None,
            })
            .expect("the resolution changed the terminal map");
        assert_eq!(pins["UUID-MINE"]["key"], "claude:sess-1");
        assert!(!desktop.denied());
    }

    /// The three `order.failed{reason}` words §4.12 publishes, each from
    /// the guard that produces it.
    #[test]
    fn each_guard_fails_the_order_with_the_reason_the_wire_publishes() {
        // Guard 3.
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Background));
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "x".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "not_foreground");
        assert!(gh.typed().is_empty(), "nothing was sent");

        // Guard 2, as the reducer decided it.
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "x".into(),
                dialog_open: true,
            },
            Instant::now(),
        );
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "dialog_open");
        assert!(gh.sources().is_empty(), "not even an enumeration");

        // Ambiguous: the beacon matched nothing, twice (one retry).
        let empty = FakeGhostty::new();
        empty.with_surface("UUID-A", "zsh");
        let mut desktop = Desktop::with(
            Arc::new(empty.handle()),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig::default(),
        );
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "x".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "ambiguous");
        assert!(empty.typed().is_empty(), "never the wrong session");
        // §5.5 step 6: the beacon was retried exactly once.
        let enumerations = empty
            .sources()
            .into_iter()
            .filter(|s| s.contains("name of tm"))
            .count();
        assert_eq!(enumerations, 2);
    }

    /// `[desktop] inject` and `[desktop] focus` are both enforced here,
    /// not only advertised in `config.get`.
    #[test]
    fn an_unlisted_terminal_program_and_a_disabled_focus_are_both_refused() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-MINE", "n0nce");
        let mut desktop = Desktop::with(
            Arc::new(gh.handle()),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig {
                inject: vec!["iterm".into()],
                focus: false,
                claude_bin: None,
                in_process: false,
                prompts: false,
            },
        );
        let out = desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "x".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "disabled");
        let out = desktop.on_job(
            DesktopJob::Focus {
                agent: agent(),
                target: target(),
            },
            Instant::now(),
        );
        assert_eq!(
            out[0],
            FromDesktop::Focus {
                agent: agent(),
                selected: false,
                reason: Some("disabled".into()),
            }
        );
        assert!(gh.sources().is_empty(), "no round trip for either");
    }

    /// A Macropad key's keystroke reports the Accessibility grant as it
    /// learns it: a refusal is `denied` (and, where prompts are allowed,
    /// the moment macOS is asked), a press that went out is `granted`, an
    /// unknown combo says nothing, and the same answer twice is reported
    /// once.
    #[test]
    fn a_keystroke_reports_the_accessibility_grant_once_per_change() {
        let (_gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        fn refused(_: &str) -> Result<(), keys::KeyError> {
            Err(keys::KeyError::NotTrusted)
        }
        fn posted(_: &str) -> Result<(), keys::KeyError> {
            Ok(())
        }
        fn unknown(_: &str) -> Result<(), keys::KeyError> {
            Err(keys::KeyError::Unknown("cmd+ptt".into()))
        }
        let press = || DesktopJob::Keystroke {
            keys: "cmd+c".into(),
        };
        desktop.set_presser(refused);
        assert_eq!(
            desktop.on_job(press(), Instant::now()),
            [FromDesktop::Accessibility { granted: false }]
        );
        assert!(
            desktop.on_job(press(), Instant::now()).is_empty(),
            "the same news is not repeated"
        );
        desktop.set_presser(unknown);
        assert!(
            desktop.on_job(press(), Instant::now()).is_empty(),
            "an unknown combo says nothing about the grant"
        );
        desktop.set_presser(posted);
        assert_eq!(
            desktop.on_job(press(), Instant::now()),
            [FromDesktop::Accessibility { granted: true }]
        );
        assert!(!desktop.prompted, "no prompt in a test binary");
    }

    /// `op.focus` resolves, activates, goes to the tab and reports whether
    /// the selection landed.
    #[test]
    fn focus_reports_whether_the_target_tab_became_the_selected_one() {
        let (_gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        let out = desktop.on_job(
            DesktopJob::Focus {
                agent: agent(),
                target: target(),
            },
            Instant::now(),
        );
        assert_eq!(
            out[0],
            FromDesktop::Focus {
                agent: agent(),
                selected: true,
                reason: None,
            }
        );
    }

    /// `-1743`: the flag is raised once, reported once, and the chain
    /// stops talking to Ghostty afterwards (§5.5). M13: this is the only
    /// evidence that path works.
    #[test]
    fn an_automation_denial_is_reported_once_and_then_skips_ghostty_entirely() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-MINE", "n0nce")
            .fail_always(ScriptError::Osa {
                number: errors::ERR_NOT_PERMITTED,
                message: "Not authorized to send Apple events to Ghostty.".into(),
            });
        let mut desktop = Desktop::with(
            Arc::new(gh.handle()),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig::default(),
        );
        let job = || DesktopJob::Inject {
            agent: agent(),
            target: target(),
            text: "x".into(),
            dialog_open: false,
        };
        let first = desktop.on_job(job(), Instant::now());
        let FromDesktop::Order { result, .. } = &first[0] else {
            panic!("{first:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "automation_denied");
        assert!(first.contains(&FromDesktop::Automation { denied: true }));
        assert!(desktop.denied());

        let calls = gh.sources().len();
        let second = desktop.on_job(job(), Instant::now());
        let FromDesktop::Order { result, .. } = &second[0] else {
            panic!("{second:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "automation_denied");
        assert_eq!(gh.sources().len(), calls, "the chain skipped Ghostty");
        assert!(
            !second.contains(&FromDesktop::Automation { denied: true }),
            "reported on the edge, not on every order"
        );
        // A grant that comes back is a state change, reported the same way.
        desktop.clear_denial();
        assert!(!desktop.denied());
    }

    /// `-1712`: activate, then exactly one retry (§5.5). The first call
    /// fails, the activate goes out, the second call succeeds.
    #[test]
    fn a_minus_1712_is_answered_by_an_activate_and_one_retry() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-MINE", "n0nce").fail_on(
            "goto_tab:",
            ScriptError::Osa {
                number: errors::ERR_TIMEOUT,
                message: "AppleEvent timed out".into(),
            },
        );
        let mut desktop = Desktop::with(
            Arc::new(gh.handle()),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig::default(),
        );
        let out = desktop.on_job(
            DesktopJob::Focus {
                agent: agent(),
                target: target(),
            },
            Instant::now(),
        );
        assert_eq!(
            out[0],
            FromDesktop::Focus {
                agent: agent(),
                selected: true,
                reason: None,
            },
            "the retry succeeded"
        );
        let sources = gh.sources();
        let activates = sources
            .iter()
            .filter(|s| *s == "tell application \"Ghostty\" to activate\n")
            .count();
        assert_eq!(activates, 1, "activated once, retried once");
        assert!(!desktop.denied(), "-1712 is not a denial");
    }

    /// **`-1712` on the enumeration**, which is the call §5.5's story is
    /// actually about ("refused 4/4 **element queries** with its main
    /// thread provably idle"), and the bound on the retry.
    ///
    /// Two halves. A Ghostty that is asleep once is woken and the order
    /// goes through; a Ghostty that answers `-1712` for ever costs a fixed
    /// number of scripts and fails — it does not spin. And the two typing
    /// verbs are **not** retried, on purpose: a timeout says nothing about
    /// whether the Apple Event was delivered, so a retry there could type
    /// the operator's sentence twice or send a second Return.
    #[test]
    fn a_minus_1712_on_the_element_query_is_woken_once_and_never_loops() {
        let asleep = || ScriptError::Osa {
            number: errors::ERR_TIMEOUT,
            message: "AppleEvent timed out".into(),
        };
        let activates = |gh: &FakeGhostty| {
            gh.sources()
                .iter()
                .filter(|s| *s == "tell application \"Ghostty\" to activate\n")
                .count()
        };
        let job = || DesktopJob::Inject {
            agent: agent(),
            target: target(),
            text: "deploy the api".into(),
            dialog_open: false,
        };

        // Asleep once: activated, asked again, and the order lands.
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        gh.fail_on("name of tm", asleep());
        let out = desktop.on_job(job(), Instant::now());
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(*result, Ok(()), "the wake fixed it");
        assert_eq!(activates(&gh), 1, "activated once");
        assert_eq!(gh.typed_into("UUID-MINE"), "deploy the api");

        // Asleep for ever: bounded. Two attempts and one activate for the
        // enumeration, and then the order fails rather than being asked a
        // third time.
        let stuck = FakeGhostty::new();
        stuck
            .with_beacon("UUID-MINE", "n0nce")
            .fail_always(asleep());
        let mut desktop = Desktop::with(
            Arc::new(stuck.handle()),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig {
                claude_bin: None,
                ..DesktopConfig::default()
            },
        );
        let out = desktop.on_job(job(), Instant::now());
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "asleep");
        assert_eq!(activates(&stuck), 1);
        assert_eq!(
            stuck.sources().len(),
            3,
            "the query, the activate, one retry — and then it stops"
        );
        assert!(stuck.typed().is_empty() && stuck.keys().is_empty());
        assert!(!desktop.denied(), "-1712 is not a denial");

        // …and a timeout from `input text` is **not** retried: the text may
        // already be in the terminal.
        let typing = FakeGhostty::new();
        typing
            .with_beacon("UUID-MINE", "n0nce")
            .fail_on("input text", asleep());
        let watch = typing.handle();
        let mut desktop = Desktop::with(
            Arc::new(typing),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig {
                claude_bin: None,
                ..DesktopConfig::default()
            },
        );
        let out = desktop.on_job(job(), Instant::now());
        let FromDesktop::Order { result, .. } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(result.as_ref().unwrap_err().reason(), "asleep");
        assert_eq!(activates(&watch), 0, "never woken for a typing verb");
        assert!(
            watch.typed().is_empty(),
            "and never asked to type the sentence a second time"
        );
        assert!(watch.keys().is_empty());
    }

    /// **The pin lifecycle across a `kill -9`**, which is the only way a
    /// `deck:<id4>` can outlive the session behind it.
    ///
    /// Start-up does not reconcile: `restore` runs no script, so the tab
    /// keeps the title the killed run left on it. What clears it is the
    /// next clean shutdown. In between, a session that comes back repairs
    /// its own entry on its first resolution — and one that does not is
    /// unreachable by `Release`, because `agent.removed` names the *new*
    /// `ag_…` id and the restored entry remembers the old one.
    #[test]
    fn a_pin_that_outlived_a_kill_is_cleared_by_the_next_clean_shutdown() {
        let gh = FakeGhostty::new();
        gh.with_surface("UUID-A", "deck:0001");
        gh.osc2("UUID-A", "✳ the session that is gone");
        let watch = gh.handle();
        let mut desktop = Desktop::with(
            Arc::new(gh),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig::default(),
        );
        let mut stored = BTreeMap::new();
        stored.insert(
            "UUID-A".to_string(),
            serde_json::json!({
                "key": "claude:sess-1", "id4": "0001", "agent": "ag_w000001",
                "tty": "/dev/ttys004", "pid": 4321, "manual": false,
            }),
        );
        desktop.restore(&stored);
        assert!(
            watch.sources().is_empty(),
            "start-up spends no round trip, so the title is still there"
        );
        assert_eq!(watch.tab_name("UUID-A").as_deref(), Some("deck:0001"));
        assert_eq!(desktop.resolver().pins(), vec!["UUID-A".to_string()]);

        // The removal of a session that spanned the restart names an
        // `ag_…` this run minted, which the restored entry has never heard
        // of: the unpin is a silent no-op.
        let out = desktop.on_job(
            DesktopJob::Release {
                key: "ag_gj8000002".into(),
            },
            Instant::now(),
        );
        assert!(out.is_empty());
        assert_eq!(
            watch.tab_name("UUID-A").as_deref(),
            Some("deck:0001"),
            "still pinned, with nothing behind it"
        );

        // The next clean shutdown is what finally clears it (§6).
        desktop.on_job(DesktopJob::ReleaseAll, Instant::now());
        assert_eq!(
            watch.tab_name("UUID-A").as_deref(),
            Some("✳ the session that is gone")
        );
        assert!(desktop.resolver().pins().is_empty());
    }

    /// §5.5 step 7 and §6: a session that ends unpins its tab, and
    /// shutdown unpins everything left.
    #[test]
    fn a_session_end_and_a_shutdown_both_leave_the_titles_as_they_were_found() {
        let (gh, mut desktop) = rig(FakeForeground::always(Verdict::Foreground));
        desktop.on_job(
            DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "x".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        gh.osc2("UUID-MINE", "LIVE_TITLE");
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("deck:cd34"));

        let out = desktop.on_job(
            DesktopJob::Release {
                key: "claude:sess-1".into(),
            },
            Instant::now(),
        );
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("LIVE_TITLE"));
        assert!(matches!(out.as_slice(), [FromDesktop::Pins(map)] if map.is_empty()));

        // And the belt: `ReleaseAll` after a second resolution.
        let mut second = target();
        second.agent = "ag_aaaa".into();
        second.key = "claude:sess-2".into();
        second.id4 = "aaaa".into();
        second.nonce = Some("zzzz".into());
        desktop.on_job(
            DesktopJob::Inject {
                agent: AgentId("ag_aaaa".into()),
                target: second,
                text: "y".into(),
                dialog_open: false,
            },
            Instant::now(),
        );
        assert_eq!(gh.tab_name("UUID-OTHER").as_deref(), Some("deck:aaaa"));
        desktop.on_job(DesktopJob::ReleaseAll, Instant::now());
        assert_eq!(gh.tab_name("UUID-OTHER").as_deref(), Some("deck:zzzz"));
        assert!(desktop.resolver().pins().is_empty());
    }

    /// The two states a handle can be in are genuinely different, and the
    /// difference is what keeps a daemon with no desktop behaving exactly
    /// as it did before week 5.
    #[test]
    fn a_handle_with_no_desktop_refuses_without_pretending_to_be_full() {
        let idle = DesktopHandle::none();
        assert!(!idle.engine());
        assert_eq!(idle.queued(), 0);
        let job = DesktopJob::ReleaseAll;
        assert_eq!(idle.send_or_return(job.clone()), Some(job.clone()));

        let (desktop, jobs, events) = DesktopHandle::detached();
        assert!(desktop.engine());
        for _ in 0..CMDS_CAPACITY {
            assert!(desktop.send_or_return(job.clone()).is_none());
        }
        assert_eq!(desktop.queued(), CMDS_CAPACITY);
        assert_eq!(
            desktop.send_or_return(job.clone()),
            Some(job.clone()),
            "bounded, and the refused job comes back"
        );
        assert_eq!(jobs.recv(), Ok(job.clone()));
        assert!(desktop.send_or_return(job).is_none());
        // Dropping every outside sender must leave the receiver **empty**
        // and not **disconnected**: a disconnected `select!` arm is always
        // ready and the core loop would spin for the daemon's whole life.
        events
            .send(FromDesktop::Automation { denied: true })
            .unwrap();
        assert_eq!(
            desktop.events.recv(),
            Ok(FromDesktop::Automation { denied: true })
        );
        drop(events);
        assert_eq!(
            desktop.events.try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        );
        assert_eq!(
            DesktopHandle::none().events.try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        );
    }

    /// The worker loop runs jobs in order and unpins on the way out, which
    /// is the shutdown path §6 requires.
    #[test]
    fn the_worker_thread_runs_jobs_in_order_and_unpins_when_the_core_goes() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-MINE", "n0nce");
        let watch = gh.handle();
        let desktop = Desktop::with(
            Arc::new(gh),
            Arc::new(FakeForeground::always(Verdict::Foreground)),
            DesktopConfig::default(),
        );
        let (jobs_tx, jobs_rx) = crossbeam_channel::bounded::<DesktopJob>(CMDS_CAPACITY);
        let (out_tx, out_rx) = crossbeam_channel::unbounded::<FromDesktop>();
        let worker = std::thread::spawn(move || run(desktop, &jobs_rx, &out_tx));
        jobs_tx
            .send(DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "one".into(),
                dialog_open: false,
            })
            .unwrap();
        jobs_tx
            .send(DesktopJob::Inject {
                agent: agent(),
                target: target(),
                text: "two".into(),
                dialog_open: false,
            })
            .unwrap();
        drop(jobs_tx);
        worker.join().unwrap();
        let orders: Vec<String> = out_rx
            .try_iter()
            .filter_map(|e| match e {
                FromDesktop::Order { text, result, .. } => result.ok().map(|()| text),
                _ => None,
            })
            .collect();
        assert_eq!(orders, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(watch.typed_into("UUID-MINE"), "onetwo");
        // The core going away unpins what is left (§6).
        assert_eq!(watch.tab_name("UUID-MINE").as_deref(), Some("deck:n0nce"));
    }

    /// `DesktopConfig::from_config` reads the keys `config.get` publishes,
    /// so a `[desktop]` reload reaches the injector.
    #[test]
    fn the_desktop_config_is_read_from_the_daemons_own_config() {
        let mut cfg = crate::config::Config::default();
        assert_eq!(cfg.desktop.inject, vec!["ghostty".to_string()]);
        let desktop = DesktopConfig::from_config(&cfg);
        assert_eq!(desktop.inject, vec!["ghostty".to_string()]);
        assert!(desktop.focus);
        assert_eq!(desktop.claude_bin.as_deref(), Some("claude"));
        assert!(desktop.in_process, "the daemon prefers NSAppleScript (M12)");
        // Liveness off means no roster probe: guard 2 keeps its card half.
        cfg.liveness.enabled = false;
        cfg.desktop.focus = false;
        cfg.desktop.inject = vec!["iterm".into()];
        let desktop = DesktopConfig::from_config(&cfg);
        assert_eq!(desktop.claude_bin, None);
        assert!(!desktop.focus);
        assert_eq!(desktop.inject, vec!["iterm".to_string()]);
        // And a test's default is the spawning runner, because libtest owns
        // `main` and never pumps a run loop.
        assert!(!DesktopConfig::default().in_process);
    }
}
