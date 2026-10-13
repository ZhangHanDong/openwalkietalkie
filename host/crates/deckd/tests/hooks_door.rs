//! What the **line door** owes a held hook (design §4.8, §4.9, §12).
//!
//! `hooks.<cli>` is the one method family whose whole product is the
//! response written back to a CLI that is blocked on it, and the UDS door
//! is the only door Codex ever uses (its hooks are shell form, C12/C13).
//! Two promises of that shape are the subject here, and both were broken
//! by the same thing: nothing owned the task a request ran on.
//!
//! 1. **A client that goes away releases its hold immediately.** Ctrl-C in
//!    the CLI, a killed helper, the helper hitting its own call timeout —
//!    §4.8/§12 say `HoldGone` "releases the hold and degrades the card
//!    *immediately* instead of letting both linger for `hold_s`". The
//!    `HoldGuard` that sends it is a local of the parked handler's future,
//!    so it only drops when the task does.
//! 2. **A notification form of a hook is not run at all.** With no `id`
//!    nothing is ever written back, so a hold parked for one is a card on
//!    the operator's deck that no CLI is waiting on, one of the 64
//!    `[api] max_held_hooks` slots burned for the whole `hold_s`, and the
//!    operator's answer reported `delivered` to nobody.
mod common;

use common::{Daemon, UdsClient};
use deck_model::{Deliver, Scope};
use serde_json::json;
use std::time::Duration;

/// A `PermissionRequest` exactly as `deckd hook claude` posts one. It is
/// the event that parks: the card goes up and the hook stays open.
fn permission() -> serde_json::Value {
    json!({
        "session_id": "01a0bca9-door",
        "cwd": "/w/project",
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": {"command": "psql -f migrate.sql staging"},
    })
}

/// A UDS peer holding `hooks:ingest`, which is what `deckd hook` is.
async fn helper(daemon: &Daemon) -> UdsClient {
    let mut helper = daemon.connect().await;
    let hello = helper
        .call(
            "session.hello",
            json!({"client": {"name": "deckd-hook", "version": "0"},
                   "scopes": ["hooks:ingest"]}),
        )
        .await;
    assert_eq!(hello["api"], "1", "{hello}");
    helper
}

fn parked(snapshot: &deck_model::Snapshot) -> bool {
    snapshot
        .agents
        .values()
        .any(|a| a.card.as_ref().is_some_and(|c| c.deliver == Deliver::Hold))
}

/// §4.8/§12: the card degrades when the hook client goes away, not when
/// `hold_s` runs out.
///
/// The budget is left at its default 120 s on purpose — the assertion is
/// that the degrade lands inside the harness's five-second patience, which
/// it cannot do by waiting for the hold to expire. Before the connection
/// owned its tasks, the parked handler ran on a detached `tokio::spawn`:
/// the session loop broke, `close_session` ran, and the hook's future sat
/// there with its `HoldGuard` still armed for the full two minutes, the
/// card `deliver:hold, status:pending` in front of an operator whose CLI
/// had already gone. Answering it then wrote the decision into a `resp_tx`
/// whose receiver had died with the loop, while `op.answer` said
/// `delivered:true`.
#[tokio::test]
async fn a_hook_client_that_goes_away_releases_its_hold_at_once() {
    let daemon = Daemon::start().await;
    let hold_s = daemon.api.config.load().hooks.claude.hold_s;
    assert!(
        hold_s >= 60,
        "this test proves the card does not wait for hold_s, so hold_s must be long: {hold_s}"
    );
    let mut hook = helper(&daemon).await;
    // The held call, written and never read: this is `deckd hook` parked
    // on the operator.
    hook.write_raw(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "hooks.claude",
                "params": {"payload": permission()}})
        .to_string(),
    )
    .await;
    daemon.until("the permission card is parked", parked).await;

    // Ctrl-C.
    drop(hook);
    let at = std::time::Instant::now();
    daemon
        .until("the hold is released and the card degraded", |s| {
            s.agents
                .values()
                .any(|a| a.card.as_ref().is_some_and(|c| c.deliver == Deliver::Focus))
        })
        .await;
    assert!(
        at.elapsed() < Duration::from_secs(u64::from(hold_s)),
        "the card waited out hold_s instead of the client's departure"
    );
    daemon.shutdown().await;
}

/// A hook with no `id` is refused before it reaches the adapter, on both
/// doors.
///
/// `agent.wait` has been refused in this form since v1 because its result
/// would go nowhere; a held hook is the same shape and worse, because the
/// thing that goes nowhere is a decision the operator was asked to make.
/// Any client holding `hooks:ingest` — or a helper that simply forgets the
/// id — could otherwise put a card on the deck that no CLI is waiting on.
#[tokio::test]
async fn the_notification_form_of_a_hook_is_never_run() {
    let daemon = Daemon::start().await;
    let auth = daemon.bearer("hooks", &[Scope::HooksIngest]);
    let mut hook = helper(&daemon).await;
    hook.write_raw(
        &json!({"jsonrpc": "2.0", "method": "hooks.claude",
                "params": {"payload": permission()}})
        .to_string(),
    )
    .await;
    // Nothing is written for a notification, so the next line on the wire
    // is the answer to the call behind it: the door read the hook and
    // moved on.
    let pong = hook.call("session.ping", json!({})).await;
    assert!(pong["pong"].is_boolean() || !pong.is_null(), "{pong}");

    // The TCP door: a body of notifications alone is `204 No Content`.
    let (status, body) = daemon
        .http(
            "POST",
            "/v1/rpc",
            &[("Authorization", &auth)],
            &json!({"jsonrpc": "2.0", "method": "hooks.claude",
                    "params": {"payload": permission()}})
            .to_string(),
        )
        .await;
    assert_eq!(status, 204, "{body}");

    // Neither one reached the adapter: no agent was registered for that
    // CLI session, so no card was ever raised for it either.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let snapshot = daemon.api.bus.snapshot();
    assert!(
        snapshot
            .agents
            .values()
            .all(|a| a.session.key.as_deref() != Some("claude:01a0bca9-door")),
        "a notification parked a hold nobody can read: {:?}",
        snapshot
            .agents
            .values()
            .map(|a| &a.session.key)
            .collect::<Vec<_>>()
    );
    assert!(!parked(&snapshot), "a card is on the deck for nobody");
    daemon.shutdown().await;
}
