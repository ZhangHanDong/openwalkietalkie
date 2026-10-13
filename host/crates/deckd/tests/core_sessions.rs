//! The session registry's bounds (contract §3.3/§3.4): a bearer token may
//! not turn `POST /v1/rpc` into unbounded memory. `client.name` is capped,
//! a token holds at most `LOGICAL_PER_TOKEN_MAX` logical sessions (its
//! least-recently-used one is evicted for a new one) and the daemon holds
//! at most `LOGICAL_TOTAL_MAX` across tokens (over that a *new* logical
//! session is refused, so one token can never evict another's).
mod common;
mod core_support;

use common::Daemon;
use deck_model::{ApiError, Principal, Scope, SessionId};
use deckd::core::sessions::{CLIENT_NAME_MAX, LOGICAL_PER_TOKEN_MAX, LOGICAL_TOTAL_MAX};
use deckd::core::{ClientInfo, Core, CoreMsg, OpenSession, SessionKind};
use std::time::{Duration, Instant};

/// One `CoreMsg::LogicalSession`, answered synchronously by the core.
fn logical(
    core: &mut Core,
    token: &str,
    client: &str,
    now: Instant,
) -> Result<SessionId, ApiError> {
    let (reply, mut rx) = tokio::sync::oneshot::channel();
    core.on_msg(
        CoreMsg::LogicalSession {
            token: token.into(),
            client: client.into(),
            principal: Principal::Token(token.into()),
            scopes: vec![Scope::Agent],
            reply,
        },
        now,
    );
    rx.try_recv().expect("the core answered at once")
}

/// One `CoreMsg::OpenSession` (a WS/UDS hello), answered synchronously.
fn open(core: &mut Core, name: &str, now: Instant) -> Result<SessionId, ApiError> {
    let (reply, mut rx) = tokio::sync::oneshot::channel();
    core.on_msg(
        CoreMsg::OpenSession {
            open: OpenSession {
                kind: SessionKind::Observer,
                principal: Principal::Uid(501),
                scopes: vec![Scope::Observe],
                trusted: false,
                client: ClientInfo {
                    name: name.into(),
                    version: String::new(),
                },
            },
            reply,
        },
        now,
    );
    rx.try_recv().expect("the core answered at once")
}

#[test]
fn an_over_long_client_name_is_refused_on_both_session_paths() {
    let (mut core, parts) = core_support::build("client-name", 64);
    let t0 = Instant::now();
    let long = "n".repeat(CLIENT_NAME_MAX + 1);
    let at_the_cap = "n".repeat(CLIENT_NAME_MAX);

    let error = logical(&mut core, "tok", &long, t0).unwrap_err();
    assert_eq!(error.code, ApiError::INVALID);
    assert_eq!(error.data["field"], "client.name");
    assert_eq!(core.sessions.logical_len(), 0, "nothing was allocated");

    assert!(logical(&mut core, "tok", &at_the_cap, t0).is_ok());
    assert_eq!(core.sessions.logical_len(), 1);

    // The same rule on a hello, so a connection cannot smuggle one in.
    let error = open(&mut core, &long, t0).unwrap_err();
    assert_eq!(error.code, ApiError::INVALID);
    assert_eq!(error.data["field"], "client.name");
    assert!(open(&mut core, &at_the_cap, t0).is_ok());
    let _ = std::fs::remove_dir_all(&parts.dir);
}

#[test]
fn logical_sessions_are_capped_per_token_and_in_total() {
    let (mut core, parts) = core_support::build("logical-caps", 64);
    let t0 = Instant::now();

    // One token fills its own allowance; each call is one second later, so
    // the first client is unambiguously the least recently used.
    let mut ids = Vec::new();
    for n in 0..LOGICAL_PER_TOKEN_MAX {
        let at = t0 + Duration::from_secs(n as u64);
        ids.push(logical(&mut core, "hot", &format!("bot{n}"), at).unwrap());
    }
    assert_eq!(core.sessions.logical_len(), LOGICAL_PER_TOKEN_MAX);
    assert_eq!(core.sessions.logical_of_token("hot"), LOGICAL_PER_TOKEN_MAX);

    // An existing `(token, client)` is only refreshed: no eviction.
    let again = logical(&mut core, "hot", "bot0", t0 + Duration::from_secs(100)).unwrap();
    assert_eq!(again, ids[0]);
    assert_eq!(core.sessions.logical_of_token("hot"), LOGICAL_PER_TOKEN_MAX);

    // A new one evicts the least recently used — which `bot0`'s refresh
    // just made `bot1` — and the count stays at the cap.
    let fresh = logical(&mut core, "hot", "overflow", t0 + Duration::from_secs(200)).unwrap();
    assert_eq!(core.sessions.logical_of_token("hot"), LOGICAL_PER_TOKEN_MAX);
    assert!(core.sessions.contains(fresh));
    assert!(core.sessions.contains(ids[0]), "the refreshed one stayed");
    assert!(!core.sessions.contains(ids[1]), "the idlest one went");

    // Another token is untouched by that pressure.
    let other = logical(&mut core, "cold", "bot", t0).unwrap();
    assert_eq!(core.sessions.logical_of_token("cold"), 1);
    assert!(core.sessions.contains(other));

    // Fill the daemon: 16 tokens × 32 = LOGICAL_TOTAL_MAX. Over it a new
    // logical session is refused rather than evicting a stranger's.
    let mut token = 0;
    while core.sessions.logical_len() < LOGICAL_TOTAL_MAX {
        let name = format!("t{token}");
        for n in 0..LOGICAL_PER_TOKEN_MAX {
            if core.sessions.logical_len() >= LOGICAL_TOTAL_MAX {
                break;
            }
            logical(&mut core, &name, &format!("c{n}"), t0).unwrap();
        }
        token += 1;
    }
    assert_eq!(core.sessions.logical_len(), LOGICAL_TOTAL_MAX);
    let error = logical(&mut core, "latecomer", "bot", t0).unwrap_err();
    assert_eq!(error.code, ApiError::RATE_LIMITED);
    assert!(error.data["retry_ms"].as_u64().unwrap_or(0) > 0);
    assert_eq!(core.sessions.logical_len(), LOGICAL_TOTAL_MAX);
    // A token that already has a session still gets served.
    assert!(core.sessions.contains(other));
    assert_eq!(logical(&mut core, "cold", "bot", t0).unwrap(), other);
    let _ = std::fs::remove_dir_all(&parts.dir);
}

/// The door turns the refusal into the hello's JSON-RPC error and closes,
/// over the socket as over HTTP.
#[tokio::test]
async fn a_hello_with_an_over_long_client_name_is_refused_by_the_door() {
    let daemon = Daemon::start().await;
    let long = "n".repeat(CLIENT_NAME_MAX + 1);
    let mut uds = daemon.connect().await;
    let response = uds
        .hello(serde_json::json!({"client": {"name": long, "version": "1"}}))
        .await;
    assert_eq!(response["error"]["code"], ApiError::INVALID);
    assert_eq!(response["error"]["data"]["field"], "client.name");
    assert!(uds.closed().await, "the door closed after refusing");

    // A name at the cap is accepted.
    let mut uds = daemon.connect().await;
    let response = uds
        .hello(serde_json::json!({
            "client": {"name": "n".repeat(CLIENT_NAME_MAX), "version": "1"}
        }))
        .await;
    assert!(response["error"].is_null(), "{response}");
    daemon.shutdown().await;
}
