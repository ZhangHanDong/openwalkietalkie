//! Contract §5 row 3: the §14 virtual agent — `host/examples/
//! virtual_agent.py`, stdlib only, over `POST /v1/rpc` — registers with a
//! `client.name`, sets a `q` card, parks in `agent.wait`, and exits 0 with
//! the answer the operator gave from a UDS `operate` session.
mod common;

use common::{Daemon, PATIENCE};
use deck_model::Scope;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// `host/examples/virtual_agent.py` from this crate's manifest dir.
fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/virtual_agent.py")
        .canonicalize()
        .expect("the virtual agent script is in the tree")
}

/// `python3` on PATH, or `None` (the row prints its reason and skips).
fn python3() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("python3"))
        .find(|p| p.is_file())
}

/// The child is killed when the test leaves, whatever happened (tokio's
/// `process` feature is not in the workspace pins, so the script runs on
/// `std::process` and its stdout is pumped by a blocking thread).
struct Script {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
}

impl Script {
    /// The next line the script printed, or the test fails.
    async fn line(&mut self) -> serde_json::Value {
        let line = tokio::time::timeout(PATIENCE, self.lines.recv())
            .await
            .expect("a line from the script in time")
            .expect("the script printed a line");
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("not json: {line:?}"))
    }

    /// The exit status, waited for off the runtime.
    async fn wait(mut self) -> i32 {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.child.try_wait().expect("the script is waitable") {
                Some(status) => return status.code().unwrap_or(-1),
                None => assert!(Instant::now() < deadline, "the script did not exit"),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The agent the script registered, once it is in the snapshot with a card.
async fn agent_with_a_card(daemon: &Daemon, name: &str) -> (deck_model::AgentId, u16) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let snapshot = daemon.api.bus.snapshot();
        let found = snapshot
            .agents
            .values()
            .find(|a| a.name == name)
            .and_then(|a| a.card.as_ref().map(|card| (a.id.clone(), card.id.0)));
        if let Some(found) = found {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "the script never registered {name} with a card"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `agent.wait` resolves from the owning session's mailbox (`w2-core`,
/// merged): the script registers over TCP, sets a card, parks in
/// `agent.wait` and is answered from an `operate` UDS session.
#[tokio::test]
async fn python_agent_end_to_end_over_tcp() {
    let Some(python) = python3() else {
        println!("skip api_virtual_agent: python3 is not on PATH");
        return;
    };
    let daemon = Daemon::start().await;
    // The script's own token: a virtual agent needs `agent`, nothing more.
    let token = daemon.token("gatebot", &[Scope::Observe, Scope::Agent]);

    let mut child = std::process::Command::new(python)
        .arg(script())
        .args(["--port", &daemon.port.to_string()])
        .args(["--token", &token])
        .args(["--name", "gatebot"])
        .args(["--text", "Deploy to staging?"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("python3 runs the virtual agent");
    let stdout = child.stdout.take().expect("a piped stdout");
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut script = Script { child, lines: rx };

    // Line 1: the script says which agent it registered and that it waits.
    let first = script.line().await;
    assert_eq!(first["waiting"], true, "{first}");

    // The card it set is a `custom` `q` card delivered by `poll`.
    let (agent, card) = agent_with_a_card(&daemon, "gatebot").await;
    assert_eq!(first["agent"], agent.0);
    {
        let snapshot = daemon.api.bus.snapshot();
        let set = snapshot.agents[&agent].card.clone().expect("the card");
        assert_eq!(set.text.as_deref(), Some("Deploy to staging?"));
        assert_eq!(set.deliver, deck_model::Deliver::Poll);
        assert_eq!(set.origin.kind, deck_model::OriginKind::Custom);
        assert_eq!(set.choices.len(), 2);
    }

    // The operator answers from a UDS session with `operate` — a custom
    // card, so the trust guard never applies.
    let mut operator = daemon.connect().await;
    operator
        .call(
            "session.hello",
            serde_json::json!({"client": {"name": "op"}, "scopes": ["observe", "operate"]}),
        )
        .await;
    let answered = operator
        .call(
            "op.answer",
            serde_json::json!({"agent": agent.0, "card": card, "choice": 0}),
        )
        .await;
    // `op.answer` answers after the delivery (design §4.9): the script's
    // `agent.wait` was parked, so the answer reached it and the door
    // reported the flush before this reply was written.
    assert_eq!(answered["delivered"], true, "{answered}");
    assert_eq!(answered["via"], "poll");
    assert!(answered["reason"].is_null(), "{answered}");

    // Line 2: the answer the script's `agent.wait` returned.
    let answer = script.line().await;
    assert_eq!(answer["kind"], "answer", "{answer}");
    assert_eq!(answer["choice"], 0);
    assert_eq!(answer["label"], "Yes");

    // The daemon clears a `poll` card when the wait result is written.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let snapshot = daemon.api.bus.snapshot();
        let cleared = snapshot.agents.get(&agent).is_none_or(|a| a.card.is_none());
        if cleared {
            break;
        }
        assert!(Instant::now() < deadline, "the card was not cleared in 1 s");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(script.wait().await, 0, "the script exits 0");
    daemon.shutdown().await;
}
