//! `deckd events` (§3.8): one JSON line per event on stdout until the reader
//! goes away, `--kinds` filtering what is subscribed, `--snapshot` printing
//! the hello's snapshot first, and exit 2 when the daemon is unreachable.
//!
//! Nothing tested this: the command was the last §9 B4 row and went from
//! "unbuilt (exit 2)" to built without a behavioural test, so a subscription
//! or a serialization regression in it was invisible to the gate.
//!
//! `events_cmd` builds its own tokio runtime and runs until SIGTERM or a
//! failing write (its `| head` escape), so it is driven here from a plain
//! thread with a writer that closes after a fixed number of lines.
mod common;

use common::{card_params, register_params, Daemon, PATIENCE};
use deck_model::Scope;
use deckd::config::Paths;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One JSON-RPC request body.
fn rpc(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string()
}

/// Stdout behind a `| head`: it keeps what is written and then refuses, and
/// `events_cmd` treats the refusal as "the reader is gone" and returns.
#[derive(Clone)]
struct Head {
    out: Arc<Mutex<String>>,
    lines: usize,
}

impl Head {
    fn new(lines: usize) -> Head {
        Head {
            out: Arc::new(Mutex::new(String::new())),
            lines,
        }
    }

    /// The complete lines written so far.
    fn taken(&self) -> Vec<String> {
        let out = self.out.lock().unwrap();
        let Some(end) = out.rfind('\n') else {
            return Vec::new();
        };
        // JSON serialization can write a line in several chunks. A partial
        // snapshot is not a signal that the reader is ready for events.
        out[..=end].lines().map(str::to_string).collect()
    }
}

impl Write for Head {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = self.out.lock().unwrap();
        if out.matches('\n').count() >= self.lines {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdout closed"));
        }
        out.push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_snapshot_is_ready_only_after_its_newline() {
    let mut head = Head::new(2);
    head.write_all(b"{\"type\":").unwrap();
    assert!(head.taken().is_empty());
    head.write_all(b"\"snapshot\"}\n{\"type\":").unwrap();
    assert_eq!(head.taken(), [r#"{"type":"snapshot"}"#]);
    head.write_all(b"\"agent.updated\"}\n").unwrap();
    assert_eq!(
        head.taken(),
        [r#"{"type":"snapshot"}"#, r#"{"type":"agent.updated"}"#]
    );
}

fn paths_of(daemon: &Daemon) -> Paths {
    Paths::from_data_dir(
        daemon.dir.clone(),
        daemon.dir.join("logs"),
        daemon.dir.join("caches"),
    )
}

/// The whole command: it connects over the UDS, prints the snapshot, then
/// one line per event, and ends (exit 0) when its reader closes — which is
/// what `deckd events | head` does.
///
/// Two readers run side by side over the same traffic, one subscribed to
/// `agent.*` and one to `card.*`, because a single reader proves nothing:
/// the first events any registration raises are `agent.*` whatever was
/// subscribed, so a `deckd events` that ignored `--kinds` would still have
/// satisfied an `agent.*`-only assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deckd_events_prints_a_line_per_event_and_filters_by_kind() {
    let daemon = Daemon::start().await;
    // Each registration raises several `agent.*` events and each card
    // exactly one `card.set`, so both readers fill from the same traffic.
    let agents = Head::new(7);
    let cards = Head::new(5);
    let mut readers = Vec::new();
    for (kind, head) in [("agent.*", &agents), ("card.*", &cards)] {
        let paths = paths_of(&daemon);
        let mut out = head.clone();
        readers.push((
            kind,
            head.clone(),
            std::thread::spawn(move || {
                deckd::cli::events_cmd(&paths, vec![kind.into()], None, false, true, &mut out)
            }),
        ));
    }

    // Both are subscribed once their snapshot line is out; only then may the
    // traffic start, or the `card.*` reader could miss all of it.
    let deadline = Instant::now() + PATIENCE;
    while agents.taken().is_empty() || cards.taken().is_empty() {
        assert!(
            Instant::now() < deadline,
            "a reader never printed its snapshot line"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    for (kind, head, _) in &readers {
        let first: serde_json::Value = serde_json::from_str(&head.taken()[0]).unwrap();
        assert_eq!(
            first["type"], "snapshot",
            "--snapshot prints it first ({kind})"
        );
        assert!(first["snapshot"].is_object(), "{first}");
    }

    let bearer = daemon.bearer("ev", &[Scope::Agent, Scope::Observe]);
    for n in 0..8 {
        let registered = daemon
            .rpc(
                &bearer,
                &rpc(
                    "agent.register",
                    register_params(&format!("evbot{n}"), &format!("http:ev{n}")),
                ),
            )
            .await;
        let agent = registered["result"]["agent"]
            .as_str()
            .unwrap_or_else(|| panic!("{registered}"))
            .to_string();
        let card = daemon
            .rpc(
                &bearer,
                &rpc("agent.card.set", card_params(&agent, "ship it?")),
            )
            .await;
        assert!(card["result"].is_object(), "{card}");
    }

    for (kind, head, join) in readers {
        let code = tokio::task::spawn_blocking(move || join.join())
            .await
            .expect("the events thread is joinable")
            .expect("the events thread did not panic")
            .expect("events_cmd returned an exit code");
        assert_eq!(code, 0, "a closed reader is an orderly end, not an error");
        let lines = head.taken();
        let want = if kind == "agent.*" { 7 } else { 5 };
        assert_eq!(lines.len(), want, "`| head -{want}` on {kind}: {lines:?}");
        let prefix = kind.trim_end_matches('*');
        for line in &lines[1..] {
            let event: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("`deckd events` must print JSON lines, got {line:?} ({e})")
            });
            let got = event["type"].as_str().unwrap_or_else(|| panic!("{event}"));
            assert!(got.starts_with(prefix), "--kinds {kind} let {got} through");
            assert!(
                event["rev"].is_u64(),
                "every event carries its rev: {event}"
            );
            assert!(event["ts"].is_string(), "and its timestamp: {event}");
        }
    }
    daemon.shutdown().await;
}

/// No daemon behind the socket: the contract's exit 2 (unreachable), not a
/// hang and not a panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deckd_events_exits_2_when_the_daemon_is_unreachable() {
    let dir = std::env::temp_dir().join(format!("dkev{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp data dir");
    let paths = Paths::from_data_dir(dir.clone(), dir.join("logs"), dir.join("caches"));
    let mut out = Head::new(8);
    let code = tokio::task::spawn_blocking(move || {
        deckd::cli::events_cmd(&paths, Vec::new(), None, false, false, &mut out)
    })
    .await
    .expect("the blocking task")
    .expect("events_cmd returned an exit code");
    assert_eq!(code, 2, "nothing is listening on the socket");
    let _ = std::fs::remove_dir_all(&dir);
}
