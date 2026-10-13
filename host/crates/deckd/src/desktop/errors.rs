//! What AppleScript says when it says no, and what the injector chain
//! does with each answer (HOST-CONTRACT-W5 §1 M3, M11, M13).
//!
//! Two error vocabularies meet here and they are not the same size. Apple
//! Events have numbers — five of them matter — and `op.order` has the three
//! `order.failed{reason}` words §4.12 publishes. The mapping between them
//! is the whole of this file, because a chain that collapsed them would
//! have to answer "the operator's Mac refused us Automation" and "the
//! operator's CLI exited to a shell" with the same word, and those two need
//! opposite reactions: the first is a permanent degradation the tray has to
//! show, the second is a transient refusal the operator fixes by looking at
//! their terminal.
//!
//! **M13 is the honesty note.** `-1743` and `-1712` were *not reproducible*
//! on this machine on 2026-09-22: Automation TCC is already granted for the
//! responsible process, and a benign `version & (count of windows)` came
//! back in band with no prompt. Both paths are implemented and unit-tested
//! against injected errors, and neither is *claimed* to have been seen
//! live. The tests below are the only evidence either path works, and they
//! say so.
use std::fmt;

/// `errAEEventNotPermitted` — Automation TCC denied us this app (§10.4).
/// The chain stops using Ghostty and `integrations.status` shows it.
pub const ERR_NOT_PERMITTED: i32 = -1743;
/// `errAETimeout` — **not** "no Ghostty" (§5.5): a long-idle instance
/// refused 4/4 element queries with its main thread provably idle, and
/// activating it fixed the session permanently. So: activate, retry once.
pub const ERR_TIMEOUT: i32 = -1712;
/// `errAEEventNotHandled` — "Can't continue". M11: `new tab` and `new
/// window with configuration` answer this and **still create the surface**,
/// so nothing may depend on their return value. Measured again on
/// 2026-09-22: `select <tab>` answers it too, because `select` is not a
/// command in this dictionary at all (see [`super::ghostty::focus`]).
pub const ERR_NOT_HANDLED: i32 = -1708;
/// `errAECoercionFail` — how Ghostty answers a word it does not know. M3:
/// `send key "return"` is `-1700 Unknown key name: return`, and the working
/// spelling is [`super::ghostty::ENTER`].
pub const ERR_COERCION: i32 = -1700;
/// `errAENoSuchObject` — the terminal the map remembers is gone (the tab
/// was closed, or the id belongs to another Ghostty **instance**: measured
/// 2026-09-22, `open -na` gives every launch its own process and
/// `tell application "Ghostty"` reaches exactly one of them).
pub const ERR_NO_SUCH_OBJECT: i32 = -1728;
/// `errAENotModifiable` — a read-only property was assigned. Measured
/// 2026-09-22 for `selected of <tab>` and `index of <window>`, which is why
/// focus goes through `perform action "goto_tab:<n>"` instead.
pub const ERR_NOT_MODIFIABLE: i32 = -10006;
/// `procNotFound` — the application is not running and could not be
/// launched.
pub const ERR_NO_PROCESS: i32 = -600;

/// One failed `Scripting::run`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    /// AppleScript itself answered, with a number. The message is the
    /// engine's own and is never content (it names apps and selectors, not
    /// what the operator typed), so it may be logged.
    #[error("applescript {number}: {message}")]
    Osa { number: i32, message: String },
    /// There was no engine to ask: `osascript` could not be spawned, the
    /// main thread never came back, this build has no scripting at all.
    #[error("no scripting engine: {0}")]
    Unavailable(String),
}

/// What the chain does about a [`ScriptError`].
///
/// A classification and not a synonym: two numbers can share a reaction
/// (`-1728` and `-600` both mean "ask again from scratch") while two
/// reactions never share a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fault {
    /// `-1743`: stop. Nothing will work until the operator grants
    /// Automation, and asking again only trains them to click Deny.
    Denied,
    /// `-1712`: activate and retry **once** (§5.5).
    Asleep,
    /// `-1708`: the verb is not in this dictionary. Keep the result — M11's
    /// creators answer this having done the work — but never read a value.
    NotHandled,
    /// `-1700`: our own vocabulary is wrong (M3). A bug in us, not a state
    /// of the machine, so it is loud.
    BadVocabulary,
    /// `-1728` / `-600`: the object or the app is gone. Drop the cached
    /// resolution and resolve again.
    Stale,
    /// Anything else, including [`ScriptError::Unavailable`].
    Other,
}

impl ScriptError {
    /// The AppleScript error number, when there was one.
    #[must_use]
    pub fn number(&self) -> Option<i32> {
        match self {
            ScriptError::Osa { number, .. } => Some(*number),
            ScriptError::Unavailable(_) => None,
        }
    }

    #[must_use]
    pub fn fault(&self) -> Fault {
        match self.number() {
            Some(ERR_NOT_PERMITTED) => Fault::Denied,
            Some(ERR_TIMEOUT) => Fault::Asleep,
            Some(ERR_NOT_HANDLED) => Fault::NotHandled,
            Some(ERR_COERCION) => Fault::BadVocabulary,
            Some(ERR_NO_SUCH_OBJECT | ERR_NO_PROCESS) => Fault::Stale,
            _ => Fault::Other,
        }
    }

    /// Automation is denied: `perm.status.automation.ghostty` and
    /// `integrations.status` both key off this one question.
    #[must_use]
    pub fn denied(&self) -> bool {
        self.fault() == Fault::Denied
    }

    /// Worth exactly one more attempt, after an `activate` (§5.5's
    /// "`-1712` means wake and retry").
    #[must_use]
    pub fn wake_and_retry(&self) -> bool {
        self.fault() == Fault::Asleep
    }

    /// The stable word the bus and the GUI key off, in the shape
    /// [`crate::audio::traits::AudioError::code`] already uses.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self.fault() {
            Fault::Denied => "automation_denied",
            Fault::Asleep => "asleep",
            Fault::NotHandled => "not_handled",
            Fault::BadVocabulary => "bad_vocabulary",
            Fault::Stale => "stale",
            Fault::Other => "script",
        }
    }
}

/// `osascript`'s stderr, as a [`ScriptError`].
///
/// The format is `<stdin>:1:10: execution error: Ghostty got an error:
/// Unknown key name: return. (-1700)` — the number is in the **last**
/// parenthesised group, which is why this reads from the right. A line with
/// no number at all is [`ScriptError::Unavailable`] and not a guessed
/// number: an unparsed stderr means we do not know what happened, and
/// inventing `-1` for it would put a made-up fact on the bus.
#[must_use]
pub fn parse_osascript_error(stderr: &str) -> ScriptError {
    let text = stderr.trim();
    let number = text
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.split_once(')'))
        .and_then(|(inside, _)| inside.trim().parse::<i32>().ok());
    match number {
        Some(number) => ScriptError::Osa {
            number,
            message: message_of(text),
        },
        None if text.is_empty() => {
            ScriptError::Unavailable("osascript failed and said nothing".into())
        }
        None => ScriptError::Unavailable(text.to_string()),
    }
}

/// The human part of an `osascript` error line: everything after its own
/// `execution error: ` prefix and before the trailing ` (-NNNN)`.
fn message_of(text: &str) -> String {
    // The **first** `error: `, not the last: `osascript` prefixes its own
    // `execution error: ` and the application's message then contains one
    // of its own ("Ghostty got an error: …"), which is part of what
    // happened and not noise.
    let body = text
        .split_once("error: ")
        .map_or(text, |(_, tail)| tail)
        .trim();
    let body = match body.rsplit_once('(') {
        Some((head, _)) => head.trim_end().trim_end_matches('.').trim(),
        None => body,
    };
    body.to_string()
}

/// Why the desktop side did not do what it was asked.
///
/// The `Script` arm carries the AppleScript error; every other arm is a
/// refusal this module made on its own, before or instead of scripting
/// anything. That split is load-bearing: a guard refusal must be
/// distinguishable from a machine failure, because the guards are the
/// security story and "the script failed" is not an acceptable way to
/// report "we would have typed into the wrong terminal".
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DesktopError {
    #[error("{0}")]
    Script(#[from] ScriptError),
    /// The beacon matched no surface, or more than one (§5.5 step 6).
    /// **Never** resolved to a guess.
    #[error("the beacon matched {0} surfaces")]
    Ambiguous(usize),
    /// The agent has no resolved terminal at all: a virtual agent's
    /// self-supplied `terminal`/`pid` is informational (§4.6) and is never
    /// an injection target.
    #[error("no resolved terminal for this agent")]
    NoTerminal,
    /// Guard 3 (M9, M10): the recorded pid is not the foreground process
    /// group of its recorded tty. **Nothing was sent, not even the text.**
    #[error("the cli pid is not the foreground of its tty")]
    NotForeground,
    /// Guard 2: a permission/question/elicitation card is open, or the CLI's
    /// own roster says it is waiting.
    #[error("a dialog is open for this agent")]
    DialogOpen,
    /// Guard 1: the text is not operator-originated, or the principal is
    /// not one that may aim it.
    #[error("not operator-originated text")]
    NotOperator,
    /// `[desktop] inject` does not list this terminal program.
    #[error("injection is not enabled for this terminal")]
    Disabled,
    /// The inbox route (`crate::inbox`): the session's socket could not be
    /// connected to or written. The word is the `order.failed{reason}`.
    #[error("inbox post failed: {0}")]
    Inbox(&'static str),
}

impl DesktopError {
    /// The `order.failed{reason}` word (§4.12). The three §5.5 names —
    /// `ambiguous`, `not_foreground`, `dialog_open` — are exactly the three
    /// the design promises; the rest carry their own cause rather than
    /// hiding under one of those.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            DesktopError::Ambiguous(_) => "ambiguous",
            DesktopError::NoTerminal => "no_terminal",
            DesktopError::NotForeground => "not_foreground",
            DesktopError::DialogOpen => "dialog_open",
            DesktopError::NotOperator => "not_operator",
            DesktopError::Disabled => "disabled",
            DesktopError::Inbox(reason) => reason,
            DesktopError::Script(e) => e.code(),
        }
    }

    /// Whether the cached resolution is now worthless (§5.5's 30 s cache is
    /// an optimisation, not a promise that the tab still exists).
    #[must_use]
    pub fn invalidates_resolution(&self) -> bool {
        match self {
            DesktopError::Ambiguous(_) => true,
            DesktopError::Script(e) => matches!(e.fault(), Fault::Stale),
            _ => false,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Fault::Denied => "denied",
            Fault::Asleep => "asleep",
            Fault::NotHandled => "not_handled",
            Fault::BadVocabulary => "bad_vocabulary",
            Fault::Stale => "stale",
            Fault::Other => "other",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four numbers §3 names, each with the reaction §5.5 gives it.
    /// M13: this test is the *only* evidence for `-1743` and `-1712`, and
    /// it is evidence about the mapping and not about the machine.
    #[test]
    fn each_applescript_number_maps_to_the_reaction_the_design_gives_it() {
        let cases = [
            (ERR_NOT_PERMITTED, Fault::Denied, "automation_denied"),
            (ERR_TIMEOUT, Fault::Asleep, "asleep"),
            (ERR_NOT_HANDLED, Fault::NotHandled, "not_handled"),
            (ERR_COERCION, Fault::BadVocabulary, "bad_vocabulary"),
            (ERR_NO_SUCH_OBJECT, Fault::Stale, "stale"),
            (ERR_NO_PROCESS, Fault::Stale, "stale"),
            (ERR_NOT_MODIFIABLE, Fault::Other, "script"),
        ];
        for (number, fault, code) in cases {
            let e = ScriptError::Osa {
                number,
                message: "x".into(),
            };
            assert_eq!(e.fault(), fault, "{number}");
            assert_eq!(e.code(), code, "{number}");
            assert_eq!(e.number(), Some(number));
        }
        let denied = ScriptError::Osa {
            number: ERR_NOT_PERMITTED,
            message: "Not authorized".into(),
        };
        assert!(denied.denied() && !denied.wake_and_retry());
        let asleep = ScriptError::Osa {
            number: ERR_TIMEOUT,
            message: "AppleEvent timed out".into(),
        };
        assert!(asleep.wake_and_retry() && !asleep.denied());
        // No engine is not a number, and must not acquire one.
        let none = ScriptError::Unavailable("osascript: not found".into());
        assert_eq!(none.number(), None);
        assert_eq!(none.fault(), Fault::Other);
        assert!(!none.denied() && !none.wake_and_retry());
    }

    /// M3's exact stderr, and the three shapes around it.
    #[test]
    fn osascript_stderr_is_parsed_from_the_right_and_never_invents_a_number() {
        let m3 = parse_osascript_error(
            "30:118: execution error: Ghostty got an error: Unknown key name: return (-1700)",
        );
        assert_eq!(
            m3,
            ScriptError::Osa {
                number: ERR_COERCION,
                message: "Ghostty got an error: Unknown key name: return".into(),
            }
        );
        // A message with its own parentheses: the number is the last group.
        let nested = parse_osascript_error(
            "execution error: Ghostty got an error: Can’t get terminal 1 (of 2) whose id = \"x\". \
             (-1728)",
        );
        assert_eq!(nested.number(), Some(ERR_NO_SUCH_OBJECT));
        assert_eq!(nested.fault(), Fault::Stale);
        // Nothing parsable: "we do not know", never a made-up number.
        let unknown = parse_osascript_error("zsh: killed  osascript");
        assert_eq!(unknown.number(), None);
        assert!(matches!(unknown, ScriptError::Unavailable(_)));
        assert!(matches!(
            parse_osascript_error("   "),
            ScriptError::Unavailable(_)
        ));
    }

    /// The `order.failed{reason}` vocabulary, which the device and the GUI
    /// both branch on.
    #[test]
    fn every_desktop_refusal_carries_the_reason_the_wire_publishes() {
        assert_eq!(DesktopError::Ambiguous(3).reason(), "ambiguous");
        assert_eq!(DesktopError::NotForeground.reason(), "not_foreground");
        assert_eq!(DesktopError::DialogOpen.reason(), "dialog_open");
        assert_eq!(DesktopError::NoTerminal.reason(), "no_terminal");
        assert_eq!(DesktopError::NotOperator.reason(), "not_operator");
        assert_eq!(DesktopError::Disabled.reason(), "disabled");
        assert_eq!(
            DesktopError::Script(ScriptError::Osa {
                number: ERR_NOT_PERMITTED,
                message: String::new(),
            })
            .reason(),
            "automation_denied"
        );
        // A stale object and an ambiguous beacon both throw the cache away;
        // a guard refusal does not — the tab is still the right tab.
        assert!(DesktopError::Ambiguous(0).invalidates_resolution());
        assert!(DesktopError::Script(ScriptError::Osa {
            number: ERR_NO_SUCH_OBJECT,
            message: String::new(),
        })
        .invalidates_resolution());
        assert!(!DesktopError::NotForeground.invalidates_resolution());
        assert!(!DesktopError::DialogOpen.invalidates_resolution());
    }
}
