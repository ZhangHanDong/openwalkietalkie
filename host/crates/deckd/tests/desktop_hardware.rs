#![cfg(target_os = "macos")]
//! The `--hardware` rows of week 5: real Ghostty 1.3.1, real ttys, real
//! `proc_pidinfo` — and nothing else.
//!
//! Every test here is `#[ignore]`d: CI runs `cargo test`, which skips them,
//! and `tools/host-test.sh --hardware` runs them with `--ignored`. Each one
//! **prints `skip` with a reason and passes** when Ghostty is absent or
//! Automation is denied (`-1743`). §7's rule is that a missing grant is a
//! skip and never a failure, and a gate that went red on a Mac without
//! Ghostty would be turned off within a week.
//!
//! Four rules the harness is built around, each of which cost somebody
//! time:
//!
//! * **Never `open -na`.** It gives *every* launch its own process, and
//!   `tell application "Ghostty"` reaches exactly one of them. Measured
//!   2026-09-22 with three running: the enumeration showed one instance's
//!   surfaces and answered `-1728` for a `terminal.id` belonging to
//!   another, so a row that opened its own instance could find itself
//!   scripting somebody else's. Every surface here is created **through
//!   AppleScript** (`new window with configuration {command:…}`), which
//!   puts it in the instance this `tell` reaches by construction — and
//!   which works from zero windows, launching Ghostty as a side effect
//!   (measured: `1.3.1/0` windows, then `window id tab-group-…`).
//! * **`tell … to <stmt>` takes one statement.** A multi-line body after
//!   `to` dies with `-1708`. `desktop::ghostty` already wraps every
//!   multi-statement script in a real `tell … end tell` block, which is why
//!   these rows can call it directly.
//! * **macOS has no `timeout(1)`.** Everything that waits here polls a file
//!   with a deadline.
//! * **The control surface is `exec cat > file`.** Nothing is ever
//!   *executed* in it: every injected keystroke lands in a file the row
//!   asserts on, and `send key "enter"` is what flushes the line. That is
//!   also the only reason it is safe to inject at all — §6 forbids typing
//!   anything destructive into a terminal, ever, and a `cat` cannot run a
//!   command even if one arrived.
//!
//! Leaving it as found (§6), in a drop guard so a panicking row cannot skip
//! it: every tab a row pins is unpinned by the code under test, the capture
//! processes are killed, each surface the row created is closed with
//! `perform action "close_window"` (measured `true`, window count 1 → 0, no
//! confirmation sheet, because the command has already exited), and the
//! application itself is quit **only** when the row found it with no
//! windows at all. An operator who had Ghostty open keeps it.
use deckd::desktop::fg::{self, Verdict};
use deckd::desktop::script::{OsaRunner, Scripting};
use deckd::desktop::{ghostty, Desktop, DesktopConfig, DesktopJob, FromDesktop, Resolver, Target};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn skip(reason: &str) {
    println!("skip  {reason}");
}

/// A private scratch dir for one row.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("deckd-w5hw-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the row's scratch dir");
    dir
}

/// Wait for `path` to be non-empty, then read it trimmed.
fn wait_for(path: &Path, budget: Duration) -> Option<String> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// Wait until `path` contains `needle`.
fn wait_until_contains(path: &Path, needle: &str, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|t| t.contains(needle)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// One Ghostty surface a row owns: a known tty, a known pid, the
/// `terminal.id` AppleScript addresses it by, and a file that captures
/// every keystroke that reaches it.
struct Capture {
    id: String,
    tty: String,
    pid: u32,
    got: PathBuf,
}

impl Capture {
    /// Push an OSC 2 title write straight at the tty — the beacon, or the
    /// hostile retitle that stands for the CLI's own title write.
    fn osc2(&self, title: &str) {
        let _ = std::fs::write(&self.tty, format!("\u{1b}]2;{title}\u{7}"));
        std::thread::sleep(Duration::from_millis(600));
    }
}

/// The Ghostty this row talks to, and everything it has to put back.
struct Ghost {
    runner: OsaRunner,
    dir: PathBuf,
    /// Windows the row found before it created any. Quitting the
    /// application is only allowed when this is zero.
    windows_at_start: u32,
    /// `terminal.id`s the row created, oldest first.
    created: Vec<String>,
    /// …and the capture shells behind them.
    pids: Vec<u32>,
}

impl Ghost {
    /// Probe Ghostty and take charge of it, or print the reason there is
    /// nothing to do. The missing app and the missing grant are both skips.
    fn start(dir: PathBuf) -> Option<Ghost> {
        if !Path::new("/Applications/Ghostty.app").exists() {
            skip("/Applications/Ghostty.app is absent — this row needs it");
            return None;
        }
        let runner = OsaRunner::new();
        let windows_at_start = match ghostty::probe(&runner) {
            Ok((version, windows)) => {
                println!("note  ghostty {version}, {windows} window(s) before this row");
                windows
            }
            Err(e) if e.denied() => {
                skip(&format!(
                    "Automation is denied for Ghostty (-1743): {e}. A missing grant is never a \
                     failure"
                ));
                return None;
            }
            Err(e) => {
                skip(&format!("Ghostty is not scriptable: {e}"));
                return None;
            }
        };
        Some(Ghost {
            runner,
            dir,
            windows_at_start,
            created: Vec::new(),
            pids: Vec::new(),
        })
    }

    /// A new surface in **this** instance, running nothing but `cat`.
    ///
    /// M11: `new window with configuration` may answer with a window id or
    /// with `-1708` having created the window anyway, so the return value
    /// is ignored — the surface's own `tty` file is what says whether it
    /// came up, and a re-enumeration is what names it.
    fn surface(&mut self, tag: &str) -> Option<Capture> {
        let script = self.dir.join(format!("cap-{tag}.sh"));
        let tty_file = self.dir.join(format!("tty-{tag}"));
        let pid_file = self.dir.join(format!("pid-{tag}"));
        let got = self.dir.join(format!("got-{tag}"));
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ntty > {}\necho $$ > {}\nexec cat > {}\n",
                tty_file.display(),
                pid_file.display(),
                got.display()
            ),
        )
        .expect("the capture script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod the capture script");

        let before: Vec<String> = ghostty::enumerate(&self.runner)
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.id)
            .collect();
        let _ = self.runner.run(&format!(
            "tell application \"Ghostty\" to new window with configuration {{command:\"{}\"}}\n",
            script.display()
        ));
        let tty = wait_for(&tty_file, Duration::from_secs(20))?;
        let pid: u32 = wait_for(&pid_file, Duration::from_secs(20))?.parse().ok()?;
        let id = ghostty::enumerate(&self.runner)
            .ok()?
            .into_iter()
            .map(|s| s.id)
            .find(|id| !before.contains(id))?;
        self.created.push(id.clone());
        self.pids.push(pid);
        Some(Capture { id, tty, pid, got })
    }
}

impl Drop for Ghost {
    fn drop(&mut self) {
        for pid in &self.pids {
            // SAFETY: a `kill` of a pid this row started itself.
            unsafe { libc::kill(*pid as libc::pid_t, libc::SIGTERM) };
        }
        std::thread::sleep(Duration::from_millis(400));
        for id in &self.created {
            let _ = self.runner.run(&format!(
                "tell application \"Ghostty\" to perform action \"close_window\" on (first \
                 terminal whose id is \"{id}\")\n"
            ));
        }
        std::thread::sleep(Duration::from_millis(600));
        if self.windows_at_start == 0 {
            // The row found nothing open, so this is the row's Ghostty to
            // quit. An operator who had one open keeps it.
            let _ = self.runner.run("tell application \"Ghostty\" to quit\n");
            std::thread::sleep(Duration::from_millis(1000));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Rows 1, 2 and 5 of the live gate: the beacon on `terminal.name`, the pin
/// surviving a hostile OSC 2 on `tab.name`, and the empty value restoring
/// the live title.
///
/// This is the resolver's whole premise (M6) and the one claim no unit test
/// can make: the fake is *built* to keep the two fields apart, so only the
/// real application can say that it does.
#[test]
#[ignore = "hardware: opens a Ghostty window and pins a tab"]
fn hw_the_beacon_the_pin_and_the_unpin_against_real_ghostty() {
    let Some(mut ghost) = Ghost::start(scratch("beacon")) else {
        return;
    };
    let Some(cap) = ghost.surface("a") else {
        skip("the control surface never reported its tty");
        return;
    };
    println!("note  control surface at {} (pid {})", cap.tty, cap.pid);

    // ── row 1: beacon ───────────────────────────────────────────────────
    let nonce = format!("{:04x}", std::process::id() & 0xffff);
    cap.osc2(&ghostty::beacon_name(&nonce));
    let surfaces = match ghostty::enumerate(&ghost.runner) {
        Ok(surfaces) => surfaces,
        Err(e) => {
            skip(&format!("the enumeration failed: {e}"));
            return;
        }
    };
    let wanted = ghostty::beacon_name(&nonce);
    let found: Vec<_> = surfaces.iter().filter(|s| s.beacons(&wanted)).collect();
    assert_eq!(
        found.len(),
        1,
        "beacon: exactly one surface carries {wanted} on terminal.name, got {surfaces:#?}"
    );
    assert_eq!(found[0].id, cap.id, "…and it is the surface we created");
    assert_eq!(cap.id.len(), 36, "terminal.id is a UUID: {}", cap.id);
    println!("ok    beacon found on terminal.name -> {}", cap.id);

    // ── row 2: the pin survives a hostile OSC 2 ─────────────────────────
    ghostty::pin(&ghost.runner, &cap.id, &nonce).expect("the pin");
    std::thread::sleep(Duration::from_millis(400));
    cap.osc2("HOSTILE_RETITLE");
    let after = ghostty::enumerate(&ghost.runner).expect("the second enumeration");
    let surface = after
        .iter()
        .find(|s| s.id == cap.id)
        .expect("the pinned surface is still there");
    assert_eq!(
        surface.tab_name,
        ghostty::pin_label(&nonce),
        "pin survived a hostile OSC 2: tab.name is the override"
    );
    println!(
        "ok    pin survived a hostile OSC 2 (tab.name={})",
        surface.tab_name
    );
    assert_eq!(
        surface.terminal_name, "HOSTILE_RETITLE",
        "terminal.name followed the OSC 2"
    );
    println!("ok    terminal.name followed the OSC 2 — the two fields are distinct");

    // …and the resolver finds it the way §5.5 step 5 says to: on `tab.name`,
    // with the id confirmed, and with the beacon long gone.
    //
    // The map entry is what makes this the real scenario rather than the
    // corpse one. A daemon that pinned this tab remembers which surface it
    // pinned; a lone `deck:<id4>` with nothing on file is what a `kill -9`
    // leaves behind for the *next* run to find, and the resolver refuses to
    // adopt that on purpose — `id4` is a function of the agent sequence
    // alone, so every run's first agent wears the same label.
    let mut resolver = Resolver::new();
    resolver.restore(&std::collections::BTreeMap::from([(
        cap.id.clone(),
        serde_json::json!({
            "key": "claude:hw",
            "id4": nonce,
            "agent": "ag_hwtest",
            "tty": cap.tty,
            "pid": cap.pid,
            "manual": false,
        }),
    )]));
    let resolution = resolver
        .resolve(
            &ghost.runner,
            &deckd::desktop::resolve::Session {
                agent: "ag_hwtest",
                key: "claude:hw",
                nonce: Some(&nonce),
                id4: &nonce,
                tty: Some(&cap.tty),
                pid: Some(cap.pid),
            },
            Instant::now(),
        )
        .expect("re-resolution on tab.name");
    assert_eq!(resolution.terminal, cap.id);
    println!("ok    tab.name re-resolved the pinned session after the beacon was overwritten");

    // ── row 5: unpin ────────────────────────────────────────────────────
    resolver.release(&ghost.runner, "claude:hw").expect("unpin");
    std::thread::sleep(Duration::from_millis(500));
    let restored = ghostty::enumerate(&ghost.runner).expect("the third enumeration");
    let surface = restored
        .iter()
        .find(|s| s.id == cap.id)
        .expect("still there after the unpin");
    assert_eq!(
        surface.tab_name, "HOSTILE_RETITLE",
        "unpin restored the live title"
    );
    println!("ok    unpin restored the live title ({})", surface.tab_name);
    assert!(
        resolver.pins().is_empty(),
        "§6: every tab this build pins is unpinned"
    );
}

/// Row 3: text aimed at an unfocused surface lands there and **nowhere
/// else** (M4).
///
/// The payload is an inert marker into a `cat`, and the surface that takes
/// the focus is another `cat` — so even a total failure of aiming could not
/// run anything. `send key "enter"` is what flushes the line into the
/// capture file, which is also this row's proof that [`ghostty::ENTER`] is
/// the spelling that works: M3's `"return"` is asserted to still be
/// `-1700` at the end.
#[test]
#[ignore = "hardware: types an inert marker into a Ghostty tab"]
fn hw_aimed_input_reaches_the_target_tab_and_not_the_focused_one() {
    let Some(mut ghost) = Ghost::start(scratch("aimed")) else {
        return;
    };
    let Some(aimed) = ghost.surface("aimed") else {
        skip("the aimed surface never reported its tty");
        return;
    };
    let Some(stealer) = ghost.surface("stealer") else {
        skip("a second surface could not be created; the aiming row needs two");
        return;
    };
    let selected: Vec<String> = ghostty::enumerate(&ghost.runner)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.selected && s.id != aimed.id)
        .map(|s| s.id)
        .collect();
    println!(
        "note  aiming at {} while {selected:?} is/are selected",
        aimed.id
    );

    ghostty::input_text(&ghost.runner, &aimed.id, "AIMED_PAYLOAD").expect("input text");
    ghostty::send_key(&ghost.runner, &aimed.id, ghostty::ENTER).expect("send key enter");
    assert!(
        wait_until_contains(&aimed.got, "AIMED_PAYLOAD", Duration::from_secs(5)),
        "aimed: the payload never arrived in the surface it was aimed at"
    );
    println!("ok    aimed input reached the target tab while another was selected");
    assert!(
        !std::fs::read_to_string(&stealer.got).is_ok_and(|t| t.contains("AIMED_PAYLOAD")),
        "the payload leaked into the other surface"
    );
    println!("ok    aimed input did not leak into the other surface");

    // M3, live: the word the design used does not exist.
    let err =
        ghostty::send_key(&ghost.runner, &aimed.id, "return").expect_err("`return` is no key name");
    assert_eq!(
        err.number(),
        Some(deckd::desktop::errors::ERR_COERCION),
        "send key \"return\" must still be -1700: {err}"
    );
    println!("note  send key \"return\" is still -1700 ({err})");
}

/// Row 4: **guard 3** against two live ttys — the four cases `fgcheck.c`
/// was proven on, through the real `proc_pidinfo` mechanism.
///
/// This is §15's "injection refused after the CLI exits to a shell" row.
#[test]
#[ignore = "hardware: opens two Ghostty surfaces to get two live ttys"]
fn hw_guard_three_allows_only_the_foreground_pid_of_the_recorded_tty() {
    let Some(mut ghost) = Ghost::start(scratch("guard3")) else {
        return;
    };
    let Some(first) = ghost.surface("fg") else {
        skip("the first surface never reported its tty");
        return;
    };
    let Some(second) = ghost.surface("other") else {
        skip("the second surface never reported its tty");
        return;
    };
    println!(
        "note  ttys: {} (pid {}) and {} (pid {})",
        first.tty, first.pid, second.tty, second.pid
    );

    // (a) the process that holds the foreground of its own tty → allow.
    let verdict = fg::check(first.pid, &first.tty);
    assert_eq!(
        verdict,
        Verdict::Foreground,
        "guard 3 refused the foreground process of {}: {verdict}",
        first.tty
    );
    println!("ok    guard 3 allows the pid that holds the foreground pgrp of its tty");

    // (b) a live pid that is **not** the foreground pgrp of the tty it is
    // aimed at. This is the "exited to a shell" case: something else owns
    // the prompt the order would land on.
    let verdict = fg::check(second.pid, &first.tty);
    assert!(
        !verdict.allows(),
        "guard 3 ALLOWED a live pid on another terminal ({verdict}) — injection would land on \
         somebody else's prompt"
    );
    assert_eq!(verdict, Verdict::OtherTty, "and for the M10 reason");
    println!("ok    guard 3 refuses a live pid that is not the foreground of the recorded tty");

    // (c) a dead pid.
    let verdict = fg::check(999_999, &first.tty);
    assert_eq!(verdict, Verdict::Dead, "guard 3 allowed a dead cli_pid");
    println!("ok    guard 3 refuses a dead cli_pid");

    // (d) the right pid aimed at the wrong tty — M10's addition, and the
    // case a pgid-only check gets wrong.
    let verdict = fg::check(first.pid, &second.tty);
    assert!(
        !verdict.allows(),
        "guard 3 allowed a foreground pid aimed at a different terminal: {verdict}"
    );
    println!("ok    guard 3 refuses a foreground pid aimed at another terminal");
}

/// The whole chain, live: resolve → pin → guard 3 → `input text` →
/// guard 3 → `send key "enter"`, through [`Desktop::on_job`] rather than
/// through the pieces — plus the focus-out and the unpin.
///
/// It is the row that proves the *module* works and not only its parts, and
/// it carries §15's "device `channel` focuses the right tab": the selection
/// is read back from `id of (selected tab of <window>)`.
#[test]
#[ignore = "hardware: runs the real injector chain into a Ghostty tab"]
fn hw_the_injector_chain_and_the_focus_out_end_to_end() {
    let Some(mut ghost) = Ghost::start(scratch("chain")) else {
        return;
    };
    let Some(cap) = ghost.surface("chain") else {
        skip("the control surface never reported its tty");
        return;
    };
    let nonce = format!("c{:03x}", std::process::id() & 0xfff);
    cap.osc2(&ghostty::beacon_name(&nonce));

    let mut desktop = Desktop::with(
        Arc::new(OsaRunner::new()),
        Arc::new(fg::Procs),
        DesktopConfig {
            inject: vec!["ghostty".into()],
            focus: true,
            // No roster probe: guard 2's roster half is the operator's own
            // `claude`, and this row is about the desktop side.
            claude_bin: None,
            in_process: false,
            prompts: false,
        },
    );
    let target = Target {
        agent: "ag_hwch".into(),
        key: "claude:hw-chain".into(),
        nonce: Some(nonce.clone()),
        id4: nonce.clone(),
        tty: cap.tty.clone(),
        pid: cap.pid,
        program: "ghostty".into(),
    };
    let agent = deck_model::AgentId("ag_hwch".into());

    let out = desktop.on_job(
        DesktopJob::Inject {
            agent: agent.clone(),
            target: target.clone(),
            text: "INERT_CHAIN_MARKER".into(),
            dialog_open: false,
        },
        Instant::now(),
    );
    let order = out
        .iter()
        .find_map(|e| match e {
            FromDesktop::Order { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("the chain reported an outcome");
    if let Err(e) = &order {
        if e.reason() == "automation_denied" {
            skip(&format!("Automation is denied (-1743): {e}"));
            return;
        }
    }
    order.expect("the injector chain");
    assert!(
        wait_until_contains(&cap.got, "INERT_CHAIN_MARKER", Duration::from_secs(5)),
        "the chain's text never reached the resolved terminal"
    );
    println!("ok    the injector chain resolved, pinned, guarded and typed into the aimed tab");
    let (terminal, pinned) = {
        let (terminal, pinned) = desktop
            .resolver()
            .known("claude:hw-chain")
            .expect("a resolution");
        (terminal.to_string(), pinned.clone())
    };
    assert_eq!(terminal, cap.id);
    println!(
        "note  pinned {terminal} as {}",
        ghostty::pin_label(&pinned.id4)
    );
    assert_eq!(pinned.tty, Some(cap.tty.clone()));
    assert_eq!(pinned.pid, Some(cap.pid));

    // The CLI's own title write, so the live title underneath the override
    // is something the restored `tab.name` can be told apart from: the
    // beacon *is* a `deck:` string, and a row that unpinned back onto it
    // could not distinguish the override from the title beneath it.
    cap.osc2("CHAIN_LIVE_TITLE");

    // Focus out: `activate` + `goto_tab`, verified on `selected tab`.
    let out = desktop.on_job(
        DesktopJob::Focus {
            agent: agent.clone(),
            target,
        },
        Instant::now(),
    );
    match out.first() {
        Some(FromDesktop::Focus {
            selected, reason, ..
        }) => {
            if *selected {
                println!("ok    op.focus selected the resolved tab");
            } else {
                // Not a failure: Ghostty may be on another Space, and §5.5
                // makes a focus failure silent on the device and visible in
                // the GUI. The row says what happened rather than pretending.
                println!("note  op.focus did not land the selection (reason {reason:?})");
            }
        }
        other => panic!("expected a focus outcome, got {other:?}"),
    }

    // §5.5 step 7 and §6: the session ends and the title goes back. The job
    // names the `ag_…` id, because that is all `agent.removed` carries.
    desktop.on_job(
        DesktopJob::Release {
            key: "ag_hwch".into(),
        },
        Instant::now(),
    );
    std::thread::sleep(Duration::from_millis(500));
    let restored = ghostty::enumerate(&ghost.runner).expect("a final enumeration");
    let surface = restored
        .iter()
        .find(|s| s.id == terminal)
        .expect("the surface is still there");
    assert_eq!(
        surface.tab_name, "CHAIN_LIVE_TITLE",
        "unpin left {} on the tab",
        surface.tab_name
    );
    println!(
        "ok    unpin left the tab title as it was found ({})",
        surface.tab_name
    );
    assert!(desktop.resolver().pins().is_empty());
}
