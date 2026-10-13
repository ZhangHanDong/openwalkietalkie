//! Exit test 17 of the week-3 contract (§6): the liveness pollers, B1 and
//! C9, against a live daemon.
//!
//! Nothing here runs the operator's own `claude`. The roster probe is
//! pointed at a fake binary in a temp directory through the `PATH` set on
//! the command itself, so the daemon's environment is untouched and the
//! fake records exactly what it was asked.
mod common;

use common::Daemon;
use deck_model::{AgentId, AgentState, PidKind};
use deckd::liveness::{self, Misses, Probe};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A pid that certainly belongs to nothing: a process spawned and reaped
/// here, so the number was real and is now free.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exit 0")
        .spawn()
        .expect("a short-lived child");
    let pid = child.id();
    child.wait().expect("reaped");
    assert!(!liveness::alive(pid), "pid {pid} was recycled");
    pid
}

/// A fake `claude` on a `PATH` of its own: it appends its argv to `calls`
/// and prints `body` (or fails with `rc`).
fn fake_claude(dir: &Path, body: &str, rc: i32) -> PathBuf {
    std::fs::create_dir_all(dir).expect("the fake's directory");
    let bin = dir.join("claude");
    let calls = dir.join("calls");
    let mut file = std::fs::File::create(&bin).expect("the fake");
    // The probe hands the child a `PATH` of this directory alone — that
    // is how the fake is found instead of the operator's own `claude` —
    // so the script uses shell builtins and absolute paths only.
    write!(
        file,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{calls}'\nprintf '%s\\n' '{body}'\nexit {rc}\n",
        calls = calls.display(),
    )
    .expect("the fake's body");
    drop(file);
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("executable");
    bin
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// `kill(pid, 0)` runs for an agent whose `pid_kind` is `cli` and for no
/// other, and the roster poll reconciles off `status` without ever
/// touching the pid of an agent it does not govern (B1, C9).
#[tokio::test]
async fn only_cli_pids_are_polled() {
    let daemon = Daemon::start().await;
    let gone = dead_pid();
    // Four agents, one pid_kind each, all four carrying the same dead pid.
    let (cli, _) = daemon
        .adapter_agent_pid("cli", Some(gone), Some(PidKind::Cli))
        .await;
    let (shell, _) = daemon
        .adapter_agent_pid("shell", Some(gone), Some(PidKind::Shell))
        .await;
    let (unknown, _) = daemon
        .adapter_agent_pid("unknown", Some(gone), Some(PidKind::Unknown))
        .await;
    let (unset, _) = daemon.adapter_agent_pid("unset", Some(gone), None).await;
    let ids: Vec<AgentId> = [&cli, &shell, &unknown, &unset]
        .iter()
        .map(|s| AgentId((*s).clone()))
        .collect();
    daemon
        .until("the four agents are on the roster", |s| {
            ids.iter().all(|id| s.agents.contains_key(id))
        })
        .await;

    // The sweep: exactly one agent is probed, and the pid it probed is the
    // one the helper resolved as a CLI.
    let asked = std::sync::Mutex::new(Vec::new());
    let (acts, tick) = liveness::sweep(&daemon.api.bus.snapshot(), |pid| {
        asked.lock().unwrap().push(pid);
        liveness::alive(pid)
    });
    assert_eq!(
        tick.polled,
        vec![AgentId(cli.clone())],
        "only pid_kind=cli is polled: {tick:?}"
    );
    assert_eq!(
        *asked.lock().unwrap(),
        vec![gone],
        "one kill(pid,0), for the cli pid"
    );
    assert_eq!(tick.released, vec![AgentId(cli.clone())]);
    liveness::apply(&daemon.api, acts).await;
    daemon
        .until("the dead cli agent is released", |s| {
            !s.agents.contains_key(&AgentId(cli.clone()))
        })
        .await;
    let left = daemon.api.bus.snapshot();
    for id in [&shell, &unknown, &unset] {
        assert!(
            left.agents.contains_key(&AgentId(id.clone())),
            "a non-cli pid_kind is never evicted by the pid poll: {id}"
        );
    }

    // The roster poll, against a fake `claude` on a PATH of its own. It
    // lists the `shell` session as busy and nothing else.
    let dir = daemon.dir.join("fake-cli");
    fake_claude(
        &dir,
        r#"[{"pid":4242,"cwd":"/tmp/p","kind":"interactive","startedAt":1789629157246,"sessionId":"shell","name":"openwalkietalkie-92","status":"busy"}]"#,
        0,
    );
    let probe = Probe::on_path("claude", &dir);
    let mut misses = Misses::new();
    let tick = liveness::roster_tick(&daemon.api, &probe, &mut misses, false).await;
    assert!(!tick.inconclusive, "{tick:?}");
    assert_eq!(tick.seen, 1);
    assert_eq!(tick.updated, vec![AgentId(shell.clone())], "{tick:?}");
    assert!(tick.released.is_empty(), "one strike is not two: {tick:?}");
    assert_eq!(calls(&dir), vec!["agents --json".to_string()]);
    daemon
        .until("the roster's `busy` reached the deck", |s| {
            s.agents
                .get(&AgentId(shell.clone()))
                .is_some_and(|a| a.state == AgentState::Working && a.name == "openwalkietalkie-92")
        })
        .await;

    // An inconclusive probe between the two strikes costs nobody a strike.
    let bad = daemon.dir.join("fake-broken");
    fake_claude(&bad, "not json at all", 1);
    let broken = liveness::roster_tick(
        &daemon.api,
        &Probe::on_path("claude", &bad),
        &mut misses,
        false,
    )
    .await;
    assert!(broken.inconclusive, "{broken:?}");
    assert!(broken.released.is_empty());

    // The second conclusive miss releases the two sessions the CLI does
    // not have — and leaves the one it does.
    let tick = liveness::roster_tick(&daemon.api, &probe, &mut misses, false).await;
    let mut released = tick.released.clone();
    released.sort();
    let mut expected = vec![AgentId(unknown.clone()), AgentId(unset.clone())];
    expected.sort();
    assert_eq!(released, expected, "{tick:?}");
    daemon
        .until("the two absent sessions are off the roster", |s| {
            !s.agents.contains_key(&AgentId(unknown.clone()))
                && !s.agents.contains_key(&AgentId(unset.clone()))
        })
        .await;
    let end = daemon.api.bus.snapshot();
    assert!(
        end.agents.contains_key(&AgentId(shell.clone())),
        "the session the CLI still lists stays"
    );
    assert_eq!(end.agents.len(), 1);
    assert!(misses.is_empty(), "a released agent keeps no strike");
    assert_eq!(
        calls(&dir).len(),
        2,
        "the fake answered both conclusive ticks"
    );
    daemon.shutdown().await;
}
