//! The fakes behind [`Scripting`] and [`Foreground`], **co-designed with
//! the real ones** (HOST-CONTRACT-W5 §3: "every resolver/injector test runs
//! against the fake; the real impl is exercised only by the hardware step").
//!
//! They are shaped by the same measurements the real calls are, so they can
//! fail the way Ghostty does:
//!
//! * **A fake answers per script, not per call count.** [`FakeGhostty`]
//!   keys its answers off what the source *says* (`input text`, `perform
//!   action`, the enumeration), so a test that changes the order of two
//!   calls does not silently start asserting something else.
//! * **Two title fields, always.** [`FakeGhostty`] keeps a surface list
//!   with `terminal_name` and `tab_name` apart, and `pin` moves only the
//!   second one — which is M6, and which makes a `terminal.name`-only
//!   resolver fail here exactly as it would fail live.
//! * **An error is a first-class answer.** `-1743`, `-1712`, `-1708` and
//!   `-1700` are all injectable, because M13 could not reproduce the first
//!   two on this machine and a unit test is the only evidence either path
//!   has.
//! * **The guards' *order* is observable.** [`FakeForeground`] counts and
//!   can change its mind between calls, which is how "checked again
//!   immediately before `send key`" is proven rather than reviewed.
use super::errors::ScriptError;
use super::fg::{Foreground, Verdict};
use super::ghostty::{self, Surface};
use super::script::Scripting;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A runner that records every source and answers one fixed string.
///
/// The minimum useful fake, for the tests whose subject is the *source*
/// rather than the reply (`ghostty`'s own).
#[derive(Debug, Default)]
pub struct ScriptLog {
    sources: Mutex<Vec<String>>,
    answer: String,
    error: Mutex<Option<ScriptError>>,
}

impl ScriptLog {
    #[must_use]
    pub fn new() -> ScriptLog {
        ScriptLog::default()
    }

    /// The reply every `run` gives.
    #[must_use]
    pub fn answering(mut self, answer: &str) -> ScriptLog {
        self.answer = answer.to_string();
        self
    }

    /// …or the error it gives instead, once.
    #[must_use]
    pub fn failing(self, error: ScriptError) -> ScriptLog {
        *self.error.lock().unwrap() = Some(error);
        self
    }

    /// Every source that was run, in order.
    #[must_use]
    pub fn sources(&self) -> Vec<String> {
        self.sources.lock().unwrap().clone()
    }
}

impl Scripting for ScriptLog {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        self.sources.lock().unwrap().push(src.to_string());
        match self.error.lock().unwrap().take() {
            Some(e) => Err(e),
            None => Ok(self.answer.clone()),
        }
    }

    fn name(&self) -> &'static str {
        "script-log"
    }
}

/// Ghostty, as a list of surfaces and a script of failures.
///
/// It understands the five sources this module generates — the
/// enumeration, `perform action "set_tab_title:…"`, `input text`, `send
/// key` and the focus walk — and answers each the way the application did
/// on 2026-09-22.
#[derive(Debug, Default)]
pub struct FakeGhostty {
    inner: Arc<Shared>,
}

#[derive(Debug, Default)]
struct Shared {
    surfaces: Mutex<Vec<Surface>>,
    /// Every source run, for the ordering assertions.
    sources: Mutex<Vec<String>>,
    /// What was typed, per `terminal.id`, in arrival order — the fake's
    /// answer to "did anything reach the wrong tab?".
    typed: Mutex<Vec<(String, String)>>,
    /// Keys sent, per `terminal.id`.
    keys: Mutex<Vec<(String, String)>>,
    /// Errors to raise, matched on a substring of the source. Each fires
    /// once, in insertion order.
    faults: Mutex<Vec<(String, ScriptError)>>,
    /// An error every call gets, for the `-1743` chain.
    always: Mutex<Option<ScriptError>>,
}

impl FakeGhostty {
    #[must_use]
    pub fn new() -> FakeGhostty {
        FakeGhostty::default()
    }

    /// A handle on the same state, so a test can watch a runner it has
    /// already handed away.
    #[must_use]
    pub fn handle(&self) -> FakeGhostty {
        FakeGhostty {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Add a surface whose `terminal.name` is the beacon and whose
    /// `tab.name` is the same string — which is what a *fresh* surface
    /// looks like before anything pins it (M7 explains why: with no
    /// override, the two fields are the one live title).
    pub fn with_beacon(&self, id: &str, nonce: &str) -> &FakeGhostty {
        let name = ghostty::beacon_name(nonce);
        self.push(Surface {
            id: id.into(),
            terminal_name: name.clone(),
            tab_name: name.clone(),
            window_name: name,
            window_id: format!("tab-group-{id}"),
            tab_id: format!("tab-{id}"),
            selected: true,
            index: 1,
        });
        self
    }

    /// Add a surface with an arbitrary live title and no pin.
    pub fn with_surface(&self, id: &str, title: &str) -> &FakeGhostty {
        self.push(Surface {
            id: id.into(),
            terminal_name: title.into(),
            tab_name: title.into(),
            window_name: title.into(),
            window_id: format!("tab-group-{id}"),
            tab_id: format!("tab-{id}"),
            selected: true,
            index: 1,
        });
        self
    }

    fn push(&self, surface: Surface) {
        self.inner.surfaces.lock().unwrap().push(surface);
    }

    /// M6's hostile case: an OSC 2 lands on `terminal.name` and `tab.name`
    /// keeps whatever override is on it.
    pub fn osc2(&self, id: &str, title: &str) -> &FakeGhostty {
        let mut surfaces = self.inner.surfaces.lock().unwrap();
        if let Some(s) = surfaces.iter_mut().find(|s| s.id == id) {
            s.terminal_name = title.to_string();
            s.window_name = if s.tab_name.starts_with(ghostty::PIN_PREFIX) {
                s.tab_name.clone()
            } else {
                title.to_string()
            };
            if !s.tab_name.starts_with(ghostty::PIN_PREFIX) {
                s.tab_name = title.to_string();
            }
        }
        drop(surfaces);
        self
    }

    /// Raise `error` on the first source containing `needle`.
    pub fn fail_on(&self, needle: &str, error: ScriptError) -> &FakeGhostty {
        self.inner
            .faults
            .lock()
            .unwrap()
            .push((needle.to_string(), error));
        self
    }

    /// Raise `error` on **every** call: the `-1743` state, where the whole
    /// chain has to skip Ghostty.
    pub fn fail_always(&self, error: ScriptError) -> &FakeGhostty {
        *self.inner.always.lock().unwrap() = Some(error);
        self
    }

    #[must_use]
    pub fn surfaces(&self) -> Vec<Surface> {
        self.inner.surfaces.lock().unwrap().clone()
    }

    /// The `tab.name` of a surface: what a pin wrote, or the live title.
    #[must_use]
    pub fn tab_name(&self, id: &str) -> Option<String> {
        self.inner
            .surfaces
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.tab_name.clone())
    }

    /// Everything typed, as `(terminal.id, text)`.
    #[must_use]
    pub fn typed(&self) -> Vec<(String, String)> {
        self.inner.typed.lock().unwrap().clone()
    }

    /// …and every key sent.
    #[must_use]
    pub fn keys(&self) -> Vec<(String, String)> {
        self.inner.keys.lock().unwrap().clone()
    }

    /// What reached one surface, concatenated: the fake's `got_cap` file.
    #[must_use]
    pub fn typed_into(&self, id: &str) -> String {
        self.inner
            .typed
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target == id)
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("")
    }

    #[must_use]
    pub fn sources(&self) -> Vec<String> {
        self.inner.sources.lock().unwrap().clone()
    }

    /// The `terminal.id` a source addresses, from its
    /// `first terminal whose id is "…"` clause.
    fn addressed(src: &str) -> Option<String> {
        let (_, tail) = src.split_once("whose id is \"")?;
        let (id, _) = tail.split_once('"')?;
        Some(id.to_string())
    }

    /// The quoted argument of a verb: `input text "x" to …` → `x`.
    fn quoted_after(src: &str, verb: &str) -> Option<String> {
        let (_, tail) = src.split_once(verb)?;
        let tail = tail.trim_start();
        let inner = tail.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.push(chars.next()?),
                '"' => return Some(out),
                other => out.push(other),
            }
        }
        None
    }
}

impl Scripting for FakeGhostty {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        self.inner.sources.lock().unwrap().push(src.to_string());
        if let Some(error) = self.inner.always.lock().unwrap().clone() {
            return Err(error);
        }
        {
            let mut faults = self.inner.faults.lock().unwrap();
            if let Some(at) = faults.iter().position(|(needle, _)| src.contains(needle)) {
                return Err(faults.remove(at).1);
            }
        }
        // The focus walk shares the enumeration's triple `repeat` and is
        // told apart by the verb inside it. Checked **first**, because it
        // also contains `perform action` and would otherwise be read as a
        // pin.
        if src.contains("goto_tab:") {
            let surfaces = self.inner.surfaces.lock().unwrap();
            let id = src
                .split_once("if (id of tm) is \"")
                .and_then(|(_, t)| t.split_once('"'))
                .map(|(id, _)| id.to_string());
            let found = id.is_some_and(|id| surfaces.iter().any(|s| s.id == id));
            return Ok(if found {
                "selected".into()
            } else {
                String::new()
            });
        }
        // The enumeration: every surface, in the wire format
        // `ghostty::parse_surfaces` reads.
        if src.contains("repeat with tm in terminals of tb") && src.contains("name of tm") {
            let surfaces = self.inner.surfaces.lock().unwrap();
            let out = surfaces
                .iter()
                .map(|s| {
                    format!(
                        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                        s.id,
                        s.terminal_name,
                        s.tab_name,
                        s.window_name,
                        s.window_id,
                        s.tab_id,
                        s.selected,
                        s.index
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1e}");
            return Ok(if out.is_empty() { out } else { out + "\u{1e}" });
        }
        let addressed = Self::addressed(src);
        if src.contains("perform action") {
            let action = Self::quoted_after(src, "perform action").unwrap_or_default();
            let Some(value) = action.strip_prefix(ghostty::PIN_ACTION) else {
                return Ok("false".into());
            };
            let Some(id) = addressed else {
                return Ok("false".into());
            };
            let mut surfaces = self.inner.surfaces.lock().unwrap();
            let Some(surface) = surfaces.iter_mut().find(|s| s.id == id) else {
                return Err(ScriptError::Osa {
                    number: super::errors::ERR_NO_SUCH_OBJECT,
                    message: format!("Can't get terminal 1 whose id = \"{id}\""),
                });
            };
            if value.is_empty() {
                // M7: the empty value clears the override and both names go
                // back to the live title, which the fake keeps in
                // `terminal_name`.
                surface.tab_name = surface.terminal_name.clone();
            } else {
                surface.tab_name = value.to_string();
            }
            surface.window_name = surface.tab_name.clone();
            return Ok("true".into());
        }
        if src.contains("input text") {
            let text = Self::quoted_after(src, "input text").unwrap_or_default();
            let Some(id) = addressed else {
                return Ok(String::new());
            };
            if !self
                .inner
                .surfaces
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.id == id)
            {
                return Err(ScriptError::Osa {
                    number: super::errors::ERR_NO_SUCH_OBJECT,
                    message: format!("Can't get terminal 1 whose id = \"{id}\""),
                });
            }
            self.inner.typed.lock().unwrap().push((id, text));
            return Ok(String::new());
        }
        if src.contains("send key") {
            let key = Self::quoted_after(src, "send key").unwrap_or_default();
            // M3, in the fake: the word the design used does not exist here
            // either, so a regression fails in CI and not on a hardware day.
            if key != ghostty::ENTER {
                return Err(ScriptError::Osa {
                    number: super::errors::ERR_COERCION,
                    message: format!("Unknown key name: {key}"),
                });
            }
            let Some(id) = addressed else {
                return Ok(String::new());
            };
            self.inner.keys.lock().unwrap().push((id, key));
            return Ok(String::new());
        }
        if src.contains("count of windows") {
            let count = self.inner.surfaces.lock().unwrap().len();
            return Ok(format!("1.3.1\u{1f}{count}"));
        }
        Ok(String::new())
    }

    fn name(&self) -> &'static str {
        "fake-ghostty"
    }
}

/// Guard 3, scripted.
///
/// The verdicts are a queue and the **last one repeats**, so a test can say
/// "foreground, then background" and prove that the second check happens at
/// all — which is the only way §5.5's "immediately before `input text`
/// **and** again immediately before `send key`" can be asserted.
#[derive(Debug)]
pub struct FakeForeground {
    verdicts: Mutex<std::collections::VecDeque<Verdict>>,
    last: Mutex<Verdict>,
    calls: AtomicUsize,
    seen: Mutex<Vec<(u32, String)>>,
}

impl Default for FakeForeground {
    fn default() -> Self {
        FakeForeground::always(Verdict::Foreground)
    }
}

impl FakeForeground {
    /// One verdict, for ever.
    #[must_use]
    pub fn always(verdict: Verdict) -> FakeForeground {
        FakeForeground {
            verdicts: Mutex::new(std::collections::VecDeque::new()),
            last: Mutex::new(verdict),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// A script; the final verdict repeats once the queue is empty.
    #[must_use]
    pub fn scripted(verdicts: &[Verdict]) -> FakeForeground {
        let last = verdicts.last().copied().unwrap_or(Verdict::Unknown);
        FakeForeground {
            verdicts: Mutex::new(verdicts.iter().copied().collect()),
            last: Mutex::new(last),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Every `(pid, tty)` it was asked about — so a test can prove the
    /// check used the **recorded** tty and not something it made up.
    #[must_use]
    pub fn seen(&self) -> Vec<(u32, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl Foreground for FakeForeground {
    fn check(&self, pid: u32, tty: &str) -> Verdict {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((pid, tty.to_string()));
        let mut queue = self.verdicts.lock().unwrap();
        match queue.pop_front() {
            Some(verdict) => verdict,
            None => *self.last.lock().unwrap(),
        }
    }
}

/// §13's Linux port: the seam exists, nothing is behind it.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoScripting;

impl Scripting for NoScripting {
    fn run(&self, _src: &str) -> Result<String, ScriptError> {
        Err(ScriptError::Unavailable(
            "this build has no AppleScript engine".into(),
        ))
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::errors::ERR_NOT_PERMITTED;

    /// The fake has to *be* M6, or every resolver test that passes against
    /// it proves nothing: a pin moves `tab.name` and an OSC 2 moves
    /// `terminal.name`, and neither touches the other.
    #[test]
    fn the_fake_keeps_the_pin_and_the_beacon_on_the_two_fields_ghostty_does() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-1", "cd34");
        // Fresh: one live title on both fields (M7).
        let before = ghostty::enumerate(&gh).unwrap();
        assert_eq!(before[0].terminal_name, "deck:cd34");
        assert_eq!(before[0].tab_name, "deck:cd34");

        ghostty::pin(&gh, "UUID-1", "cd34").unwrap();
        gh.osc2("UUID-1", "HOSTILE_RETITLE");
        let after = ghostty::enumerate(&gh).unwrap();
        assert_eq!(after[0].terminal_name, "HOSTILE_RETITLE", "the OSC 2 won");
        assert_eq!(after[0].tab_name, "deck:cd34", "the pin survived");
        assert_eq!(after[0].pin(), Some("cd34"));

        // M7: unpinning puts the live title back on both.
        ghostty::unpin(&gh, "UUID-1").unwrap();
        let restored = ghostty::enumerate(&gh).unwrap();
        assert_eq!(restored[0].tab_name, "HOSTILE_RETITLE");
        assert_eq!(restored[0].terminal_name, "HOSTILE_RETITLE");
        // An unpinned surface with no override takes the OSC 2 on both,
        // exactly as the live one did.
        gh.with_surface("UUID-2", "zsh");
        gh.osc2("UUID-2", "vim");
        assert_eq!(gh.tab_name("UUID-2").as_deref(), Some("vim"));
    }

    /// The fake aims: text goes to the surface the source named and
    /// nowhere else, which is the property M4 measured live.
    #[test]
    fn the_fake_delivers_text_to_the_addressed_surface_and_no_other() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-A", "aaaa")
            .with_beacon("UUID-B", "bbbb");
        ghostty::input_text(&gh, "UUID-A", "AIMED_PAYLOAD").unwrap();
        ghostty::send_key(&gh, "UUID-A", ghostty::ENTER).unwrap();
        assert_eq!(gh.typed_into("UUID-A"), "AIMED_PAYLOAD");
        assert_eq!(gh.typed_into("UUID-B"), "", "nothing leaked");
        assert_eq!(gh.keys(), vec![("UUID-A".to_string(), "enter".to_string())]);
        // M3: the fake refuses the word Ghostty refuses.
        let err = ghostty::send_key(&gh, "UUID-A", "return").unwrap_err();
        assert_eq!(err.number(), Some(crate::desktop::errors::ERR_COERCION));
        // A surface that is gone is `-1728`, not a silent success.
        let err = ghostty::input_text(&gh, "UUID-GONE", "x").unwrap_err();
        assert_eq!(
            err.number(),
            Some(crate::desktop::errors::ERR_NO_SUCH_OBJECT)
        );
    }

    /// Injected errors, one-shot and permanent, are what stands in for the
    /// two numbers M13 could not reproduce.
    #[test]
    fn the_fake_can_raise_the_numbers_this_machine_would_not_produce() {
        let gh = FakeGhostty::new();
        gh.with_beacon("UUID-1", "cd34");
        gh.fail_on(
            "input text",
            ScriptError::Osa {
                number: crate::desktop::errors::ERR_TIMEOUT,
                message: "AppleEvent timed out".into(),
            },
        );
        let err = ghostty::input_text(&gh, "UUID-1", "x").unwrap_err();
        assert!(err.wake_and_retry());
        // One-shot: the next call goes through.
        ghostty::input_text(&gh, "UUID-1", "x").unwrap();

        let denied = FakeGhostty::new();
        denied
            .with_beacon("UUID-1", "cd34")
            .fail_always(ScriptError::Osa {
                number: ERR_NOT_PERMITTED,
                message: "Not authorized to send Apple events to Ghostty.".into(),
            });
        assert!(ghostty::enumerate(&denied).unwrap_err().denied());
        assert!(ghostty::input_text(&denied, "UUID-1", "x")
            .unwrap_err()
            .denied());
        assert_eq!(denied.name(), "fake-ghostty");
    }

    /// The guard-3 fake's whole purpose: a verdict that changes between
    /// calls, and a record of what it was asked.
    #[test]
    fn the_foreground_fake_can_change_its_mind_between_two_checks() {
        let fg = FakeForeground::scripted(&[Verdict::Foreground, Verdict::Background]);
        assert_eq!(fg.check(4321, "/dev/ttys004"), Verdict::Foreground);
        assert_eq!(fg.check(4321, "/dev/ttys004"), Verdict::Background);
        // The last verdict repeats rather than running out.
        assert_eq!(fg.check(4321, "/dev/ttys004"), Verdict::Background);
        assert_eq!(fg.calls(), 3);
        assert_eq!(fg.seen()[0], (4321, "/dev/ttys004".to_string()));
        assert_eq!(FakeForeground::default().check(1, "x"), Verdict::Foreground);
        // An empty script is inconclusive, which refuses.
        assert!(!FakeForeground::scripted(&[]).check(1, "x").allows());
        // And the seam with nothing behind it refuses everything.
        assert!(NoScripting.run("anything").is_err());
        assert_eq!(NoScripting.name(), "none");
    }
}
