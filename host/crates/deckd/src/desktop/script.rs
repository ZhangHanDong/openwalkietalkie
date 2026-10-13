//! The scripting seam (HOST-CONTRACT-W5 §3, mandatory): **everything** in
//! `deckd::desktop` is written against [`Scripting`], so every resolver and
//! injector path has a test that needs no Ghostty, no window and no
//! Automation grant.
//!
//! That is not a testing convenience. The three guards of §5.5 are the only
//! thing between a dictated sentence and somebody else's shell prompt, and
//! a guard whose test needs a terminal open is a guard that is checked once
//! and then trusted for ever. With the seam, [`super::fakes`] can answer
//! "two surfaces carry your nonce" or "`-1743`" on demand and the refusal
//! is an ordinary assertion.
//!
//! Two implementations live behind it:
//!
//! * [`AppleScriptRunner`] — `NSAppleScript` compiled and executed **on the
//!   main thread**, through B10's existing hop
//!   ([`crate::audio::mainloop`]). This is what §5.5 prefers: M12 measured
//!   **~118 ms per `osascript` round trip** (129/118/119/118/118 ms), and a
//!   resolve-plus-pin is two of them. In-process removes the fork, the
//!   dynamic-linker warm-up and the extra TCC identity.
//! * [`OsaRunner`] — a child `osascript` reading the source on **stdin**.
//!   The documented fallback, and the only runner a non-main thread can use
//!   when the run loop is not turning.
//!
//! **What is not settled, and week 7 owns it.** TCC attributes an Apple
//! Event to the *responsible* process, which for a launchd-spawned `deckd`
//! is `deckd` itself and for a terminal-spawned one is the terminal (W4 V9,
//! and `core::dispatch::PERM_LIMITS` says so on the wire). A child
//! `osascript` is a *different* process and may therefore be attributed
//! differently from an in-process `NSAppleScript` on the same daemon. M13
//! could not reproduce `-1743` on this machine at all, so **neither runner's
//! TCC behaviour is claimed here**: the shipped bundle's
//! `NSAppleEventsUsageDescription` + `com.apple.security.automation
//! .apple-events` path (§10.4) is week 7's, and that is when the question
//! "which of these two does the grant follow?" can be answered with a
//! measurement instead of a paragraph. Both runners are built, both are
//! exercised by the same trait tests, and [`default_runner`] picks
//! `NSAppleScript` when the main run loop is available and `osascript`
//! otherwise.
use super::errors::{parse_osascript_error, ScriptError};
use std::sync::Arc;

/// How long a script may take before the caller gives up on it.
///
/// Generous against M12's 118 ms, because the numbers that matter are the
/// bad ones: `-1712` came from an instance whose main thread was provably
/// idle, and a Ghostty being woken from a long idle has no published bound.
/// Finite, because the core is waiting on the other end of a channel.
pub const SCRIPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// One AppleScript, run somewhere, with its result as text.
///
/// `&self` and `Send + Sync`: the desktop worker holds one runner for the
/// daemon's life and the hardware test holds the same one, so a runner that
/// needed `&mut` would put a lock in front of a 118 ms call.
pub trait Scripting: Send + Sync + std::fmt::Debug {
    /// Compile and run `src`; the value of the last statement, as text.
    fn run(&self, src: &str) -> Result<String, ScriptError>;

    /// For logs and for `integrations.status`: which runner answered.
    fn name(&self) -> &'static str;
}

impl Scripting for Arc<dyn Scripting> {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        (**self).run(src)
    }

    fn name(&self) -> &'static str {
        (**self).name()
    }
}

/// `s` as an AppleScript string literal, quotes and all.
///
/// AppleScript escapes exactly two characters inside a literal — `"` and
/// `\` — and everything else goes through as itself. Control characters are
/// **not** escaped here on purpose: `op.order` rejects them with `-32009`
/// before the text ever reaches this module (§5.5), and a quoter that
/// quietly encoded a `\r` would turn that refusal into a Return somebody
/// did not ask for.
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// osascript
// ---------------------------------------------------------------------------

/// A child `osascript`, fed on stdin.
///
/// **On stdin and not `-e`.** One `-e` argument per line is how the
/// week-5 probes drove it by hand, and it has two properties this module
/// cannot accept: a multi-line script has to be split by the caller (the
/// probes' `ghb` helper exists for exactly that), and the whole source ends
/// up in `argv`, where `ps` shows it to every process on the machine. An
/// order is content (§4.3). stdin is neither split nor public.
#[derive(Clone, Debug)]
pub struct OsaRunner {
    bin: String,
    budget: std::time::Duration,
}

impl Default for OsaRunner {
    fn default() -> Self {
        OsaRunner::new()
    }
}

impl OsaRunner {
    #[must_use]
    pub fn new() -> OsaRunner {
        OsaRunner {
            // Absolute: `PATH` belongs to whoever launched the daemon, and
            // an `osascript` earlier on it would be running as us.
            bin: "/usr/bin/osascript".into(),
            budget: SCRIPT_BUDGET,
        }
    }

    /// Another binary and budget — the tests' hook, and nothing else uses
    /// it.
    #[must_use]
    pub fn with_bin(bin: impl Into<String>, budget: std::time::Duration) -> OsaRunner {
        OsaRunner {
            bin: bin.into(),
            budget,
        }
    }
}

impl Scripting for OsaRunner {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};

        let mut child = Command::new(&self.bin)
            .arg("-") // read the script from stdin
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ScriptError::Unavailable(format!("{}: {e}", self.bin)))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A write error is not fatal by itself: the child may have
            // exited already and its stderr is the better message.
            let _ = stdin.write_all(src.as_bytes());
        }
        // The deadline is polled rather than awaited for the same reason
        // `liveness::Probe::rows` polls: this runs on a plain std thread
        // with no reactor, and a `wait()` with no timeout is how a wedged
        // Ghostty takes the desktop worker with it.
        let deadline = std::time::Instant::now() + self.budget;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(ScriptError::Unavailable(format!(
                            "osascript did not finish within {:?}",
                            self.budget
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(ScriptError::Unavailable(e.to_string())),
            }
        };
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let mut err = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        if status.success() {
            // `osascript` adds a trailing newline to the value it prints;
            // a title that really ends in whitespace is not a thing the
            // resolver can tell apart from that, and does not need to be.
            Ok(out.trim_end_matches('\n').to_string())
        } else {
            Err(parse_osascript_error(&err))
        }
    }

    fn name(&self) -> &'static str {
        "osascript"
    }
}

// ---------------------------------------------------------------------------
// NSAppleScript, on the main thread
// ---------------------------------------------------------------------------

/// `NSAppleScript`, compiled and executed on the main thread.
///
/// The main-thread rule is not a guess about thread safety: `NSAppleScript`
/// is documented as not thread-safe, §2.3 already puts it on `main` beside
/// the TCC prompts and the AVSpeech callbacks, and the hop it uses is the
/// one B10 built for those. The objects are created *inside* the posted
/// closure and dropped there, so nothing ObjC ever crosses a thread —
/// exactly the discipline `audio::apple` keeps.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Default)]
pub struct AppleScriptRunner {
    _private: (),
}

#[cfg(target_os = "macos")]
impl AppleScriptRunner {
    #[must_use]
    pub const fn new() -> AppleScriptRunner {
        AppleScriptRunner { _private: () }
    }
}

#[cfg(target_os = "macos")]
impl Scripting for AppleScriptRunner {
    fn run(&self, src: &str) -> Result<String, ScriptError> {
        let src = src.to_string();
        let job = move || execute_on_main(&src);
        if crate::audio::mainloop::is_main_thread() {
            // Posting from main and then waiting is a self-deadlock for the
            // whole budget: the queue drains on main, and main is the
            // thread that would be waiting (`audio::apple::on_main` makes
            // the same call for the same reason).
            return job();
        }
        crate::audio::mainloop::call(job)
            .recv_timeout(SCRIPT_BUDGET)
            .unwrap_or_else(|_| {
                Err(ScriptError::Unavailable(
                    "the main run loop did not answer; is `deckd run` pumping it?".into(),
                ))
            })
    }

    fn name(&self) -> &'static str {
        "nsapplescript"
    }
}

/// The body of [`AppleScriptRunner::run`], on `main`.
#[cfg(target_os = "macos")]
fn execute_on_main(src: &str) -> Result<String, ScriptError> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleScript, NSAppleScriptErrorMessage, NSAppleScriptErrorNumber,
        NSDictionary, NSNumber, NSString,
    };

    let source = NSString::from_str(src);
    let Some(script) = NSAppleScript::initWithSource(NSAppleScript::alloc(), &source) else {
        return Err(ScriptError::Unavailable(
            "NSAppleScript refused the source".into(),
        ));
    };
    let mut info: Option<Retained<NSDictionary<NSString, AnyObject>>> = None;
    // `executeAndReturnError:` returns **nil** on failure, which the
    // generated non-optional binding cannot represent, so the message is
    // sent by hand with an `Option<Retained<_>>` return. The selector is
    // in the `none` method family, so objc2 retains the autoreleased
    // result for us.
    // SAFETY: the selector, its one out-parameter and the return type are
    // the ones Foundation documents for `NSAppleScript`.
    let descriptor: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { objc2::msg_send![&*script, executeAndReturnError: &mut info] };
    if let Some(info) = info {
        // SAFETY: `NSAppleScriptErrorNumber` and `…ErrorMessage` are the
        // framework's own keys; the values are an NSNumber and an NSString
        // when present, which is what the casts below check for.
        let (number, message) = unsafe {
            let number = info
                .objectForKey(NSAppleScriptErrorNumber)
                .and_then(|v| Retained::downcast::<NSNumber>(v).ok())
                .map_or(0, |n| n.as_i32());
            let message = info
                .objectForKey(NSAppleScriptErrorMessage)
                .and_then(|v| Retained::downcast::<NSString>(v).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            (number, message)
        };
        return Err(ScriptError::Osa { number, message });
    }
    match descriptor {
        // A script whose last statement has no value (`pin` is one) answers
        // with a descriptor whose `stringValue` is nil. That is a success
        // with nothing to say, not a failure.
        Some(descriptor) => Ok(descriptor
            .stringValue()
            .map(|s| s.to_string())
            .unwrap_or_default()),
        None => Err(ScriptError::Unavailable(
            "NSAppleScript returned neither a result nor an error".into(),
        )),
    }
}

/// The runner the daemon uses.
///
/// `NSAppleScript` when the main run loop is this process's to turn (M12:
/// in-process saves the fork on every one of the two round trips a
/// resolve-and-pin costs), `osascript` otherwise — which is every test
/// binary, because libtest owns `main` and never pumps a run loop, and a
/// hop that is never drained would answer `Unavailable` after five seconds.
#[cfg(target_os = "macos")]
#[must_use]
pub fn default_runner(in_process: bool) -> Arc<dyn Scripting> {
    if in_process {
        Arc::new(AppleScriptRunner::new())
    } else {
        Arc::new(OsaRunner::new())
    }
}

/// §13's Linux port has no Apple Events at all; the seam survives and the
/// runner does not.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn default_runner(_in_process: bool) -> Arc<dyn Scripting> {
    Arc::new(super::fakes::NoScripting)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::errors::ERR_COERCION;

    /// AppleScript escapes two characters and no others, and a quoter that
    /// "helpfully" encoded a control character would undo `-32009`.
    #[test]
    fn a_quoted_literal_escapes_only_the_two_characters_applescript_escapes() {
        assert_eq!(quote("hello"), "\"hello\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote("C:\\path"), "\"C:\\\\path\"");
        assert_eq!(quote(""), "\"\"");
        // Non-ASCII is a literal byte sequence, not an escape.
        assert_eq!(quote("café ☕"), "\"café ☕\"");
        // A tab survives unchanged — `op.order` allows it and nothing here
        // has an opinion.
        assert_eq!(quote("a\tb"), "\"a\tb\"");
    }

    /// The real child-process runner, against `/bin/cat`-like stand-ins
    /// rather than Ghostty: `run` has to turn an exit status and a stderr
    /// line into the right [`ScriptError`], and that is testable with any
    /// program at all.
    #[test]
    fn the_osascript_runner_reports_the_childs_verdict_and_never_its_own() {
        // A "script engine" that echoes its stdin: proves the source
        // reaches the child on stdin and the value comes back trimmed.
        let echo = OsaRunner::with_bin("/bin/cat", std::time::Duration::from_secs(5));
        assert_eq!(echo.name(), "osascript");
        assert_eq!(echo.run("a value\n").unwrap(), "a value");

        // A missing binary is `Unavailable` with no number invented for it.
        let absent =
            OsaRunner::with_bin("/nonexistent/osascript", std::time::Duration::from_secs(5));
        let err = absent.run("anything").unwrap_err();
        assert_eq!(err.number(), None);
        assert!(matches!(err, ScriptError::Unavailable(_)), "{err:?}");

        // A non-zero exit whose stderr carries M3's number becomes that
        // number, through the same path the real engine takes.
        let dir = std::env::temp_dir().join(format!("deckd-desktop-osa-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("osascript");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncat >/dev/null\n\
             echo '30:118: execution error: Ghostty got an error: Unknown key name: return \
             (-1700)' >&2\nexit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let failing = OsaRunner::with_bin(
            fake.to_string_lossy().to_string(),
            std::time::Duration::from_secs(5),
        );
        let err = failing.run("send key \"return\"").unwrap_err();
        assert_eq!(err.number(), Some(ERR_COERCION));

        // And the budget is a budget: a child that never exits is killed
        // and reported, rather than holding the desktop worker for ever.
        let hang = dir.join("hang");
        std::fs::write(&hang, "#!/bin/sh\nexec sleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let wedged = OsaRunner::with_bin(
            hang.to_string_lossy().to_string(),
            std::time::Duration::from_millis(200),
        );
        let started = std::time::Instant::now();
        let err = wedged.run("anything").unwrap_err();
        assert!(matches!(err, ScriptError::Unavailable(_)), "{err:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `Arc<dyn Scripting>` is itself `Scripting`, so a resolver can be
    /// built over either the concrete runner or the shared one without a
    /// second generic parameter.
    #[test]
    fn a_shared_runner_forwards_the_trait_it_stands_for() {
        let shared: Arc<dyn Scripting> = Arc::new(OsaRunner::with_bin(
            "/bin/cat",
            std::time::Duration::from_secs(5),
        ));
        assert_eq!(Scripting::name(&shared), "osascript");
        assert_eq!(Scripting::run(&shared, "x").unwrap(), "x");
    }
}
