//! Contract §5 rows 4 and 10: the Unix socket door — one session per
//! connection, ownership between two peers of the same uid, and the three
//! ways a first line can be wrong.
mod common;

use common::{register_params, Daemon, Options, UdsClient};
use deck_model::ApiError;

/// The scopes a UDS hello asks for in these tests.
fn hello_params(name: &str, scopes: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "client": {"name": name, "version": "1"},
        "scopes": scopes,
    })
}

/// Row 4: two peers of the same uid are two sessions; the key belongs to
/// the first until its connection goes away, then the second can claim it.
#[tokio::test]
async fn two_uds_clients_same_uid_get_a_key_conflict() {
    let daemon = Daemon::start().await;
    let mut first = daemon.connect().await;
    let hello = first
        .call("session.hello", hello_params("one", &["observe", "agent"]))
        .await;
    let uid = hello["principal"]["id"].as_str().unwrap().to_string();
    assert!(uid.starts_with("uid:"), "{hello}");
    assert_eq!(hello["principal"]["trusted"], false, "no verifier says so");
    assert_eq!(hello["api"], "1");
    let session_one = hello["session"].as_str().unwrap().to_string();
    assert!(session_one.starts_with("sess_"));

    let mut second = daemon.connect().await;
    let hello_two = second
        .call("session.hello", hello_params("two", &["observe", "agent"]))
        .await;
    let session_two = hello_two["session"].as_str().unwrap().to_string();
    assert_eq!(hello_two["principal"]["id"], uid, "the same uid");
    assert_ne!(session_one, session_two, "one session per connection");

    // The first registers the key; the second cannot take it.
    let mine = first
        .call("agent.register", register_params("one", "uds:key"))
        .await;
    let agent = mine["agent"].as_str().unwrap().to_string();
    assert_eq!(mine["owner"]["session"], session_one.as_str());
    assert_eq!(mine["owner"]["principal"], uid.as_str());
    let err = second
        .call_err("agent.register", register_params("two", "uds:key"))
        .await;
    assert_eq!(err["code"], ApiError::CONFLICT, "{err}");
    assert_eq!(err["data"]["owner"]["principal"], uid.as_str());
    assert_eq!(err["data"]["owner"]["session"], session_one.as_str());
    // Nor may it drive the agent.
    let err = second
        .call_err(
            "agent.update",
            serde_json::json!({"agent": agent, "state": "working"}),
        )
        .await;
    assert_eq!(err["code"], ApiError::CONFLICT);
    let err = second
        .call_err("agent.claim", serde_json::json!({"key": "uds:key"}))
        .await;
    assert_eq!(
        err["code"],
        ApiError::CONFLICT,
        "a live owner keeps its agent"
    );

    // The first connection goes away: the agent is orphaned and claimable.
    drop(first);
    let id = deck_model::AgentId(agent.clone());
    daemon
        .until("the first session to close", |snap| {
            snap.agents
                .get(&id)
                .is_some_and(|a| a.state == deck_model::AgentState::Idle)
        })
        .await;
    let claimed = second
        .call("agent.claim", serde_json::json!({"key": "uds:key"}))
        .await;
    assert_eq!(claimed["agent"], agent);
    assert_eq!(claimed["owner"]["session"], session_two.as_str());
    daemon.shutdown().await;
}

/// Row 10: the first line must be `session.hello`; anything else — a plain
/// call, an HTTP request line, or a line past the 256 KiB cap — is
/// answered once and the connection closes.
#[tokio::test]
async fn hello_must_be_first_and_http_is_refused() {
    let daemon = Daemon::start().await;

    // A call before the hello: -32001 {why:"hello_first"}, then EOF.
    let mut c = daemon.connect().await;
    c.write_raw(r#"{"method":"session.ping"}"#).await;
    let resp = c.line().await;
    assert_eq!(resp["error"]["code"], ApiError::UNAUTHORIZED, "{resp}");
    assert_eq!(resp["error"]["data"]["why"], "hello_first");
    assert_eq!(resp["id"], serde_json::Value::Null);
    assert!(c.closed().await, "the door hung up");

    // An HTTP request line is not JSON: -32700, then EOF (no HTTP parser
    // lives on the socket by design).
    let mut c = daemon.connect().await;
    c.write_raw("GET /v1/info HTTP/1.1\r").await;
    let resp = c.line().await;
    assert_eq!(resp["error"]["code"], ApiError::PARSE, "{resp}");
    assert!(c.closed().await);

    // A 300 KiB first line: -32012 {max}, then EOF.
    let mut c = daemon.connect().await;
    c.write_raw(&format!(
        r#"{{"id":1,"method":"session.hello","params":{{"pad":"{}"}}}}"#,
        "x".repeat(300 * 1024)
    ))
    .await;
    let resp = c.line().await;
    assert_eq!(resp["error"]["code"], ApiError::TOO_LARGE, "{resp}");
    assert_eq!(resp["error"]["data"]["max"], deckd::api::conn::LINE_MAX);
    assert!(c.closed().await);

    // A batch first is refused too (a hello is one request).
    let mut c = daemon.connect().await;
    c.write_raw(r#"[{"id":1,"method":"session.hello","params":{}}]"#)
        .await;
    let resp = c.line().await;
    assert_eq!(resp["error"]["code"], ApiError::UNAUTHORIZED);
    assert_eq!(resp["error"]["data"]["why"], "hello_first");
    assert!(c.closed().await);

    // After a good hello the very same connection serves calls, and a
    // line past the cap then also closes it.
    let mut c = daemon.connect().await;
    c.call("session.hello", hello_params("t", &["observe"]))
        .await;
    assert!(c.call("session.ping", serde_json::json!({})).await["ts"].is_string());
    c.write_raw(&"y".repeat(deckd::api::conn::LINE_MAX + 1))
        .await;
    let resp = c.line().await;
    assert_eq!(resp["error"]["code"], ApiError::TOO_LARGE);
    assert!(c.closed().await);
    daemon.shutdown().await;
}

/// The UDS maximum is `[api] uds_max_scope`: asking past it is refused by
/// name, and omitting `scopes` grants `uds_default_scope`.
#[tokio::test]
async fn the_uds_maximum_is_the_configured_scope() {
    let daemon = Daemon::with(Options {
        uds_max_scope: "observe".into(),
        uds_default_scope: "observe".into(),
        ..Options::default()
    })
    .await;
    let mut c = UdsClient::connect(&daemon.socket).await;
    let err = c
        .call_err("session.hello", hello_params("t", &["operate"]))
        .await;
    assert_eq!(err["code"], ApiError::FORBIDDEN, "{err}");
    assert_eq!(err["data"]["need"], "operate");
    assert!(err["data"].get("reason").is_none(), "not the ancestry cap");
    assert!(c.closed().await, "a refused hello closes the connection");

    let mut c = daemon.connect().await;
    let hello = c
        .call(
            "session.hello",
            serde_json::json!({"client": {"name": "t"}}),
        )
        .await;
    assert_eq!(hello["principal"]["scopes"], serde_json::json!(["observe"]));
    let err = c
        .call_err("agent.register", register_params("t", "k"))
        .await;
    assert_eq!(err["code"], ApiError::FORBIDDEN);
    assert_eq!(err["data"]["need"], "agent");
    daemon.shutdown().await;
}

/// `observe` alone never sees content; `observe:content` does. The same
/// snapshot, two sessions (§4.12).
#[tokio::test]
async fn the_uds_view_is_redacted_per_session_scope() {
    let daemon = Daemon::start().await;
    let mut owner = daemon.connect().await;
    owner
        .call(
            "session.hello",
            hello_params("owner", &["agent", "observe"]),
        )
        .await;
    let agent = owner
        .call("agent.register", register_params("owner", "uds:red"))
        .await["agent"]
        .as_str()
        .unwrap()
        .to_string();
    owner
        .call("agent.card.set", common::card_params(&agent, "secret?"))
        .await;

    let mut watcher = daemon.connect().await;
    watcher
        .call("session.hello", hello_params("watch", &["observe"]))
        .await;
    let state = watcher.call("state.get", serde_json::json!({})).await;
    let card = &state["snapshot"]["agents"][&agent]["card"];
    assert_eq!(card["text"], serde_json::Value::Null, "{state}");
    assert_eq!(card["redacted"], true);
    assert_eq!(card["choices_n"], 2);

    // The owner keeps its own content even with bare `observe`.
    let mine = owner.call("state.get", serde_json::json!({})).await;
    assert_eq!(
        mine["snapshot"]["agents"][&agent]["card"]["text"],
        "secret?"
    );
    daemon.shutdown().await;
}
