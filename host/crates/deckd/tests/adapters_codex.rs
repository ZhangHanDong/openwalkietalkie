//! The Codex adapter end to end (contract §6 rows 7–8): recorded hook
//! payloads in, cards and roster entries out, and the exact JSON Codex
//! reads back on stdout.
//!
//! Every payload is replayed from `tests/fixtures/hooks/codex/*.json`,
//! captured from `codex-cli 0.153.4` and **scrubbed** before it was
//! committed — the scrubbing is asserted here, not assumed (contract §9:
//! hook payloads are the most sensitive data on the box).
//!
//! The daemon is in-process with a temp data dir under `$TMPDIR`; nothing
//! here reads or writes the user's `~/.codex`, and no real `codex` is run.
mod common;

use common::{http, Daemon, Options};
use deck_model::{Deliver, OriginKind, Scope};
use std::path::PathBuf;

/// The CLI session id every fixture carries, and the roster key it makes.
const SESSION: &str = "01a0bca9-1344-7d20-a793-10903639be67";
const KEY: &str = "codex:01a0bca9-1344-7d20-a793-10903639be67";
const TURN: &str = "01a0bcad-e001-7042-8668-78526755dd61";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hooks/codex")
}

/// One recorded payload.
fn fixture(name: &str) -> serde_json::Value {
    let path = fixtures().join(format!("{name}.json"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} : {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

/// A `hooks.<method>` JSON-RPC body carrying one payload, shaped exactly
/// as `deckd hook` sends it.
fn call(method: &str, payload: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": method,
        "params": {
            "payload": payload,
            "env": {"TERM_PROGRAM": "ghostty", "TMUX_PANE": "%3"},
            "tty": "/dev/ttys004",
            "cli_pid": 4242,
            "pid_kind": "cli"
        }
    })
    .to_string()
}

/// The agent id and card of the one Codex session on the roster.
fn one_agent(daemon: &Daemon) -> (String, Option<deck_model::Card>) {
    let snapshot = daemon.api.bus.snapshot();
    let agent = snapshot
        .agents
        .values()
        .find(|a| a.session.key.as_deref() == Some(KEY))
        .unwrap_or_else(|| panic!("the codex session is on the roster"));
    (agent.id.0.clone(), agent.card.clone())
}

async fn started(daemon: &Daemon, auth: &str) {
    let out = daemon
        .rpc(auth, &call("hooks.codex", fixture("session-start")))
        .await;
    assert_eq!(out["result"], serde_json::json!({}), "{out}");
    daemon
        .until("the codex session is on the roster", |s| {
            s.agents
                .values()
                .any(|a| a.session.key.as_deref() == Some(KEY))
        })
        .await;
}

fn untrusted() -> Options {
    Options {
        verifier: deckd::trust::FakeVerifier {
            trusted: false,
            capped: false,
        },
        ..Options::default()
    }
}

/// Contract §6 row 7. A `PermissionRequest` parks a held card with
/// `Allow`/`Deny` and nothing else — the hook protocol has no
/// session-scoped approval (facts §4.2) — and the operator's `Deny` comes
/// back as the exact object Codex parses, **with a message**: a denial
/// with no reason is one the model retries verbatim.
#[tokio::test]
async fn a_permission_request_becomes_a_held_card_and_deny_carries_its_message() {
    let daemon = Daemon::with(untrusted()).await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);
    let operate = daemon.bearer("op", &[Scope::Observe, Scope::Operate]);
    started(&daemon, &hooks).await;

    // B1: the helper's walk reaches the roster, so liveness knows which
    // pids it may poll.
    let snapshot = daemon.api.bus.snapshot();
    let agent = snapshot
        .agents
        .values()
        .find(|a| a.session.key.as_deref() == Some(KEY))
        .expect("the agent");
    assert_eq!(agent.kind, deck_model::Kind::Codex);
    assert_eq!(agent.name, "cohdl-api", "the cwd basename");
    assert_eq!(agent.session.pid, Some(4242));
    assert_eq!(
        agent.session.pid_kind,
        Some(deck_model::PidKind::Cli),
        "only `cli` pids are polled, so the walk's verdict must arrive"
    );
    assert_eq!(
        agent
            .session
            .terminal
            .as_ref()
            .and_then(|t| t.tty.as_deref()),
        Some("/dev/ttys004")
    );
    drop(snapshot);

    // The hook parks: its HTTP response does not come back until the card
    // is answered, so it runs on its own task.
    let port = daemon.port;
    let hooks_header = hooks.clone();
    let body = call("hooks.codex", fixture("permission-request"));
    let parked = tokio::spawn(async move {
        http(
            port,
            "POST",
            "/v1/rpc",
            &[("Authorization", &hooks_header)],
            &body,
        )
        .await
    });

    daemon
        .until("the permission card is on the deck", |s| {
            s.agents
                .values()
                .any(|a| a.card.as_ref().is_some_and(|c| c.deliver == Deliver::Hold))
        })
        .await;
    let (agent, card) = one_agent(&daemon);
    let card = card.expect("a card");
    assert_eq!(card.origin.kind, OriginKind::Permission);
    assert_eq!(card.origin.tool.as_deref(), Some("Bash"));
    assert_eq!(card.origin.request_id.as_deref(), Some(TURN));
    assert_eq!(
        card.choices
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Allow", "Deny"]
    );
    assert_eq!(card.rec, Some(0));
    assert!(
        card.text
            .as_deref()
            .is_some_and(|t| t.starts_with("Bash: ")),
        "{:?}",
        card.text
    );

    // Deny is not a guarded operation (only an allow is), so an untrusted
    // token may choose it — and `op.answer` does not answer `delivered`
    // until the adapter has written the hook's response.
    let denied = daemon
        .rpc(
            &operate,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "op.answer",
                "params": {"agent": agent, "card": card.id, "choice": 1}
            })
            .to_string(),
        )
        .await;
    assert!(denied["error"].is_null(), "{denied}");
    assert_eq!(denied["result"]["delivered"], true, "{denied}");
    assert_eq!(denied["result"]["via"], "hold", "{denied}");

    let (status, answer) = tokio::time::timeout(common::PATIENCE, parked)
        .await
        .expect("the held hook returns")
        .expect("the hook task lives");
    assert_eq!(status, 200);
    let decision = &answer["result"]["hookSpecificOutput"];
    assert_eq!(decision["hookEventName"], "PermissionRequest", "{answer}");
    assert_eq!(decision["decision"]["behavior"], "deny", "{answer}");
    assert_eq!(
        decision["decision"]["message"], "Denied from the deck",
        "a deny always says why: {answer}"
    );

    // The card is gone once the decision is written (`hold_written`).
    daemon
        .until("the card is cleared", |s| {
            s.agents.values().all(|a| a.card.is_none())
        })
        .await;
    daemon.shutdown().await;
}

/// The other half of row 7: `Allow` is the recommended row and writes the
/// allow shape. It needs a trusted principal (the week-2 guard), so the
/// verifier trusts this peer — which is what the deck itself is.
#[tokio::test]
async fn an_allow_from_a_trusted_principal_writes_the_allow_shape() {
    let daemon = Daemon::with(Options {
        verifier: deckd::trust::FakeVerifier {
            trusted: true,
            capped: false,
        },
        ..Options::default()
    })
    .await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);
    started(&daemon, &hooks).await;

    let port = daemon.port;
    let header = hooks.clone();
    let body = call("hooks.codex", fixture("permission-request"));
    let parked = tokio::spawn(async move {
        http(
            port,
            "POST",
            "/v1/rpc",
            &[("Authorization", &header)],
            &body,
        )
        .await
    });
    daemon
        .until("the permission card is on the deck", |s| {
            s.agents
                .values()
                .any(|a| a.card.as_ref().is_some_and(|c| c.deliver == Deliver::Hold))
        })
        .await;
    let (agent, card) = one_agent(&daemon);
    let card = card.expect("a card");

    // A trusted UDS peer answers Allow.
    let mut peer = daemon.connect().await;
    peer.call(
        "session.hello",
        serde_json::json!({"client": {"name": "gui"}, "scopes": ["observe", "operate"]}),
    )
    .await;
    let allowed = peer
        .call(
            "op.answer",
            serde_json::json!({"agent": agent, "card": card.id, "choice": 0}),
        )
        .await;
    assert_eq!(allowed["delivered"], true, "{allowed}");

    let (status, answer) = tokio::time::timeout(common::PATIENCE, parked)
        .await
        .expect("the held hook returns")
        .expect("the hook task lives");
    assert_eq!(status, 200);
    assert_eq!(
        answer["result"],
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "allow"}
            }
        }),
        "the exact two-field shape 0.153.4 parses: {answer}"
    );
    daemon.shutdown().await;
}

/// §5.6: a hold released without a decision answers `{}` — Codex then
/// runs its own approval flow — and the **same card id** stays on the
/// deck as `⌨ … / Focus terminal`, so the Allow/Deny rows left in the
/// drawer are replaced by a row that still does something.
///
/// The `⌨ ` belongs to the **wire encoder**, which writes one for every
/// degraded card. An adapter that re-pushed the text with a prefix of its
/// own put two on the line the deck received, so the card's stored text is
/// asserted un-prefixed here and the device line is asserted to carry
/// exactly one.
#[tokio::test]
async fn a_released_codex_hold_answers_empty_and_degrades_to_focus_terminal() {
    let daemon = Daemon::with(untrusted()).await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);
    let operate = daemon.bearer("op", &[Scope::Observe, Scope::Operate]);
    started(&daemon, &hooks).await;

    let port = daemon.port;
    let header = hooks.clone();
    let body = call("hooks.codex", fixture("permission-request"));
    let parked = tokio::spawn(async move {
        http(
            port,
            "POST",
            "/v1/rpc",
            &[("Authorization", &header)],
            &body,
        )
        .await
    });
    daemon
        .until("the permission card is on the deck", |s| {
            s.agents
                .values()
                .any(|a| a.card.as_ref().is_some_and(|c| c.deliver == Deliver::Hold))
        })
        .await;
    let (agent, card) = one_agent(&daemon);
    let card = card.expect("a card");

    let snoozed = daemon
        .rpc(
            &operate,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 3, "method": "op.snooze",
                "params": {"agent": agent, "card": card.id}
            })
            .to_string(),
        )
        .await;
    assert!(snoozed["error"].is_null(), "{snoozed}");

    let (status, answer) = tokio::time::timeout(common::PATIENCE, parked)
        .await
        .expect("the released hook returns")
        .expect("the hook task lives");
    assert_eq!(status, 200);
    assert_eq!(
        answer["result"],
        serde_json::json!({}),
        "a release decides nothing: {answer}"
    );

    daemon
        .until("the card degrades to focus", |s| {
            s.agents.values().any(|a| {
                a.card
                    .as_ref()
                    .is_some_and(|c| c.deliver == Deliver::Focus && c.id == card.id)
            })
        })
        .await;
    let (_, degraded) = one_agent(&daemon);
    let degraded = degraded.expect("the degraded card");
    assert_eq!(degraded.id, card.id, "the same id: no new chime");
    assert_eq!(
        degraded.status,
        deck_model::CardStatus::Degraded { focused_at: None }
    );
    assert_eq!(degraded.text, card.text, "the ask is still what it says");
    assert!(
        !degraded.text.as_deref().unwrap_or_default().contains('⌨'),
        "the prefix is the encoder's, never the card's: {:?}",
        degraded.text
    );

    // What the deck is actually sent: one `⌨ `, one row that works.
    let line = daemon
        .device_ops()
        .into_iter()
        .rev()
        .find_map(|op| match op {
            deck_model::DeviceOp::Put(
                deck_model::Key::Card(_, id),
                deck_model::HostLine::Card { text, choices, .. },
            ) if id == card.id.0 => Some((text, choices)),
            _ => None,
        })
        .expect("the degraded card reached the device");
    assert_eq!(
        line.0,
        format!("⌨ {}", card.text.as_deref().unwrap_or_default()),
        "exactly one prefix"
    );
    assert_eq!(line.1, vec!["Focus terminal".to_string()]);
    daemon.shutdown().await;
}

/// Contract §6 row 8. `Stop` and `notify agent-turn-complete` are the
/// same turn seen through two doors (C17: `notify` is a single command
/// and is already occupied on this machine, so the `Stop` hook is the
/// primary path and `notify` the redundancy). The second one to arrive is
/// a no-op, keyed on the **pair** `(session_id, turn_id)`.
#[tokio::test]
async fn the_notify_payload_is_deduped_against_the_stop_hook() {
    let daemon = Daemon::with(untrusted()).await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);
    started(&daemon, &hooks).await;

    // 1. The `Stop` hook reports the turn.
    let stopped = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("stop")))
        .await;
    assert_eq!(stopped["result"], serde_json::json!({}), "{stopped}");
    daemon
        .until("the turn is reported ready", |s| {
            s.agents.values().any(|a| {
                a.state == deck_model::AgentState::Ready
                    && a.reply.as_ref().and_then(|r| r.text.as_deref())
                        == Some("REDACTED assistant message")
            })
        })
        .await;

    // 2. A new prompt moves the session back to `working`, so a second
    //    report of the same turn would be visible as a state change.
    let prompted = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("user-prompt-submit")))
        .await;
    assert_eq!(prompted["result"], serde_json::json!({}), "{prompted}");
    daemon
        .until("the session is working again", |s| {
            s.agents
                .values()
                .any(|a| a.state == deck_model::AgentState::Working)
        })
        .await;

    // 3. `notify` for the *same* turn: answered, and nothing enqueued.
    let mut duplicate = fixture("notify-agent-turn-complete");
    duplicate["last-assistant-message"] = "the duplicate".into();
    let ignored = daemon
        .rpc(&hooks, &call("hooks.codex_notify", duplicate))
        .await;
    assert_eq!(ignored["result"], serde_json::json!({}), "{ignored}");

    // 4. `notify` for the *next* turn is a real report. The queue is
    //    serial, so once this one lands the duplicate has had its chance.
    let mut next = fixture("notify-agent-turn-complete");
    next["turn-id"] = "01a0bcae-0000-7042-8668-78526755dd62".into();
    next["last-assistant-message"] = "the second turn".into();
    let reported = daemon.rpc(&hooks, &call("hooks.codex_notify", next)).await;
    assert_eq!(reported["result"], serde_json::json!({}), "{reported}");
    daemon
        .until("the second turn is reported", |s| {
            s.agents.values().any(|a| {
                a.reply.as_ref().and_then(|r| r.text.as_deref()) == Some("the second turn")
            })
        })
        .await;
    let snapshot = daemon.api.bus.snapshot();
    for agent in snapshot.agents.values() {
        assert_ne!(
            agent.reply.as_ref().and_then(|r| r.text.as_deref()),
            Some("the duplicate"),
            "the duplicate turn was reported twice"
        );
    }
    drop(snapshot);
    daemon.shutdown().await;
}

/// The events this build does not act on answer `{}` and touch nothing —
/// C10's discipline, which is what keeps a Codex release that adds a
/// thirteenth event from taking a tool call down with it.
#[tokio::test]
async fn an_event_this_adapter_does_not_know_answers_empty_and_changes_nothing() {
    let daemon = Daemon::with(untrusted()).await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);
    started(&daemon, &hooks).await;
    // Put the session somewhere an unknown event could visibly move it
    // from, so "nothing changed" is an assertion and not a tautology.
    let prompted = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("user-prompt-submit")))
        .await;
    assert_eq!(prompted["result"], serde_json::json!({}), "{prompted}");
    daemon
        .until("the session is working", |s| {
            s.agents
                .values()
                .any(|a| a.state == deck_model::AgentState::Working)
        })
        .await;

    // `PostToolUse` is a real Codex event this build does not act on, and
    // the second one is from a Codex that does not exist yet.
    let mut future = fixture("post-tool-use");
    future["hook_event_name"] = "SomethingNewInTheNextRelease".into();
    for payload in [fixture("post-tool-use"), future] {
        let out = daemon.rpc(&hooks, &call("hooks.codex", payload)).await;
        assert_eq!(out["result"], serde_json::json!({}), "{out}");
    }
    let snapshot = daemon.api.bus.snapshot();
    assert!(
        snapshot
            .agents
            .values()
            .all(|a| a.state == deck_model::AgentState::Working),
        "an event we do not know moved the session"
    );
    drop(snapshot);

    // `Interrupt` *is* known: it is the one that must change something.
    let out = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("interrupt")))
        .await;
    assert_eq!(out["result"], serde_json::json!({}), "{out}");
    daemon
        .until("the interrupted session is idle", |s| {
            s.agents
                .values()
                .any(|a| a.state == deck_model::AgentState::Idle)
        })
        .await;
    daemon.shutdown().await;
}

/// `SessionEnd` releases the agent, and a `SessionEnd` for a session the
/// daemon never saw does not put one on the roster on its way out.
#[tokio::test]
async fn session_end_releases_the_agent_and_never_resurrects_an_unknown_one() {
    let daemon = Daemon::with(untrusted()).await;
    let hooks = daemon.bearer("hooks", &[Scope::HooksIngest]);

    // Unknown session first: nothing is registered.
    let out = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("session-end")))
        .await;
    assert_eq!(out["result"], serde_json::json!({}), "{out}");
    assert!(
        daemon.api.bus.snapshot().agents.is_empty(),
        "a SessionEnd never creates an agent"
    );

    started(&daemon, &hooks).await;
    let out = daemon
        .rpc(&hooks, &call("hooks.codex", fixture("session-end")))
        .await;
    assert_eq!(out["result"], serde_json::json!({}), "{out}");
    daemon
        .until("the agent is released", |s| {
            !s.agents
                .values()
                .any(|a| a.session.key.as_deref() == Some(KEY))
        })
        .await;
    daemon.shutdown().await;
}

/// B8: `integrations.status` reads `capabilities` to tell "the hooks are
/// installed and the daemon answers them" from "the hooks are installed
/// and this daemon has no adapter for them". The Codex adapter landing
/// puts its name there, on both surfaces.
#[tokio::test]
async fn the_daemon_advertises_hooks_codex() {
    let daemon = Daemon::start().await;
    let (status, info) = daemon.http("GET", "/v1/info", &[], "").await;
    assert_eq!(status, 200);
    let capabilities = info["capabilities"]
        .as_array()
        .unwrap_or_else(|| panic!("{info}"));
    assert!(
        capabilities.iter().any(|c| c == "hooks.codex"),
        "{capabilities:?}"
    );
    // The Claude adapter is in this build too, on the other plan layer,
    // and says so on the same surface. What must stay absent is a CLI
    // nothing answers for.
    assert!(
        !capabilities.iter().any(|c| c == "hooks.gemini"),
        "nothing answers hooks.gemini in this build: {capabilities:?}"
    );
    daemon.shutdown().await;
}

/// Contract §9. Fixtures are recorded from real sessions, so the scrubbing
/// is not a convention: it is checked every time they are read. A payload
/// that slipped a home directory, an API key or a real prompt into the
/// repository would be committed forever.
#[test]
fn every_committed_fixture_is_scrubbed() {
    let dir = fixtures();
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("the fixture directory") {
        let path = entry.expect("a fixture").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).expect("a readable fixture");
        serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
        let name = path.display();
        for secret in [
            "/Users/",
            "/home/",
            "sk-ant-",
            "sk-proj-",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "Bearer ",
            "@gmail",
            "ssh-rsa",
            "BEGIN PRIVATE KEY",
        ] {
            assert!(
                !text.contains(secret),
                "{name} carries {secret:?}: scrub it before committing"
            );
        }
        // Prompts, commands and assistant messages are the payload's own
        // content; each is replaced by a marker rather than shortened, so
        // "it looked harmless" is never the reason something stayed in.
        for (key, value) in [
            ("\"prompt\"", "REDACTED prompt"),
            ("\"command\"", "REDACTED command"),
            ("\"last_assistant_message\"", "REDACTED assistant message"),
            ("\"last-assistant-message\"", "REDACTED assistant message"),
        ] {
            if text.contains(key) {
                assert!(
                    text.contains(value),
                    "{name} has {key} but not the {value:?} marker"
                );
            }
        }
    }
    assert!(seen >= 8, "only {seen} fixtures found in {}", dir.display());
    // The session id in the fixtures is a UUID, not the `thr_…` the vendor
    // docs show (C19): a fixture in the old shape would silently test the
    // wrong thing.
    assert_eq!(fixture("session-start")["session_id"], SESSION);
    assert!(!SESSION.starts_with("thr_"));
}
