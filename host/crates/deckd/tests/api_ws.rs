//! Contract §5 row 2: `GET /v1/ws` is the same line session as the Unix
//! socket, one JSON value per text frame — hello with a bearer, a
//! subscription glob, a live event from another session, and the ring
//! replayed from `since` on the next connection.
mod common;

use common::{card_params, register_params, Daemon};
use deck_model::Scope;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect with `Authorization: Bearer <token>`.
async fn open(port: u16, token: &str) -> Ws {
    let mut req = format!("ws://127.0.0.1:{port}/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    connect_async(req).await.expect("the ws upgrade").0
}

async fn send(ws: &mut Ws, value: serde_json::Value) {
    ws.send(Message::text(value.to_string())).await.unwrap();
}

/// The next JSON frame (pings and pongs are the socket's business).
async fn frame(ws: &mut Ws) -> serde_json::Value {
    loop {
        let msg = tokio::time::timeout(common::PATIENCE, ws.next())
            .await
            .expect("a frame in time")
            .expect("the socket is open")
            .expect("a readable frame");
        if let Ok(text) = msg.to_text() {
            if !text.is_empty() {
                return serde_json::from_str(text).unwrap_or_else(|_| panic!("not json: {text:?}"));
            }
        }
    }
}

/// The next frame that is an `event` notification of type `kind`.
async fn event(ws: &mut Ws, kind: &str) -> serde_json::Value {
    let deadline = std::time::Instant::now() + common::PATIENCE;
    loop {
        let v = frame(ws).await;
        if v["method"] == "event" && v["params"]["type"] == kind {
            return v;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no {kind} event arrived"
        );
    }
}

#[tokio::test]
async fn ws_round_trip_with_events() {
    let daemon = Daemon::start().await;
    let watcher_token = daemon.token("watcher", &[Scope::Observe, Scope::ObserveContent]);

    // 1. Hello with the bearer: the token names the principal, its scopes
    //    are the maximum, and `trusted` is never true over HTTP.
    let mut ws = open(daemon.port, &watcher_token).await;
    send(
        &mut ws,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "session.hello",
            "params": {"client": {"name": "watcher", "version": "1"}, "subscribe": ["card.*"]}
        }),
    )
    .await;
    let hello = frame(&mut ws).await;
    assert_eq!(hello["id"], 1);
    assert_eq!(hello["result"]["api"], "1", "{hello}");
    assert_eq!(hello["result"]["principal"]["id"], "token:watcher");
    assert_eq!(hello["result"]["principal"]["trusted"], false);
    assert_eq!(
        hello["result"]["principal"]["scopes"],
        serde_json::json!(["observe", "observe:content"]),
        "omitted scopes: the token's own list"
    );
    let rev0 = hello["result"]["rev"].as_u64().unwrap();
    assert!(
        hello["result"].get("replayed").is_none(),
        "no since, no replay"
    );

    // 2. `session.ping` and `session.subscribe` are answered on the socket.
    send(
        &mut ws,
        serde_json::json!({"id": 2, "method": "session.ping"}),
    )
    .await;
    let pong = frame(&mut ws).await;
    assert!(pong["result"]["ts"].is_string(), "{pong}");
    send(
        &mut ws,
        serde_json::json!({"id": 3, "method": "session.subscribe", "params": {"types": ["agent.added"]}}),
    )
    .await;
    assert_eq!(
        frame(&mut ws).await["result"]["subscribed"],
        serde_json::json!(["card.*", "agent.added"])
    );

    // 3. Another session (the Unix socket) registers an agent and sets a
    //    card: both events reach this socket, the card with its content
    //    (this token has `observe:content`).
    let mut setter = daemon.connect().await;
    setter
        .call(
            "session.hello",
            serde_json::json!({"client": {"name": "setter"}, "scopes": ["agent", "observe"]}),
        )
        .await;
    let agent = setter
        .call("agent.register", register_params("setter", "ws:key"))
        .await["agent"]
        .as_str()
        .unwrap()
        .to_string();
    let added = event(&mut ws, "agent.added").await;
    assert_eq!(added["jsonrpc"], "2.0");
    assert_eq!(added["params"]["data"]["agent"]["name"], "setter");
    setter
        .call("agent.card.set", card_params(&agent, "ship it?"))
        .await;
    let card = event(&mut ws, "card.set").await;
    assert_eq!(card["params"]["data"]["agent"], agent);
    assert_eq!(card["params"]["data"]["card"]["text"], "ship it?");
    let rev1 = card["params"]["rev"].as_u64().unwrap();
    assert!(rev1 > rev0);

    // 4. An `agent.changed` is not subscribed, so it never arrives: the
    //    next event this socket sees is the second card.
    setter
        .call(
            "agent.update",
            serde_json::json!({"agent": agent, "state": "working"}),
        )
        .await;
    setter
        .call("agent.card.set", card_params(&agent, "and again?"))
        .await;
    let next = event(&mut ws, "card.set").await;
    assert_eq!(next["params"]["data"]["card"]["text"], "and again?");
    let rev2 = next["params"]["rev"].as_u64().unwrap();
    ws.close(None).await.unwrap();

    // 5. A second connection with `since: rev0` replays the ring through
    //    its own subscriptions and redaction, counted in `replayed`.
    let mut replayer = open(daemon.port, &watcher_token).await;
    send(
        &mut replayer,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "session.hello",
            "params": {
                "client": {"name": "replayer"}, "subscribe": ["card.set"],
                "since": rev0, "snapshot": true
            }
        }),
    )
    .await;
    let hello = frame(&mut replayer).await;
    assert!(
        hello["result"].get("gap").is_none(),
        "the ring still holds it"
    );
    let replayed = hello["result"]["replayed"]
        .as_u64()
        .expect("a replay count");
    assert_eq!(replayed, 2, "two card.set events since rev0: {hello}");
    assert!(hello["result"]["snapshot"]["agents"].is_object());
    let first = frame(&mut replayer).await;
    assert_eq!(first["params"]["type"], "card.set");
    assert_eq!(first["params"]["data"]["card"]["text"], "ship it?");
    let second = frame(&mut replayer).await;
    assert_eq!(second["params"]["rev"], rev2);
    assert_eq!(second["params"]["data"]["card"]["text"], "and again?");

    // A `since` older than the ring is a `gap`, with no replay at all.
    let mut lagged = open(daemon.port, &watcher_token).await;
    send(
        &mut lagged,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "session.hello",
            "params": {"client": {"name": "lagged"}, "since": rev2}
        }),
    )
    .await;
    let hello = frame(&mut lagged).await;
    assert_eq!(
        hello["result"]["replayed"], 0,
        "nothing after the newest rev"
    );
    tokio::time::timeout(Duration::from_millis(50), lagged.next())
        .await
        .expect_err("no further frames without a subscription");
    daemon.shutdown().await;
}
