//! `deckd uninstall [--purge]` (design §10.5, §15's exit test).
//!
//! The exit test is one sentence — **uninstall leaves the CLIs' configs as
//! they were** — and week 3 already built the machinery that makes it true:
//! every installer write goes through `integrations::backup`, which *moves*
//! the original into `<data>/backups/` before publishing, and
//! `Verb::Uninstall` takes back exactly the entries this installer appended
//! and puts the displaced bytes back. So this module does not re-implement
//! any of that; it decides *what to run and in what order*, and the order is
//! the part that can go wrong:
//!
//! 1. **the CLIs first.** `--purge` deletes the data directory, and
//!    `<data>/backups/` is inside it. Purging before uninstalling would throw
//!    away the only copy of the operator's original `settings.json`, which is
//!    the exact failure the exit test exists to catch.
//! 2. **the LaunchAgent next**, so nothing restarts the daemon while its
//!    files are being taken apart.
//! 3. **the data directory last**, and only with `--purge`.
//!
//! Two things this deliberately does **not** do, and says so instead:
//! `SMAppService.unregister()` for the agent and the `mainApp` login item
//! is the GUI's call (there is no Rust binding for it here, and a
//! `launchctl bootout` of an SMAppService registration is not the same
//! thing); and the Keychain item and `tccutil reset All
//! ai.cohdl.openwalkietalkie` are **printed, not run** — a CLI that silently
//! revokes microphone grants is a CLI nobody can undo.
//!
//! Logs are kept, always. They are the record of what ran, they contain
//! nothing content-bearing unless the operator turned `[privacy]
//! log_content` on, and an uninstall that deletes the evidence of why the
//! operator is uninstalling is the wrong shape.
use crate::config::Paths;
use crate::integrations::{self, Launchctl, Status, Target, Verb};
use std::path::{Path, PathBuf};

/// The v1 targets that write into a CLI's own files.
pub const CLI_TARGETS: [Target; 2] = [Target::Claude, Target::Codex];

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Also delete the data directory, the caches and the GUI's preferences.
    pub purge: bool,
}

/// What the run did, in the order it did it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Outcome {
    /// One line per thing that happened.
    pub steps: Vec<String>,
    /// Files put back, for the byte-for-byte claim.
    pub restored: Vec<PathBuf>,
    /// What was deliberately left behind, and why.
    pub kept: Vec<String>,
    /// What the operator still has to do themselves.
    pub next_steps: Vec<String>,
    /// What did not work. A non-empty list is a non-zero exit.
    pub problems: Vec<String>,
}

impl Outcome {
    pub fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.steps {
            out.push_str(&format!("  {line}\n"));
        }
        if !self.kept.is_empty() {
            out.push_str("\nkept:\n");
            for line in &self.kept {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if !self.next_steps.is_empty() {
            out.push_str("\nstill yours to do:\n");
            for line in &self.next_steps {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if !self.problems.is_empty() {
            out.push_str("\nPROBLEMS:\n");
            for line in &self.problems {
                out.push_str(&format!("  {line}\n"));
            }
        }
        out
    }
}

/// Take deckd back out. See the module docs for why the order is the order.
pub fn run(env: &integrations::Env, paths: &Paths, opts: Options) -> Outcome {
    let mut outcome = Outcome::default();

    // 1. The CLIs' own files, before anything can delete `<data>/backups`.
    for target in CLI_TARGETS {
        let installed = integrations::status(env, Some(target))
            .first()
            .map(|report| report.status != Status::Absent)
            .unwrap_or(false);
        if !installed {
            outcome
                .steps
                .push(format!("{}: nothing installed", target.display_name()));
            continue;
        }
        match integrations::run(env, target, Verb::Uninstall) {
            Ok(receipt) => {
                outcome.steps.push(format!(
                    "{}: {} file(s) put back as they were",
                    target.display_name(),
                    receipt.changed.len()
                ));
                outcome.restored.extend(receipt.changed);
                outcome.next_steps.extend(receipt.next_steps);
                for conflict in receipt.conflicts {
                    outcome.problems.push(conflict);
                }
            }
            Err(why) => outcome
                .problems
                .push(format!("{}: {why}", target.display_name())),
        }
    }

    // 2. The LaunchAgent: the loaded job, then a plist on disk. Both halves
    //    are needed — `deckd install --dev` writes a plist into
    //    `~/Library/LaunchAgents` and bootstraps it, and either can outlive
    //    the other.
    //    The agent under the project's old name goes the same way, quietly
    //    when it was never there.
    for label in [
        crate::launchd::DECKD_LABEL,
        crate::launchd::LEGACY_DECKD_LABEL,
    ] {
        let legacy = label == crate::launchd::LEGACY_DECKD_LABEL;
        match bootout(env, label) {
            Ok(true) => outcome.steps.push(format!(
                "launchd: booted {label} out of gui/{}",
                crate::launchd::uid()
            )),
            Ok(false) if !legacy => outcome
                .steps
                .push(format!("launchd: {label} was not loaded")),
            Ok(false) => {}
            Err(why) => outcome.problems.push(why),
        }
        let plist = env
            .home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        if plist.exists() {
            match std::fs::remove_file(&plist) {
                Ok(()) => outcome
                    .steps
                    .push(format!("launchd: removed {}", plist.display())),
                Err(e) => outcome
                    .problems
                    .push(format!("cannot remove {}: {e}", plist.display())),
            }
        }
    }
    outcome.next_steps.push(
        "the app itself: quit OpenWalkieTalkie and drag it to the Trash. Its SMAppService \
         registration and the `mainApp` login item are unregistered by the app (Settings → \
         Uninstall…), not by this command."
            .into(),
    );

    // 3. The data, last, and only when asked.
    if opts.purge {
        for dir in [&paths.data, &paths.caches] {
            match remove_dir(dir) {
                Ok(true) => outcome.steps.push(format!("purged {}", dir.display())),
                Ok(false) => outcome
                    .steps
                    .push(format!("purged {} (was absent)", dir.display())),
                Err(e) => outcome.problems.push(e),
            }
        }
        let prefs = env
            .home
            .join("Library/Preferences/ai.cohdl.openwalkietalkie.plist");
        if prefs.exists() {
            match std::fs::remove_file(&prefs) {
                Ok(()) => outcome.steps.push(format!("purged {}", prefs.display())),
                Err(e) => outcome
                    .problems
                    .push(format!("cannot remove {}: {e}", prefs.display())),
            }
        }
        outcome.next_steps.push(
            "the Keychain item `ai.cohdl.openwalkietalkie` (cloud STT/TTS keys, if you ever set \
             one) is not deleted by this command: remove it in Keychain Access."
                .into(),
        );
        outcome.next_steps.push(
            "to give back the TCC grants as well, run `tccutil reset All \
             ai.cohdl.openwalkietalkie` — our own bundle id and nothing else. This command does \
             not run it for you."
                .into(),
        );
    } else {
        outcome.kept.push(format!(
            "{} — settings, tokens and the deck's remembered slots. `deckd uninstall --purge` \
             deletes them.",
            paths.data.display()
        ));
    }
    outcome.kept.push(format!(
        "{} — the logs are the record of what ran and are never deleted, purge or not.",
        paths.logs.display()
    ));
    outcome
}

/// Remove a directory tree; `Ok(false)` when it was not there.
fn remove_dir(dir: &Path) -> Result<bool, String> {
    if !dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(dir)
        .map(|()| true)
        .map_err(|e| format!("cannot remove {}: {e}", dir.display()))
}

/// `launchctl bootout gui/<uid>/<label>`, guarded by a `print` so an absent
/// job is not an error, and disabled outright in a test environment — a
/// `bootout` from a unit test would stop the operator's own running service.
fn bootout(env: &integrations::Env, label: &str) -> Result<bool, String> {
    if env.launchctl == Launchctl::Disabled {
        return Ok(false);
    }
    let service = format!("gui/{}/{label}", crate::launchd::uid());
    if crate::launchd::print(label).is_none() {
        return Ok(false);
    }
    let out = std::process::Command::new("/bin/launchctl")
        .args(["bootout", &service])
        .output()
        .map_err(|e| format!("cannot run launchctl: {e}"))?;
    if out.status.success() {
        return Ok(true);
    }
    Err(format!(
        "`launchctl bootout {service}` failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::testdir::TestDir;

    /// A `Paths` whose data directory is the test environment's, so a purge
    /// deletes the throwaway tree and nothing else.
    fn paths_for(env: &integrations::Env, dir: &TestDir) -> Paths {
        Paths::from_data_dir(env.data.clone(), dir.sub("logs"), dir.sub("caches"))
    }

    #[test]
    fn launchctl_is_never_run_from_a_test_environment() {
        let dir = TestDir::new("uninstall-launchctl");
        let env = integrations::test_env(&dir);
        assert_eq!(env.launchctl, Launchctl::Disabled);
        assert_eq!(bootout(&env, crate::launchd::DECKD_LABEL), Ok(false));
    }

    /// Without `--purge` the data directory stays, and the logs stay either
    /// way — both of them as a line the operator can read, not silently.
    #[test]
    fn data_stays_without_purge_and_the_logs_stay_with_it() {
        let dir = TestDir::new("uninstall-keep");
        let env = integrations::test_env(&dir);
        let paths = paths_for(&env, &dir);
        std::fs::write(paths.state.as_path(), b"{}").unwrap();
        let outcome = run(&env, &paths, Options { purge: false });
        assert!(paths.data.is_dir(), "the data dir stays");
        assert!(paths.state.exists(), "and what is in it");
        assert!(
            outcome.kept.iter().any(|k| k.contains("--purge")),
            "{:?}",
            outcome.kept
        );
        assert!(
            outcome.kept.iter().any(|k| k.contains("logs")),
            "{:?}",
            outcome.kept
        );
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
    }

    #[test]
    fn purge_deletes_the_data_dir_and_the_caches_and_says_so() {
        let dir = TestDir::new("uninstall-purge");
        let env = integrations::test_env(&dir);
        let paths = paths_for(&env, &dir);
        std::fs::write(paths.state.as_path(), b"{}").unwrap();
        std::fs::write(paths.caches.join("chime.pcm"), b"x").unwrap();
        let outcome = run(&env, &paths, Options { purge: true });
        assert!(!paths.data.exists(), "{:?}", outcome.steps);
        assert!(!paths.caches.exists(), "{:?}", outcome.steps);
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
        // The two things a purge must *not* do quietly.
        assert!(
            outcome
                .next_steps
                .iter()
                .any(|s| s.contains("tccutil reset All")),
            "{:?}",
            outcome.next_steps
        );
        assert!(
            outcome.next_steps.iter().any(|s| s.contains("Keychain")),
            "{:?}",
            outcome.next_steps
        );
        // And the logs are still named as kept.
        assert!(outcome.kept.iter().any(|k| k.contains("never deleted")));
    }

    /// The ordering claim, as a test rather than as a comment: the backups
    /// live inside the data directory, so a purge that ran first would take
    /// the operator's original `settings.json` with it.
    #[test]
    fn a_purge_uninstalls_the_clis_before_it_deletes_the_backups_they_restore_from() {
        let dir = TestDir::new("uninstall-order");
        let env = integrations::test_env(&dir);
        let paths = paths_for(&env, &dir);
        // The one CLI file an install still merges into — Codex's
        // `hooks.json` (the Claude plugin lives in deckd's own data dir) —
        // is backed up on the way in and restored from that backup on the
        // way out, which is the case the order exists for.
        let settings = integrations::codex::hooks_path(&env);
        let before = b"{\n    \"description\": \"mine\",\n    \"hooks\": {}\n}\n".to_vec();
        std::fs::write(&settings, &before).unwrap();
        integrations::run(&env, Target::Codex, Verb::Install { trust: false }).unwrap();
        assert_ne!(std::fs::read(&settings).unwrap(), before, "install wrote");
        assert!(
            env.backups().is_dir() && env.backups().starts_with(&paths.data),
            "the backups are inside the data dir, which is what makes the order matter"
        );

        let outcome = run(&env, &paths, Options { purge: true });
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
        assert_eq!(
            std::fs::read(&settings).unwrap(),
            before,
            "the file came back before the backups were purged"
        );
        assert!(!paths.data.exists(), "and then the data dir went");
    }
}
