//! Hostile hook payloads through **both** doors.
//!
//! A hook payload is attacker-shaped by construction: it is whatever the
//! CLI's model, the operator's prompt and the tool arguments happen to
//! contain, and the daemon has no say in any of it. The week-3 contract
//! §9 names one failure the daemon may never have: a hook that makes a CLI
//! wait, or that answers it with something the CLI cannot read. A panic in
//! the handler is exactly that — the connection dies mid-response, Claude
//! sees `curl: (52) Empty reply from server`, and for Codex an empty reply
//! from the helper's own door is an exit-2 **deny** on a tool the operator
//! never saw.
//!
//! So every row below is fed to `POST /v1/hooks/<cli>` **and** to
//! `hooks.<cli>` over the UDS (which is how `deckd hook` reaches the
//! daemon), for both CLIs that have an adapter, and every one of them must
//! answer a JSON object with a 200 — never a closed connection, never a
//! non-object body, and never a panic raised from `deckd`'s own source.
//!
//! The one row that reproduced live before this file existed is
//! [`a_non_ascii_session_id_with_no_cwd_does_not_panic`]: `name_of` cut the
//! session id with a **byte** slice, so `{"session_id":"aaaaaé",
//! "hook_event_name":"Stop"}` panicked inside the handler and the CLI got
//! an empty reply.
mod common;

use common::{Daemon, UdsClient, PATIENCE};
use deck_model::Scope;
use serde_json::json;
use std::sync::{Mutex, Once, OnceLock};

/// The hold budget these rows run with. A `PermissionRequest` parks, and a
/// hostile payload is still a payload: the row waits out the budget rather
/// than skipping the held path, so one second is what makes the table
/// affordable.
const HOSTILE_HOLD_S: u32 = 1;

// ---------------------------------------------------------------------------
// Panic capture
//
// A panic inside a hyper connection task does not fail the test process: the
// task unwinds, the connection is dropped and the client sees an empty
// reply. The assertions below would catch that (there is no status line to
// parse), but they would not say *where* — so the hook records every panic
// raised from `deckd`'s own `src/`, and the rows assert the list is empty.
// A panic from a test's own `assert!` lives under `tests/` and is left
// alone, and the previous hook still runs, so a genuinely failing test
// prints what it always printed.

fn panics() -> &'static Mutex<Vec<String>> {
    static PANICS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PANICS.get_or_init(|| Mutex::new(Vec::new()))
}

fn watch_for_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(at) = info.location() {
                // `…/deckd/src/…` is the daemon; `…/tests/…` is a row's own
                // assertion and is none of this file's business.
                let file = at.file().replace('\\', "/");
                if file.contains("/src/") || file.starts_with("src/") {
                    if let Ok(mut seen) = panics().lock() {
                        seen.push(format!("{file}:{}", at.line()));
                    }
                }
            }
            previous(info);
        }));
    });
}

/// Every panic raised from the daemon's own source since the process
/// started. It is a whole-binary list on purpose: a panic on a spawned task
/// belongs to whichever row put the payload in, and a row that leaks one
/// into another row's window is still a bug in this file's subject.
fn daemon_panics() -> Vec<String> {
    panics().lock().map(|s| s.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The doors

/// A daemon whose held cards expire in a second, with a `hooks:ingest`
/// bearer and a UDS peer holding the same scope.
async fn doors() -> (Daemon, String, UdsClient) {
    watch_for_panics();
    let daemon = Daemon::start().await;
    // The Claude adapter reads `[hooks.claude]` live off the api config
    // (B2); the Codex adapter's budget lives on the core's own adapter
    // table, refreshed by `Core::set_config`.
    let mut config = (**daemon.api.config.load()).clone();
    config.hooks.claude.hold_s = HOSTILE_HOLD_S;
    config.hooks.codex.hold_s = HOSTILE_HOLD_S;
    daemon.api.config.store(std::sync::Arc::new(config));
    daemon
        .api
        .core
        .adapters()
        .set_hold_s(HOSTILE_HOLD_S, HOSTILE_HOLD_S, HOSTILE_HOLD_S);
    let auth = daemon.bearer("hooks", &[Scope::HooksIngest]);
    let mut helper = daemon.connect().await;
    helper
        .call(
            "session.hello",
            json!({"client": {"name": "deckd-hook", "version": "0"},
                   "scopes": ["hooks:ingest"]}),
        )
        .await;
    (daemon, auth, helper)
}

/// The TCP door: the status and the body, exactly as Claude reads them.
async fn post(daemon: &Daemon, auth: &str, cli: &str, payload: &serde_json::Value) -> String {
    let (status, body) = tokio::time::timeout(
        PATIENCE * 4,
        daemon.http(
            "POST",
            &format!("/v1/hooks/{cli}"),
            &[("Authorization", auth)],
            &payload.to_string(),
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("POST /v1/hooks/{cli} never answered"));
    assert_eq!(status, 200, "POST /v1/hooks/{cli}: {body}");
    assert!(
        body.is_object(),
        "POST /v1/hooks/{cli} answered something a CLI cannot parse: {body}"
    );
    body.to_string()
}

/// The UDS door, which is the one `deckd hook` uses.
async fn over_uds(
    helper: &mut UdsClient,
    cli: &str,
    payload: &serde_json::Value,
) -> serde_json::Value {
    let reply = tokio::time::timeout(
        PATIENCE * 4,
        helper.request(&format!("hooks.{cli}"), json!({"payload": payload})),
    )
    .await
    .unwrap_or_else(|_| panic!("hooks.{cli} never answered"));
    assert!(
        reply["error"].is_null(),
        "hooks.{cli} refused a payload it must answer: {reply}"
    );
    assert!(
        reply["result"].is_object(),
        "hooks.{cli} answered something a CLI cannot parse: {reply}"
    );
    reply["result"].clone()
}

/// One payload through both doors, for both CLIs with an adapter.
async fn both_doors(
    daemon: &Daemon,
    auth: &str,
    helper: &mut UdsClient,
    payload: &serde_json::Value,
) {
    for cli in ["claude", "codex"] {
        post(daemon, auth, cli, payload).await;
        over_uds(helper, cli, payload).await;
    }
}

// ---------------------------------------------------------------------------
// The table

/// A JSON value nested `depth` objects deep — `{"a":{"a":{…}}}`.
fn nested(depth: usize) -> serde_json::Value {
    let mut value = json!({"leaf": true});
    for _ in 0..depth {
        value = json!({ "a": value });
    }
    value
}

/// Every hostile payload, with a name for the failure message.
fn hostile() -> Vec<(&'static str, serde_json::Value)> {
    // 200 kB fits under both doors' body limits (1 MiB on TCP,
    // `conn::LINE_MAX` on the socket), so it exercises the adapters rather
    // than the doors' own caps, which have rows of their own below.
    let big = "b".repeat(200_000);
    vec![
        // The live repro: a multi-byte session id with nothing else to name
        // the agent by, which `name_of` used to cut with a byte slice.
        (
            "non-ascii id, no cwd",
            json!({"session_id": "aaaaaé", "hook_event_name": "Stop"}),
        ),
        (
            "non-ascii id on the register path",
            json!({"session_id": "日本語のセッション", "hook_event_name": "SessionStart"}),
        ),
        (
            "non-ascii id cut at every byte offset",
            json!({"session_id": "éééééééé", "hook_event_name": "UserPromptSubmit"}),
        ),
        (
            "a 4-byte emoji id",
            json!({"session_id": "🙂🙂🙂", "hook_event_name": "Stop"}),
        ),
        (
            "a cwd that is only separators",
            json!({"session_id": "ünïcøde", "hook_event_name": "SessionStart", "cwd": "///"}),
        ),
        (
            "a cwd that is only whitespace",
            json!({"session_id": "ünïcøde", "hook_event_name": "SessionStart", "cwd": "/   /"}),
        ),
        (
            "every string empty",
            json!({"session_id": "", "hook_event_name": "", "cwd": "", "tool_name": "", "prompt": ""}),
        ),
        ("an empty object", json!({})),
        ("a bare array", json!([1, 2, 3])),
        ("a bare string", json!("hook_event_name")),
        ("a bare number", json!(42)),
        ("a bare null", serde_json::Value::Null),
        ("a bare bool", json!(true)),
        (
            "hook_event_name is not a string",
            json!({"session_id": "s", "hook_event_name": 7}),
        ),
        (
            "session_id is not a string",
            json!({"session_id": {"nope": true}, "hook_event_name": "Stop"}),
        ),
        (
            "null where an object is expected",
            json!({"session_id": "s-null", "hook_event_name": "PermissionRequest",
                   "tool_name": "Bash", "tool_input": null}),
        ),
        (
            "an array where an object is expected",
            json!({"session_id": "s-arr", "hook_event_name": "PermissionRequest",
                   "tool_name": "Bash", "tool_input": [1, 2, 3]}),
        ),
        (
            "a string where an object is expected",
            json!({"session_id": "s-str", "hook_event_name": "PermissionRequest",
                   "tool_name": "Bash", "tool_input": "rm -rf /"}),
        ),
        (
            "a deeply nested tool_input",
            json!({"session_id": "s-deep", "hook_event_name": "PermissionRequest",
                   "tool_name": "mcp__srv__tool", "tool_input": nested(100)}),
        ),
        (
            "a deeply nested payload with no session",
            json!({"hook_event_name": "PostToolUse", "tool_input": nested(100)}),
        ),
        (
            "a 200 kB session id",
            json!({"session_id": big.clone(), "hook_event_name": "SessionStart"}),
        ),
        (
            "a 200 kB assistant message",
            json!({"session_id": "s-big", "hook_event_name": "Stop",
                   "last_assistant_message": big.clone()}),
        ),
        (
            "a 200 kB command on a held card",
            json!({"session_id": "s-bigcmd", "hook_event_name": "PermissionRequest",
                   "tool_name": "Bash", "tool_input": {"command": big.clone()}}),
        ),
        (
            "an AskUserQuestion with no options",
            json!({"session_id": "s-q0", "hook_event_name": "PreToolUse",
                   "tool_name": "AskUserQuestion",
                   "tool_input": {"questions": [{"question": "?", "options": []}]}}),
        ),
        (
            "an AskUserQuestion with too many questions",
            json!({"session_id": "s-q9", "hook_event_name": "PreToolUse",
                   "tool_name": "AskUserQuestion",
                   "tool_input": {"questions": (0..9).map(|n| json!({"question": n.to_string(),
                        "options": [{"label": "a"}, {"label": "b"}]})).collect::<Vec<_>>()}}),
        ),
        (
            "an AskUserQuestion whose questions are not an array",
            json!({"session_id": "s-qn", "hook_event_name": "PreToolUse",
                   "tool_name": "AskUserQuestion", "tool_input": {"questions": "two"}}),
        ),
        (
            "an AskUserQuestion whose labels are all escapes",
            json!({"session_id": "s-qe", "hook_event_name": "PreToolUse",
                   "tool_name": "AskUserQuestion",
                   "tool_input": {"questions": [{"question": "\u{1b}[31m", "header": "\u{1b}[0m",
                        "options": [{"label": "\u{1b}[1m"}, {"label": "\u{7}"}]}]}}),
        ),
        (
            "an unknown notification type",
            json!({"session_id": "s-n", "hook_event_name": "Notification",
                   "notification_type": "something_2_2_0_added", "message": null}),
        ),
        (
            "a notification whose message is an object",
            json!({"session_id": "s-n2", "hook_event_name": "Notification",
                   "notification_type": "agent_needs_input", "message": {"text": "hi"}}),
        ),
        (
            "an unknown event",
            json!({"session_id": "s-u", "hook_event_name": "SubagentStopFailureCompact"}),
        ),
        (
            "a SessionEnd for a session nobody registered",
            json!({"session_id": "never-seen-é", "hook_event_name": "SessionEnd"}),
        ),
        // Codex's own shapes: the notify payload and its kebab-cased ids.
        (
            "a codex notify payload with no ids",
            json!({"type": "agent-turn-complete"}),
        ),
        (
            "a codex notify payload whose ids are objects",
            json!({"type": "agent-turn-complete", "thread-id": {"a": 1}, "turn-id": []}),
        ),
        (
            "a codex tool_input whose command is an array of non-strings",
            json!({"session_id": "cx-1", "hook_event_name": "PreToolUse",
                   "tool_name": "shell", "tool_input": {"command": [1, {"a": 2}, null]}}),
        ),
    ]
}

// ---------------------------------------------------------------------------
// The rows

/// The live repro, on its own so a regression names itself: a non-ASCII
/// `session_id` with no usable `cwd` is the shortest payload that reached
/// `name_of`'s fallback, and the fallback cut it mid-character.
#[tokio::test]
async fn a_non_ascii_session_id_with_no_cwd_does_not_panic() {
    let (daemon, auth, mut helper) = doors().await;
    let payload = json!({"session_id": "aaaaaé", "hook_event_name": "Stop"});
    both_doors(&daemon, &auth, &mut helper, &payload).await;
    // The same id through the one event that builds an agent name from it.
    let start = json!({"session_id": "aaaaaé", "hook_event_name": "SessionStart"});
    both_doors(&daemon, &auth, &mut helper, &start).await;
    assert_eq!(daemon_panics(), Vec::<String>::new());
    daemon.shutdown().await;
}

/// The whole table, through both doors, for both adapters.
#[tokio::test]
async fn every_hostile_payload_is_answered_with_json_on_both_doors() {
    let (daemon, auth, mut helper) = doors().await;
    for (name, payload) in hostile() {
        for cli in ["claude", "codex"] {
            let body = post(&daemon, &auth, cli, &payload).await;
            assert!(!body.is_empty(), "{name} / {cli}: an empty body");
            over_uds(&mut helper, cli, &payload).await;
        }
        assert_eq!(daemon_panics(), Vec::<String>::new(), "{name}");
    }
    daemon.shutdown().await;
}

/// A body over the door's own cap is refused **with a body**: the caps are
/// the doors', not the adapters', and a refusal a CLI can parse is what
/// keeps it from waiting. Claude treats a non-2xx http hook as a
/// non-blocking error and shows its own dialog; the helper prints `{}` and
/// exits 0 whatever comes back.
#[tokio::test]
async fn a_payload_over_a_door_cap_is_refused_with_json_not_a_dead_socket() {
    let (daemon, auth, mut helper) = doors().await;
    let over_http = json!({"session_id": "s", "hook_event_name": "Stop",
                           "last_assistant_message": "x".repeat(deckd::api::http::BODY_MAX)});
    let (status, body) = daemon
        .http(
            "POST",
            "/v1/hooks/claude",
            &[("Authorization", &auth)],
            &over_http.to_string(),
        )
        .await;
    assert_eq!(status, 413, "{body}");
    assert!(body.is_object(), "{body}");
    assert_eq!(
        body["error"]["code"],
        deck_model::ApiError::TOO_LARGE,
        "{body}"
    );

    // The socket's own cap, on a throwaway connection: `api::conn` writes
    // `-32012` and then closes the line, and whether that response survives
    // the close is the kernel's business (a close with unread bytes still
    // in flight resets the connection and drops what was already written).
    // What this row is about is the daemon: no panic, and the door still
    // answering hooks afterwards.
    let mut throwaway = daemon.connect().await;
    throwaway
        .call(
            "session.hello",
            json!({"client": {"name": "deckd-hook", "version": "0"},
                   "scopes": ["hooks:ingest"]}),
        )
        .await;
    let over_line = json!({"session_id": "s", "hook_event_name": "Stop",
                           "last_assistant_message":
                               "x".repeat(deckd::api::conn::LINE_MAX)});
    let line = json!({"jsonrpc": "2.0", "id": 99, "method": "hooks.claude",
                      "params": {"payload": over_line}})
    .to_string();
    throwaway.write_raw(&line).await;
    drop(throwaway);

    // The door still works afterwards, on a connection that was open the
    // whole time.
    over_uds(
        &mut helper,
        "claude",
        &json!({"session_id": "after", "hook_event_name": "Stop"}),
    )
    .await;
    assert_eq!(daemon_panics(), Vec::<String>::new());
    daemon.shutdown().await;
}

/// A body that is not JSON at all: the TCP door answers `{}` rather than
/// letting a parse error reach the CLI as a non-answer.
#[tokio::test]
async fn a_body_that_is_not_json_is_answered_empty() {
    let (daemon, auth, _helper) = doors().await;
    for body in ["", "not json", "{\"unterminated\":", "\u{feff}{}"] {
        let (status, answer) = daemon
            .http(
                "POST",
                "/v1/hooks/claude",
                &[("Authorization", &auth)],
                body,
            )
            .await;
        assert_eq!((status, answer), (200, json!({})), "{body:?}");
    }
    assert_eq!(daemon_panics(), Vec::<String>::new());
    daemon.shutdown().await;
}
