//! Contract §5 rows 6 and 7: what the peer verifier's verdict is worth at
//! the API — the guarded allow on a permission card that only a trusted
//! principal may make, and the agent-descended cap on a hello's scopes.
//! Both are driven by `FakeVerifier`; the real `MacVerifier` is B3's.
mod common;

use common::{permission_card_params, register_params, Daemon, Options};
use deck_model::{ApiError, CardId, CardSet, Scope};

/// A hello asking for exactly these scopes, as a UDS client sends it.
fn hello_params(name: &str, scopes: &[&str]) -> serde_json::Value {
    serde_json::json!({"client": {"name": name, "version": "1"}, "scopes": scopes})
}

/// Row 7: a peer whose ancestry holds an agent CLI (`claude`, `codex`,
/// `gemini`, `node`) is capped to `Scope::AGENT_DESCENDED_CAP` — asking
/// past it is refused by name with `reason:"agent_ancestry"`, and the four
/// scopes inside the cap are granted.
#[tokio::test]
async fn agent_descended_peer_is_capped() {
    let daemon = Daemon::with(Options {
        verifier: deckd::trust::FakeVerifier {
            trusted: false,
            capped: true,
        },
        ..Options::default()
    })
    .await;

    // `operate` is outside the cap: refused, by name, with the reason.
    let mut c = daemon.connect().await;
    let err = c
        .call_err("session.hello", hello_params("capped", &["operate"]))
        .await;
    assert_eq!(err["code"], ApiError::FORBIDDEN, "{err}");
    assert_eq!(err["data"]["need"], "operate");
    assert_eq!(err["data"]["reason"], "agent_ancestry");
    assert!(c.closed().await, "a refused hello closes the connection");

    // So are `admin`, `device` and `observe:content` — the cap is a
    // whitelist, not a subtraction of `operate`.
    for scope in ["admin", "device", "observe:content", "observe:raw"] {
        let mut c = daemon.connect().await;
        let err = c
            .call_err("session.hello", hello_params("capped", &[scope]))
            .await;
        assert_eq!(err["code"], ApiError::FORBIDDEN, "{scope}: {err}");
        assert_eq!(err["data"]["need"], scope, "{scope}");
    }

    // The cap itself is granted, in one hello.
    let mut c = daemon.connect().await;
    let hello = c
        .call(
            "session.hello",
            hello_params("capped", &["agent", "voice", "hooks:ingest"]),
        )
        .await;
    assert_eq!(
        hello["principal"]["scopes"],
        serde_json::json!(["agent", "voice", "hooks:ingest"]),
        "{hello}"
    );
    assert_eq!(hello["principal"]["trusted"], false);
    let hello_observe = daemon
        .connect()
        .await
        .call("session.hello", hello_params("capped", &["observe"]))
        .await;
    assert_eq!(
        hello_observe["principal"]["scopes"],
        serde_json::json!(["observe"])
    );

    // A capped session really may register an agent (`agent` is in the
    // cap) but may not answer anybody's card (`operate` is not).
    let agent = c
        .call("agent.register", register_params("capped", "uds:capped"))
        .await;
    assert!(agent["agent"].as_str().unwrap().starts_with("ag_"));
    let err = c
        .call_err(
            "op.answer",
            serde_json::json!({"agent": agent["agent"], "card": 1, "choice": 0}),
        )
        .await;
    assert_eq!(err["code"], ApiError::FORBIDDEN);
    assert_eq!(err["data"]["need"], "operate");

    // The same peer over HTTP is never capped: the cap is a UDS verdict
    // and a token's scopes are its own maximum.
    let auth = daemon.bearer("op", &[Scope::Observe, Scope::Operate]);
    let (status, body) = daemon
        .http(
            "POST",
            "/v1/rpc",
            &[("Authorization", &auth)],
            r#"{"id":1,"method":"op.answer","params":{"agent":"ag_nope","card":1,"choice":0}}"#,
        )
        .await;
    assert_eq!(status, 200);
    assert_ne!(
        body["error"]["code"],
        ApiError::FORBIDDEN,
        "the token holds operate: not a scope refusal, {body}"
    );
    daemon.shutdown().await;
}

/// Row 6: an **allow** on a permission card is a guarded operation — only
/// a trusted principal may make it. A token session and an untrusted UDS
/// peer are refused `-32002 {need:"trusted", how:"gui_confirm"}` and the
/// refusal puts `op.confirm_requested` on the bus for the GUI's sheet; a
/// **deny** needs no trust; a verified GUI peer's allow goes through.
///
/// The held card and the trust check inside `core::guard::Guard::check`
/// are B1's (`w2-core`, merged).
#[tokio::test]
async fn token_and_uds_allow_on_a_permission_card_are_refused() {
    // 1. An untrusted world: the daemon's verifier trusts nobody.
    let daemon = Daemon::with(Options {
        verifier: deckd::trust::FakeVerifier {
            trusted: false,
            capped: false,
        },
        ..Options::default()
    })
    .await;
    let (agent, card, mut hold) = held_permission_card(&daemon).await;

    // An observer watches `op.*` on its own connection.
    let mut observer = daemon.connect().await;
    observer
        .call(
            "session.hello",
            serde_json::json!({
                "client": {"name": "gui"}, "scopes": ["observe"], "subscribe": ["op.*"]
            }),
        )
        .await;

    // A token session with `operate` chooses Allow: refused.
    let auth = daemon.bearer("bot", &[Scope::Observe, Scope::Operate]);
    let answer = |choice: u8| {
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "op.answer",
            "params": {"agent": agent, "card": card, "choice": choice}
        })
        .to_string()
    };
    let refused = daemon.rpc(&auth, &answer(0)).await;
    assert_eq!(refused["error"]["code"], ApiError::FORBIDDEN, "{refused}");
    assert_eq!(refused["error"]["data"]["need"], "trusted");
    assert_eq!(refused["error"]["data"]["how"], "gui_confirm");

    // The refusal is on the bus, so the GUI can offer its sheet.
    let confirm = observer
        .line_where("op.confirm_requested", |v| {
            v["method"] == "event" && v["params"]["type"] == "op.confirm_requested"
        })
        .await;
    assert_eq!(confirm["params"]["data"]["agent"], agent);
    assert_eq!(confirm["params"]["data"]["card"], card.0);
    assert_eq!(confirm["params"]["data"]["need"], "trusted");
    assert_eq!(confirm["params"]["principal"], "token:bot");

    // An untrusted UDS peer with `operate` is refused the same way.
    let mut peer = daemon.connect().await;
    peer.call(
        "session.hello",
        hello_params("cli", &["observe", "operate"]),
    )
    .await;
    let err = peer
        .call_err(
            "op.answer",
            serde_json::json!({"agent": agent, "card": card, "choice": 0}),
        )
        .await;
    assert_eq!(err["code"], ApiError::FORBIDDEN, "{err}");
    assert_eq!(err["data"]["need"], "trusted");
    assert!(
        hold.try_recv().is_err(),
        "a refused allow never resolves the hold"
    );

    // Deny is never guarded: the same token may choose it, and the hold
    // resolves with that answer. The adapter that parked the hold writes
    // its hook response and says so, which is what lets `op.answer` answer
    // after the delivery (§4.9) instead of at the 4 s budget.
    let written = adapter_writes(&daemon, &agent, card, hold);
    let denied = daemon.rpc(&auth, &answer(2)).await;
    assert!(denied["error"].is_null(), "{denied}");
    assert_eq!(denied["result"]["delivered"], true, "{denied}");
    assert_eq!(denied["result"]["via"], "hold");
    let resolved = tokio::time::timeout(common::PATIENCE, written)
        .await
        .expect("the hold resolves")
        .expect("the adapter task lives")
        .expect("the hold was not dropped");
    match resolved {
        deckd::core::handle::HoldResult::Answer(answer) => {
            assert_eq!(answer.choice, Some(2));
            assert_eq!(answer.label.as_deref(), Some("Deny"));
        }
        other => panic!("{other:?}"),
    }
    daemon.shutdown().await;

    // 2. A trusted world: the verifier recognises the GUI, whose allow
    //    goes through and resolves the hold with the allowing choice.
    let daemon = Daemon::with(Options {
        verifier: deckd::trust::FakeVerifier {
            trusted: true,
            capped: false,
        },
        ..Options::default()
    })
    .await;
    let (agent, card, hold) = held_permission_card(&daemon).await;
    let mut gui = daemon.connect().await;
    let hello = gui
        .call(
            "session.hello",
            hello_params("gui", &["observe", "operate"]),
        )
        .await;
    assert_eq!(hello["principal"]["trusted"], true, "{hello}");
    let written = adapter_writes(&daemon, &agent, card, hold);
    let allowed = gui
        .call(
            "op.answer",
            serde_json::json!({"agent": agent, "card": card, "choice": 0}),
        )
        .await;
    // The allow is accepted and the reply waits for the hook response to be
    // written (§4.9), so `delivered` is the truth and not a promise.
    assert!(allowed["error"].is_null(), "{allowed}");
    assert_eq!(allowed["delivered"], true, "{allowed}");
    assert_eq!(allowed["via"], "hold");
    let resolved = tokio::time::timeout(common::PATIENCE, written)
        .await
        .expect("the hold resolves")
        .expect("the adapter task lives")
        .expect("the hold was not dropped");
    match resolved {
        deckd::core::handle::HoldResult::Answer(answer) => {
            assert_eq!(answer.choice, Some(0));
            assert_eq!(answer.label.as_deref(), Some("Allow"));
        }
        other => panic!("{other:?}"),
    }
    daemon.shutdown().await;
}

/// What a real adapter does with a resolved hold: write the hook response
/// and tell the core (`hold_written`), which clears the card and completes
/// the delivery. The task hands the resolution back.
fn adapter_writes(
    daemon: &Daemon,
    agent: &str,
    card: CardId,
    hold: deckd::core::handle::HoldRx,
) -> tokio::task::JoinHandle<
    Result<deckd::core::handle::HoldResult, tokio::sync::oneshot::error::RecvError>,
> {
    let core = daemon.api.core.clone();
    let agent = deck_model::AgentId(agent.to_string());
    tokio::spawn(async move {
        let resolved = hold.await;
        if resolved.is_ok() {
            core.hold_written(agent, card).await;
        }
        resolved
    })
}

/// The §4.13 scenario as the demo builds it: an adapter-owned agent with a
/// **held** permission card. Returns the agent id, its card and the parked
/// side of the hold (the guard is disarmed: the test, not a drop, decides).
async fn held_permission_card(daemon: &Daemon) -> (String, CardId, deckd::core::handle::HoldRx) {
    let (agent, session) = daemon.adapter_agent("cohdl-api").await;
    let params = permission_card_params(&agent);
    let cmd: CardSet = serde_json::from_value(params).expect("a well-formed CardSet");
    let (card, hold, mut guard) = daemon
        .api
        .core
        .set_card_held(
            deck_model::Principal::Adapter(deck_model::AdapterKind::Claude),
            session,
            cmd,
        )
        .await
        .expect("the core holds the card");
    guard.disarm();
    (agent, card, hold)
}
