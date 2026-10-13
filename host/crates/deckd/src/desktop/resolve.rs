//! Resolve then pin (design §5.5; HOST-CONTRACT-W5 §3), and the two
//! things that keep it cheap and honest: a 30 s cache and a persisted
//! `terminal_map`.
//!
//! The order of the two matches is the whole design, and M6 is why:
//!
//! 1. **Re-resolution first, on `tab.name`.** An agent this daemon has
//!    already pinned carries `deck:<id4>` on its **tab**, and that survives
//!    every later OSC 2 (M6, M8). So the cheap, stable match is tried
//!    before the beacon — and §5.5 step 5's "a match on `terminal.name`
//!    alone is not sufficient" is satisfied by never looking there for a
//!    pinned session.
//! 2. **First resolution, on `terminal.name`.** A session whose
//!    `SessionStart` has just answered `terminalSequence` has the nonce on
//!    its **terminal** name, and only until the CLI writes its own title.
//!    `working directory` is not a discriminator on this Mac (every tab is
//!    `/Users/tony`) and is read for nothing.
//!
//! Then the pin is written, the `terminal.id` UUID is persisted, and both
//! are cached for 30 s (M12: each round trip is ~118 ms, and a resolve-plus
//! -pin is two).
//!
//! **Ambiguity is never resolved by guessing.** Zero matches and two
//! matches are the same refusal, [`super::errors::DesktopError::Ambiguous`],
//! with the count, because "the beacon lost a race with Claude's own title
//! write" and "there are two Ghostty instances and this `tell` reached the
//! other one" (measured 2026-09-22) both look like this and neither may be
//! papered over with the first candidate. §5.5's answer is one retry on the
//! beacon and then the one-time GUI prompt.
use super::errors::DesktopError;
use super::ghostty::{self, Surface};
use super::script::Scripting;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// §5.5: "Resolutions are cached for 30 s."
///
/// Load-bearing rather than an optimisation (M12): without it every order
/// pays two AppleScript round trips before the first keystroke, and the
/// guard-3 re-check that has to sit immediately before the Return would be
/// separated from it by another one.
pub const CACHE_TTL: Duration = Duration::from_secs(30);

/// What `state.json.terminal_map` holds, keyed on the **`terminal.id`**
/// UUID (§5.5 step 4) and never on a name: names are user-retitleable, and
/// M6 makes "the name" three different strings anyway.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pinned {
    /// The agent key (`claude:<session_id>`) this surface belongs to. The
    /// key and not the `AgentId`, because an `ag_…` is per daemon run and
    /// the map outlives a restart.
    pub key: String,
    /// The four characters the pin carries, so a restart can recognise its
    /// own tab titles without re-deriving them.
    pub id4: String,
    /// The `ag_…` id this surface belonged to *in this daemon run*. It is
    /// what `agent.removed` names, and the unpin is addressed by it.
    /// Meaningless after a restart, which is why [`Pinned::key`] exists.
    pub agent: String,
    /// The tty guard 3 checks against (§5.5 guard 3, M10's third clause).
    pub tty: Option<String>,
    /// The CLI pid guard 3 checks. `None` for a session whose hook helper
    /// resolved no `cli` pid, which is a session that can never be injected
    /// into — and saying so here is better than an absent entry, which
    /// would look like "not resolved yet".
    pub pid: Option<u32>,
    /// Set by the one-time GUI prompt of §5.5 step 6 rather than by a
    /// beacon match. Kept because a hand-picked answer must not be thrown
    /// away by an ambiguous enumeration later.
    pub manual: bool,
}

/// What [`Resolver::resolve`] is told about the session it is resolving:
/// the hook environment's half of §5.5, borrowed rather than owned because
/// the caller already holds every field in an [`super::Target`].
///
/// It is a struct and not six arguments because the six travel together
/// everywhere and always have — and because a positional `Option<&str>`
/// next to a `&str` next to another `Option<&str>` is a call site nobody
/// can read.
#[derive(Clone, Copy, Debug)]
pub struct Session<'a> {
    /// The `ag_…` id, for an unpin addressed by what `agent.removed` names.
    pub agent: &'a str,
    /// The agent key (`claude:<session_id>`): the cache and map key.
    pub key: &'a str,
    /// The beacon nonce `SessionStart` answered, for a first resolution.
    pub nonce: Option<&'a str>,
    /// The four characters the pin carries.
    pub id4: &'a str,
    /// What guard 3 checks against (§5.5 guard 3, M10).
    pub tty: Option<&'a str>,
    pub pid: Option<u32>,
}

/// A resolution that is good until [`CACHE_TTL`] runs out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The `terminal.id` UUID — the injection target.
    pub terminal: String,
    pub pinned: Pinned,
}

#[derive(Clone, Debug)]
struct Cached {
    resolution: Resolution,
    at: Instant,
}

/// The resolver: the map, the cache, and the two matches.
///
/// It owns no `Scripting`: the runner is passed in per call, because the
/// desktop worker holds one for the daemon's life and the hardware test
/// holds a different one for one row, and a resolver that captured either
/// would have to be rebuilt to swap it.
#[derive(Debug, Default)]
pub struct Resolver {
    /// `terminal.id` → what is pinned there. Exactly
    /// `state.json.terminal_map`.
    map: BTreeMap<String, Pinned>,
    /// agent key → its last resolution.
    cache: BTreeMap<String, Cached>,
    /// Beacon retries already spent, per key (§5.5 step 6: **one**).
    retried: BTreeMap<String, bool>,
}

impl Resolver {
    #[must_use]
    pub fn new() -> Resolver {
        Resolver::default()
    }

    /// Seed from `state.json` at start-up.
    ///
    /// Unparsable entries are dropped rather than refused: a `terminal_map`
    /// written by a newer daemon must not stop this one from starting, and
    /// the cost of dropping one is a re-resolution.
    pub fn restore(&mut self, stored: &BTreeMap<String, serde_json::Value>) {
        for (terminal, value) in stored {
            if let Ok(pinned) = serde_json::from_value::<Pinned>(value.clone()) {
                if !pinned.key.is_empty() {
                    self.map.insert(terminal.clone(), pinned);
                }
            }
        }
    }

    /// …and back, for `state.json`.
    #[must_use]
    pub fn terminal_map(&self) -> BTreeMap<String, serde_json::Value> {
        self.map
            .iter()
            .filter_map(|(terminal, pinned)| {
                Some((terminal.clone(), serde_json::to_value(pinned).ok()?))
            })
            .collect()
    }

    /// Every surface this daemon has pinned — what shutdown and uninstall
    /// unpin (§6: leave the operator's Ghostty as we found it).
    #[must_use]
    pub fn pins(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    /// The remembered entry for an agent key **or** an `ag_…` id, if any.
    ///
    /// Either, because the two moments that need it are different: a
    /// resolution has the key (it comes from the hook environment) and an
    /// `agent.removed` has only the id (the agent is already out of the
    /// snapshot by the time the event exists).
    #[must_use]
    pub fn known(&self, key: &str) -> Option<(&str, &Pinned)> {
        self.map
            .iter()
            .find(|(_, pinned)| pinned.key == key || pinned.agent == key)
            .map(|(terminal, pinned)| (terminal.as_str(), pinned))
    }

    /// The agent key pinned on a surface, if any.
    #[must_use]
    pub fn owner_of(&self, terminal: &str) -> Option<&Pinned> {
        self.map.get(terminal)
    }

    /// A resolution the operator picked in the GUI (§5.5 step 6). It
    /// persists under the same `terminal.id` as a beacon match would, and
    /// is marked so a later ambiguous enumeration cannot discard it.
    pub fn adopt(&mut self, terminal: &str, mut pinned: Pinned, now: Instant) -> Resolution {
        pinned.manual = true;
        self.map.insert(terminal.to_string(), pinned.clone());
        let resolution = Resolution {
            terminal: terminal.to_string(),
            pinned,
        };
        self.cache.insert(
            resolution.pinned.key.clone(),
            Cached {
                resolution: resolution.clone(),
                at: now,
            },
        );
        resolution
    }

    /// Whether `key`'s resolution is still inside the 30 s window.
    #[must_use]
    pub fn cached(&self, key: &str, now: Instant) -> Option<&Resolution> {
        self.cache
            .get(key)
            .filter(|c| now.saturating_duration_since(c.at) < CACHE_TTL)
            .map(|c| &c.resolution)
    }

    /// Throw a resolution away: the tab was closed (`-1728`), the
    /// enumeration went ambiguous, or the session ended.
    pub fn invalidate(&mut self, key: &str) {
        self.cache.remove(key);
    }

    /// Resolve `key`'s terminal, pinning it if it is not pinned already.
    ///
    /// `nonce` is what `SessionStart` answered; `None` for a session that
    /// never got a beacon, which can only be re-resolved by its pin. `tty`
    /// and `pid` are the hook environment's, recorded with the surface so
    /// guard 3 has something to check (§5.5, M10).
    pub fn resolve(
        &mut self,
        script: &dyn Scripting,
        session: &Session<'_>,
        now: Instant,
    ) -> Result<Resolution, DesktopError> {
        let Session {
            agent,
            key,
            nonce,
            id4,
            tty,
            pid,
        } = *session;
        if let Some(hit) = self.cached(key, now) {
            return Ok(hit.clone());
        }
        let surfaces = ghostty::enumerate(script)?;
        let found = match_surface(&surfaces, key, nonce, id4, self.known(key))?;
        let pinned = Pinned {
            key: key.to_string(),
            id4: id4.to_string(),
            agent: agent.to_string(),
            tty: tty.map(str::to_string),
            pid,
            manual: self.map.get(&found.id).is_some_and(|p| p.manual),
        };
        // Pin only when the tab is not carrying our label already: M5 is
        // cheap but it is still a round trip, and re-writing an identical
        // override on every 30 s cache miss is 118 ms of nothing.
        if !found.pinned_with(&ghostty::pin_label(id4)) {
            ghostty::pin(script, &found.id, id4)?;
        }
        self.map.insert(found.id.clone(), pinned.clone());
        self.retried.remove(key);
        let resolution = Resolution {
            terminal: found.id,
            pinned,
        };
        self.cache.insert(
            key.to_string(),
            Cached {
                resolution: resolution.clone(),
                at: now,
            },
        );
        Ok(resolution)
    }

    /// Whether `key` may have one more go at the beacon (§5.5 step 6:
    /// "Ambiguous → **one** retry on the beacon → then `None` and the
    /// one-time GUI prompt").
    ///
    /// Consuming, so a caller cannot loop: the second ask is `false`, and
    /// the budget comes back only when a resolution succeeds.
    pub fn may_retry(&mut self, key: &str) -> bool {
        let spent = self.retried.entry(key.to_string()).or_insert(false);
        if *spent {
            return false;
        }
        *spent = true;
        true
    }

    /// Unpin a session's surface and forget it: session end, uninstall,
    /// shutdown (§5.5 step 7, M7).
    ///
    /// The map entry is dropped **even when the unpin fails**, because the
    /// usual failure is `-1728` — the tab is already gone — and keeping an
    /// entry for a surface that does not exist would have every later
    /// shutdown try to unpin it again.
    pub fn release(&mut self, script: &dyn Scripting, key: &str) -> Result<(), DesktopError> {
        let Some((terminal, pinned)) = self.known(key) else {
            self.cache.remove(key);
            self.retried.remove(key);
            return Ok(());
        };
        let (terminal, cache_key) = (terminal.to_string(), pinned.key.clone());
        self.cache.remove(&cache_key);
        self.retried.remove(&cache_key);
        self.map.remove(&terminal);
        ghostty::unpin(script, &terminal)?;
        Ok(())
    }

    /// Unpin everything, for `deckd` shutdown and `integrations uninstall`.
    ///
    /// Returns how many were cleared; errors are collected and the walk
    /// continues, because one closed tab must not leave the rest of the
    /// operator's titles pinned.
    pub fn release_all(&mut self, script: &dyn Scripting) -> usize {
        let pins = self.pins();
        let mut cleared = 0;
        for terminal in &pins {
            match ghostty::unpin(script, terminal) {
                Ok(()) => cleared += 1,
                Err(e) => tracing::debug!(%terminal, error = %e, "unpin failed"),
            }
        }
        self.map.clear();
        self.cache.clear();
        self.retried.clear();
        cleared
    }
}

/// The two matches of §5.5, in order, over one enumeration.
///
/// `known` is what the map remembers for this key: the `terminal.id` UUID
/// **and** the `deck:<id4>` that was written on it. Both halves are needed,
/// and neither is sufficient:
///
/// * The **label alone** is not identity. `id4` is the last four characters
///   of an `ag_…`, which is `base32((sid << 32) | agent_seq)` — so the tail
///   is a function of `agent_seq` alone and the first agent of *every*
///   daemon run is `deck:0001`. A pin a killed daemon left behind is
///   therefore matched, exactly, by the next run's first agent, whatever
///   session that is and whatever tab it is really in. Adopting a label
///   this run did not write is how dictation reaches a terminal the
///   operator did not aim at.
/// * The **id alone** is not proof either: the tab may have been unpinned,
///   closed and its UUID handed to another surface. It has to still carry
///   our label.
///
/// And the label to look for is the **remembered** one, not the one built
/// from today's `id4`: a session that outlives a daemon restart gets a new
/// `ag_…`, so its tab still reads `deck:<yesterday's id4>`. Searching for
/// today's label finds nothing and loses a surface the map can name exactly.
/// The caller re-pins with the new label once the surface is found.
fn match_surface(
    surfaces: &[Surface],
    key: &str,
    nonce: Option<&str>,
    id4: &str,
    known: Option<(&str, &Pinned)>,
) -> Result<Surface, DesktopError> {
    // Re-resolution (§5.5 step 5): our own surface, by the UUID the map
    // remembers and the label we wrote on it.
    if let Some((id, remembered)) = known {
        let ours = ghostty::pin_label(&remembered.id4);
        if let Some(found) = surfaces.iter().find(|s| s.id == id && s.pinned_with(&ours)) {
            return Ok(found.clone());
        }
    }
    let label = ghostty::pin_label(id4);
    let pinned = surfaces.iter().filter(|s| s.pinned_with(&label)).count();
    if pinned > 0 {
        // A tab carrying this label that the map cannot confirm: a corpse
        // from a killed run, a hand-retitled tab, another session's. Never
        // adopted — the beacon below is the only way in.
        tracing::warn!(%key, count = pinned, "an unconfirmed pin label is not adopted");
    }
    let Some(nonce) = nonce else {
        return Err(DesktopError::Ambiguous(pinned));
    };
    let wanted = ghostty::beacon_name(nonce);
    let beaconed: Vec<&Surface> = surfaces.iter().filter(|s| s.beacons(&wanted)).collect();
    match beaconed.as_slice() {
        [only] => Ok((*only).clone()),
        other => Err(DesktopError::Ambiguous(other.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::errors::{ScriptError, ERR_NOT_PERMITTED, ERR_NO_SUCH_OBJECT};
    use crate::desktop::fakes::FakeGhostty;

    const AGENT: &str = "ag_cd34";
    const KEY: &str = "claude:9f2c-session";
    const ID4: &str = "cd34";

    fn rig() -> (FakeGhostty, Resolver, Instant) {
        (FakeGhostty::new(), Resolver::new(), Instant::now())
    }

    /// The hook environment of the session every row below resolves.
    fn session<'a>(
        agent: &'a str,
        key: &'a str,
        nonce: Option<&'a str>,
        id4: &'a str,
    ) -> Session<'a> {
        Session {
            agent,
            key,
            nonce,
            id4,
            tty: None,
            pid: None,
        }
    }

    /// …and the same with the tty and pid guard 3 needs.
    fn mine() -> Session<'static> {
        Session {
            tty: Some("/dev/ttys004"),
            pid: Some(4321),
            ..session(AGENT, KEY, Some("n0nce"), ID4)
        }
    }

    /// The plain one: our agent, our key, our beacon.
    fn plain() -> Session<'static> {
        session(AGENT, KEY, Some("n0nce"), ID4)
    }

    /// §5.5 steps 1–4: the beacon is found on `terminal.name`, the pin goes
    /// on, and the `terminal.id` UUID is what is persisted.
    #[test]
    fn a_fresh_session_is_found_by_its_beacon_and_pinned_under_its_terminal_id() {
        let (gh, mut r, now) = rig();
        gh.with_surface("UUID-OTHER", "~/src")
            .with_beacon("UUID-MINE", "n0nce")
            .with_surface("UUID-THIRD", "vim");
        let got = r.resolve(&gh, &mine(), now).unwrap();
        assert_eq!(got.terminal, "UUID-MINE");
        assert_eq!(got.pinned.key, KEY);
        assert_eq!(got.pinned.tty.as_deref(), Some("/dev/ttys004"));
        assert_eq!(got.pinned.pid, Some(4321));
        assert!(!got.pinned.manual);
        // The pin is on the tab, on that surface and no other.
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("deck:cd34"));
        assert_eq!(gh.tab_name("UUID-OTHER").as_deref(), Some("~/src"));
        // …and the map is keyed on the UUID, never on a title.
        let map = r.terminal_map();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("UUID-MINE"));
        assert_eq!(map["UUID-MINE"]["key"], KEY);
        assert_eq!(r.pins(), vec!["UUID-MINE".to_string()]);
    }

    /// **M6, as the resolver's central claim.** After the CLI's own OSC 2
    /// has taken `terminal.name`, the only field that still identifies the
    /// session is `tab.name` — so a re-resolution finds it there, and a
    /// resolver that looked at `terminal.name` would have found nothing.
    #[test]
    fn a_pinned_session_is_re_resolved_on_tab_name_after_the_beacon_is_overwritten() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce");
        r.resolve(&gh, &plain(), now).unwrap();
        // The CLI writes its own title, exactly as measured on 2026-09-22.
        gh.osc2("UUID-MINE", "✳ claude — building");
        let surfaces = ghostty::enumerate(&gh).unwrap();
        assert_eq!(surfaces[0].terminal_name, "✳ claude — building");
        assert_eq!(surfaces[0].tab_name, "deck:cd34");
        // Past the cache, with the beacon long gone from `terminal.name`.
        let later = now + CACHE_TTL + Duration::from_secs(1);
        let again = r.resolve(&gh, &plain(), later).unwrap();
        assert_eq!(again.terminal, "UUID-MINE");
        // A `terminal.name`-only resolver is exactly this, and it fails.
        assert!(!surfaces[0].beacons(&ghostty::beacon_name("n0nce")));
        // No second pin was written: the label is already on the tab.
        let pins = gh
            .sources()
            .into_iter()
            .filter(|s| s.contains("set_tab_title:deck:cd34"))
            .count();
        assert_eq!(pins, 1, "the pin is written once, not once per resolve");
    }

    /// §5.5 step 5's other half: a `deck:<id4>` on a tab this daemon did
    /// **not** pin is not adopted. The operator can retitle a tab by hand,
    /// and two agent ids can end in the same four characters.
    #[test]
    fn a_pin_label_on_an_unexpected_terminal_id_is_not_adopted() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce");
        r.resolve(&gh, &plain(), now).unwrap();
        r.invalidate(KEY);
        // Someone else's tab now carries our label, and ours is gone.
        let gh2 = FakeGhostty::new();
        gh2.with_surface("UUID-IMPOSTOR", "deck:cd34");
        let err = r.resolve(&gh2, &plain(), now).unwrap_err();
        assert_eq!(err.reason(), "ambiguous", "{err:?}");
        assert!(err.invalidates_resolution());
        // The beacon is the way back in, and it wins on `terminal.name`.
        gh2.with_beacon("UUID-MINE-2", "n0nce");
        let got = r.resolve(&gh2, &plain(), now).unwrap();
        assert_eq!(got.terminal, "UUID-MINE-2");
    }

    /// §5.5 step 6: zero and two are the same refusal, with the count, and
    /// the retry budget is exactly one.
    #[test]
    fn an_ambiguous_beacon_refuses_with_its_count_and_one_retry() {
        let (gh, mut r, now) = rig();
        // Nothing carries it — the beacon lost the race, or this `tell`
        // reached the wrong Ghostty instance (measured 2026-09-22).
        gh.with_surface("UUID-A", "zsh");
        let err = r.resolve(&gh, &plain(), now).unwrap_err();
        assert_eq!(err, DesktopError::Ambiguous(0));
        assert!(r.may_retry(KEY), "one retry on the beacon");
        assert!(!r.may_retry(KEY), "and only one");

        // Two carry it — never the first candidate.
        let two = FakeGhostty::new();
        two.with_beacon("UUID-A", "n0nce")
            .with_beacon("UUID-B", "n0nce");
        let err = r
            .resolve(&two, &session("ag_o", "other", Some("n0nce"), ID4), now)
            .unwrap_err();
        assert_eq!(err, DesktopError::Ambiguous(2));
        assert!(two.typed().is_empty(), "and nothing was typed anywhere");
        // A session with no beacon at all and no pin cannot be resolved,
        // and says so with the same word.
        let none = FakeGhostty::new();
        none.with_surface("UUID-A", "zsh");
        assert_eq!(
            r.resolve(&none, &session("ag_n", "nobeacon", None, ID4), now)
                .unwrap_err(),
            DesktopError::Ambiguous(0)
        );
    }

    /// **Two tabs carrying one pin** — the `pinned.len() > 1` refusal,
    /// which nothing exercised: mutating `if let [only] =` to
    /// `if let [only, ..] =` left all 394 lib tests green.
    ///
    /// It is reachable, and it is the harm this module exists to stop.
    /// `deck:<id4>` is not reserved and `id4` is the tail of an `ag_…` id
    /// that any `observe` client can read from `agent.list`. A pin is an
    /// *override* on `tab.name` (M8), but a tab with **no** override has
    /// `tab.name == terminal.name` (M7), so any session that can write an
    /// OSC 2 can make its own tab read `deck:cd34`. With no map entry yet —
    /// a first resolution, or the first one after a restart that lost
    /// `state.json` — the only thing standing between that and a sentence
    /// typed into the wrong terminal is this count.
    #[test]
    fn two_surfaces_carrying_one_pin_refuse_rather_than_pick_the_first() {
        let (gh, mut r, now) = rig();
        // Our own tab, pinned by a previous run, and a second tab that has
        // retitled itself to the same label through OSC 2.
        gh.with_surface("UUID-MINE", "deck:cd34")
            .with_surface("UUID-IMPOSTOR", "deck:cd34");
        // No beacon anywhere: the CLI has long since written its own title,
        // which is the ordinary state of a session minutes into its life.
        let err = r
            .resolve(&gh, &session(AGENT, KEY, None, ID4), now)
            .unwrap_err();
        assert_eq!(err, DesktopError::Ambiguous(2), "never the first candidate");
        assert!(err.invalidates_resolution());
        assert!(gh.typed().is_empty(), "and nothing was typed anywhere");
        assert!(gh.keys().is_empty());
        assert!(r.terminal_map().is_empty(), "nor was anything pinned");

        // The same two tabs, but with `terminal_map` already naming ours.
        //
        // Two reviews disagreed here and this is the resolution. Refusing
        // would be the stricter-looking answer and the worse one: `deck:` is
        // not reserved and an `id4` is readable from `agent.list`, so any
        // session that can write an OSC 2 could halt another's dictation for
        // ever simply by wearing its label — a denial of service anyone can
        // mount. Confirming against the id `state.json` already holds defeats
        // the harm that actually matters (being *redirected* into the
        // impostor's surface) without handing out that lever: the remembered
        // UUID is this session's own, the impostor's is not, and no amount of
        // retitling changes which one we wrote down. The count still rules
        // when there is nothing on file — the case above, and the one that
        // can strand a first resolution.
        let mut known = Resolver::new();
        known.restore(&BTreeMap::from([(
            "UUID-MINE".to_string(),
            serde_json::json!({
                "key": KEY, "id4": ID4, "agent": AGENT,
                "tty": "/dev/ttys004", "pid": 4321, "manual": false,
            }),
        )]));
        let gh2 = FakeGhostty::new();
        gh2.with_surface("UUID-MINE", "deck:cd34")
            .with_surface("UUID-IMPOSTOR", "deck:cd34");
        let got = known
            .resolve(&gh2, &session(AGENT, KEY, None, ID4), now)
            .expect("the id on file confirms which of the two is ours");
        assert_eq!(
            got.terminal, "UUID-MINE",
            "the remembered surface, never the impostor and never the first"
        );
        assert!(gh2.typed().is_empty(), "resolving types nothing by itself");

        // …and the beacon is the way back in: it lives on `terminal.name`,
        // which the impostor's `tab.name` did not touch (M6).
        let gh3 = FakeGhostty::new();
        gh3.with_surface("UUID-IMPOSTOR", "deck:cd34")
            .with_beacon("UUID-MINE", "n0nce");
        let got = known.resolve(&gh3, &mine(), now).unwrap();
        assert_eq!(got.terminal, "UUID-MINE");
    }

    /// M12's 30 s window, from both sides: inside it nothing is enumerated
    /// at all, and past it the whole resolution runs again.
    #[test]
    fn a_resolution_is_cached_for_thirty_seconds_and_then_re_enumerated() {
        // The number, not just the symbol. Every other assertion here is
        // written in terms of `CACHE_TTL`, so they all hold for a cache of
        // any length — including one that would hand out an hour-old
        // `terminal.id` for a tab that closed fifty-nine minutes ago.
        assert_eq!(
            CACHE_TTL,
            Duration::from_secs(30),
            "§5.5/M12: thirty seconds, and a widening is not a refactor"
        );
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce");
        r.resolve(&gh, &plain(), now).unwrap();
        let after_first = gh.sources().len();
        assert_eq!(after_first, 2, "one enumerate + one pin (M12: ~236 ms)");

        // Inside the window: no round trip at all.
        for offset in [0, 1, 29] {
            let hit = r
                .resolve(&gh, &plain(), now + Duration::from_secs(offset))
                .unwrap();
            assert_eq!(hit.terminal, "UUID-MINE");
        }
        assert_eq!(gh.sources().len(), after_first, "the cache spent nothing");
        assert!(r.cached(KEY, now + Duration::from_secs(29)).is_some());

        // On the boundary and past it: enumerated again — by the clock, not
        // by the constant.
        assert!(r.cached(KEY, now + Duration::from_secs(30)).is_none());
        assert!(r.cached(KEY, now + Duration::from_secs(31)).is_none());
        assert!(r.cached(KEY, now + CACHE_TTL).is_none());
        r.resolve(&gh, &plain(), now + CACHE_TTL).unwrap();
        assert_eq!(gh.sources().len(), after_first + 1, "enumerate, no re-pin");
        // An invalidation is the same as an expiry, for the caller that
        // saw a `-1728`.
        r.invalidate(KEY);
        assert!(r.cached(KEY, now).is_none());
    }

    /// §5.5 step 7 and M7: session end unpins, and the map forgets — even
    /// when the tab has already gone, which is the common case.
    #[test]
    fn a_session_that_ends_unpins_its_tab_and_is_forgotten() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce");
        r.resolve(&gh, &plain(), now).unwrap();
        gh.osc2("UUID-MINE", "LIVE_TITLE");
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("deck:cd34"));

        r.release(&gh, KEY).unwrap();
        // M7: the empty value put the live title back.
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("LIVE_TITLE"));
        assert!(r.terminal_map().is_empty());
        assert!(r.cached(KEY, now).is_none());
        assert!(r.known(KEY).is_none());
        // Releasing an unknown key is a no-op and not an error.
        r.release(&gh, "never-seen").unwrap();

        // A tab that is already gone answers `-1728`; the entry still goes,
        // or every shutdown from now on would retry it.
        let gone = FakeGhostty::new();
        gone.with_beacon("UUID-2", "n2");
        r.resolve(&gone, &session("ag_k2", "k2", Some("n2"), "aaaa"), now)
            .unwrap();
        gone.fail_on(
            "set_tab_title:",
            ScriptError::Osa {
                number: ERR_NO_SUCH_OBJECT,
                message: "Can't get terminal 1".into(),
            },
        );
        let err = r.release(&gone, "k2").unwrap_err();
        assert!(err.invalidates_resolution());
        assert!(r.terminal_map().is_empty(), "forgotten regardless");
    }

    /// §6's rule: every tab this build pins is unpinned at shutdown, and
    /// one closed tab does not strand the others.
    #[test]
    fn shutdown_unpins_every_tab_this_daemon_pinned() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-A", "na").with_beacon("UUID-B", "nb");
        r.resolve(&gh, &session("ag_ka", "ka", Some("na"), "aaaa"), now)
            .unwrap();
        r.resolve(&gh, &session("ag_kb", "kb", Some("nb"), "bbbb"), now)
            .unwrap();
        gh.osc2("UUID-A", "A_LIVE").osc2("UUID-B", "B_LIVE");
        assert_eq!(r.pins().len(), 2);
        // The first unpin fails; the second must still happen.
        gh.fail_on(
            "set_tab_title:\" on (first terminal whose id is \"UUID-A",
            ScriptError::Osa {
                number: ERR_NO_SUCH_OBJECT,
                message: "gone".into(),
            },
        );
        assert_eq!(r.release_all(&gh), 1);
        assert_eq!(gh.tab_name("UUID-B").as_deref(), Some("B_LIVE"));
        assert!(r.pins().is_empty());
        assert!(r.terminal_map().is_empty());
    }

    /// The map is what survives a restart, and a hand-picked answer
    /// survives an ambiguous enumeration (§5.5 step 6).
    #[test]
    fn the_terminal_map_round_trips_through_state_json_and_keeps_a_manual_answer() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce");
        r.resolve(&gh, &mine(), now).unwrap();
        let stored = r.terminal_map();

        let mut restored = Resolver::new();
        restored.restore(&stored);
        assert_eq!(restored.pins(), vec!["UUID-MINE".to_string()]);
        let (terminal, pinned) = restored.known(KEY).unwrap();
        assert_eq!(terminal, "UUID-MINE");
        assert_eq!(pinned.tty.as_deref(), Some("/dev/ttys004"));
        assert_eq!(pinned.pid, Some(4321));
        assert_eq!(restored.owner_of("UUID-MINE").unwrap().key, KEY);
        // Junk in the file is dropped, not fatal.
        let mut junk = BTreeMap::new();
        junk.insert("UUID-X".to_string(), serde_json::json!("not an object"));
        junk.insert("UUID-Y".to_string(), serde_json::json!({"key": ""}));
        let mut tolerant = Resolver::new();
        tolerant.restore(&junk);
        assert!(tolerant.pins().is_empty());

        // The GUI's one-time answer persists under the same id, is cached,
        // and is marked so a later resolve keeps the mark.
        let mut picked = Resolver::new();
        let adopted = picked.adopt(
            "UUID-PICKED",
            Pinned {
                key: KEY.into(),
                id4: ID4.into(),
                agent: AGENT.into(),
                tty: Some("/dev/ttys007".into()),
                pid: Some(9),
                manual: false,
            },
            now,
        );
        assert!(adopted.pinned.manual);
        assert_eq!(picked.cached(KEY, now).unwrap().terminal, "UUID-PICKED");
        let gh2 = FakeGhostty::new();
        gh2.with_surface("UUID-PICKED", "deck:cd34");
        picked.invalidate(KEY);
        let again = picked
            .resolve(&gh2, &session(AGENT, KEY, None, ID4), now)
            .unwrap();
        assert!(again.pinned.manual, "a hand-picked answer stays marked");
    }

    /// **A pin a killed daemon left behind must not capture the next run's
    /// session.**
    ///
    /// `id4` is the tail of `base32((sid << 32) | agent_seq)`, so it is a
    /// function of `agent_seq` alone: the first agent of *every* daemon run
    /// is `deck:0001`, the second `deck:0002`. A `kill -9` leaves those
    /// labels on the operator's tabs, and the next run's agents match them
    /// exactly — a different session, a different tty, a different tab.
    /// Guard 3 cannot catch it: it checks the new session's pid against the
    /// new session's tty and says yes, because both are real. The only
    /// thing standing between that and a sentence typed into the wrong
    /// terminal is the resolver refusing a label the map cannot confirm.
    #[test]
    fn a_pin_left_by_a_killed_daemon_does_not_capture_the_next_runs_session() {
        let mut r = Resolver::new();
        // `state.json` as the killed run left it: tab A, the old session.
        let mut stored = BTreeMap::new();
        stored.insert(
            "UUID-A".to_string(),
            serde_json::json!({
                "key": "claude:OLD", "id4": "0001", "agent": "ag_w000001",
                "tty": "/dev/ttys004", "pid": 4321, "manual": false,
            }),
        );
        r.restore(&stored);

        let gh = FakeGhostty::new();
        // Tab A still carries `deck:0001`, with the old session's own title
        // underneath it. Tab B is the new session, beacon fresh.
        gh.with_surface("UUID-A", "deck:0001");
        gh.osc2("UUID-A", "✳ the session that was here yesterday");
        // Tab B's *tab* title is overridden too — a `deck:` from an older
        // run, or the operator's own — so the beacon is on `terminal.name`
        // and **only** there. A resolver that read `tab.name` for the
        // beacon finds nothing here (M6's other half).
        gh.with_surface("UUID-B", "deck:9999");
        gh.osc2("UUID-B", &ghostty::beacon_name("0030"));
        assert_eq!(
            ghostty::enumerate(&gh).unwrap()[1].tab_name,
            "deck:9999",
            "the beacon is not on this surface's tab.name"
        );
        let new = Session {
            agent: "ag_gj8000001",
            key: "claude:NEW",
            nonce: Some("0030"),
            id4: "0001", // the collision, and it is not a coincidence
            tty: Some("/dev/ttys009"),
            pid: Some(999),
        };
        let got = r.resolve(&gh, &new, Instant::now()).unwrap();
        assert_eq!(
            got.terminal, "UUID-B",
            "the beacon's tab, never the corpse of the last run's"
        );
        assert_eq!(
            gh.tab_name("UUID-B").as_deref(),
            Some("deck:0001"),
            "and it is this run's label that is written on it"
        );
        // …and the stale label on A was left exactly as it was found: the
        // resolver does not get to retitle a tab it could not claim.
        assert_eq!(gh.tab_name("UUID-A").as_deref(), Some("deck:0001"));

        // With no beacon to fall back on, the same label is a refusal and
        // not a guess — and the count the GUI prompt shows is the number of
        // tabs that carried it, which is now two: the corpse and the tab
        // this resolution just pinned.
        let mut bare = Resolver::new();
        bare.restore(&stored);
        let err = bare
            .resolve(&gh, &Session { nonce: None, ..new }, Instant::now())
            .unwrap_err();
        assert_eq!(err, DesktopError::Ambiguous(2));
    }

    /// …and the other half of the same fact: a session that **survives** a
    /// daemon restart is still found, by the UUID the map persisted and the
    /// label this daemon actually wrote.
    ///
    /// It has no second beacon to offer — `terminalSequence` is one-shot on
    /// `SessionStart` (C25) and Claude has long since overwritten
    /// `terminal.name` — and its new `ag_…` gives it a new `id4`, so its tab
    /// reads `deck:<yesterday's id4>`. A resolver that searched for today's
    /// label would lose a surface `state.json` can name exactly.
    #[test]
    fn a_session_that_outlives_a_restart_is_re_found_by_the_persisted_uuid() {
        let mut r = Resolver::new();
        let mut stored = BTreeMap::new();
        stored.insert(
            "UUID-A".to_string(),
            serde_json::json!({
                "key": KEY, "id4": "0001", "agent": "ag_w000001",
                "tty": "/dev/ttys004", "pid": 4321, "manual": false,
            }),
        );
        r.restore(&stored);
        let gh = FakeGhostty::new();
        gh.with_surface("UUID-A", "deck:0001");
        gh.osc2("UUID-A", "✳ claude — building");

        let today = Session {
            agent: "ag_gj8000002",
            key: KEY,
            nonce: None,
            id4: "0002",
            tty: Some("/dev/ttys004"),
            pid: Some(4321),
        };
        let got = r.resolve(&gh, &today, Instant::now()).unwrap();
        assert_eq!(got.terminal, "UUID-A");
        // Re-pinned with this run's label, so the next restart has a
        // current one to look for.
        assert_eq!(gh.tab_name("UUID-A").as_deref(), Some("deck:0002"));
        assert_eq!(r.terminal_map()["UUID-A"]["id4"], "0002");
        // The UUID still has to carry our label: a tab that was unpinned
        // under us is not confirmed by its id alone.
        let unpinned = FakeGhostty::new();
        unpinned.with_surface("UUID-A", "zsh");
        let mut fresh = Resolver::new();
        fresh.restore(&stored);
        assert_eq!(
            fresh
                .resolve(&unpinned, &today, Instant::now())
                .unwrap_err(),
            DesktopError::Ambiguous(0)
        );
    }

    /// `-1743` stops the resolver before it pins anything, and the error
    /// reaches the caller as the automation denial and not as an ambiguity
    /// (M13: this is the only evidence that path works).
    #[test]
    fn an_automation_denial_stops_the_resolver_without_touching_a_title() {
        let (gh, mut r, now) = rig();
        gh.with_beacon("UUID-MINE", "n0nce")
            .fail_always(ScriptError::Osa {
                number: ERR_NOT_PERMITTED,
                message: "Not authorized to send Apple events to Ghostty.".into(),
            });
        let err = r.resolve(&gh, &plain(), now).unwrap_err();
        assert_eq!(err.reason(), "automation_denied");
        assert!(!err.invalidates_resolution(), "the tab is fine; we are not");
        assert!(r.terminal_map().is_empty());
        assert_eq!(gh.tab_name("UUID-MINE").as_deref(), Some("deck:n0nce"));
    }
}
