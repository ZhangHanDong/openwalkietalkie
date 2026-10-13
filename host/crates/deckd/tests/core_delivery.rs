//! Contract §5 rows 8 and 9: the delivery executor of §3.2/§4.8 end to end
//! through the core — the fast path (a parked `agent.wait` answered from the
//! deck's side clears the card well inside the budget) and the slow one (no
//! waiter → `failed` after 4 s, then a late pickup that agrees with the
//! device again).
mod core_support;

use core_support::{
    agent_of, answer, card_of, card_status, ctx, is_card_clear, live_session, park_wait, poll_card,
    poll_register, uid, Running,
};
use deck_model::{
    CardStatus, Command, Deliver, DeviceOp, EventKind, Request, Scope, SessionId, DELIVERY_BUDGET,
};
use deckd::core::sessions::{MailKind, WaitReply};
use deckd::core::{CoreMsg, WaitResult};
use std::time::{Duration, Instant};

/// Row 8: on a `--no-device` core (the hid channel detached) an `op.answer`
/// on a `poll` card whose owner is parked in `agent.wait` reaches the hid
/// channel as a `card clear` in well under the 100 ms budget of §6.7 —
/// measured with `Instant` across the real core thread.
#[tokio::test]
async fn answer_to_card_clear_under_100_ms() {
    let live = Running::start("clear-budget", 4096);
    let handle = live.handle.clone();
    let (open, _outbound) = live_session(&[Scope::Agent]);
    let owner = handle.open_session(open).await.unwrap();
    let operator = handle
        .open_session(core_support::observer_session(&[Scope::Operate]))
        .await
        .unwrap();

    let agent = agent_of(
        &handle
            .api(
                uid(),
                owner,
                false,
                Request::AgentRegister(poll_register("waiter")),
            )
            .await,
    );
    let card = card_of(
        &handle
            .api(
                uid(),
                owner,
                false,
                Request::CardSet(poll_card(&agent, "deploy to staging?")),
            )
            .await,
    );
    // Only the clear is timed: the register/card ops are already queued.
    live.parts.drain();

    // The wait goes on the core's own ordered channel first, so the core
    // parks it strictly before it sees the answer: no sleep decides this.
    let parked = park_wait(&handle, owner, &agent, Duration::from_secs(30));

    // `op.answer` answers after the delivery (§4.9), and the delivery is
    // this test's own `agent.wait` door, so the two run concurrently —
    // exactly as an operator's connection and an owner's do in the daemon.
    let t0 = Instant::now();
    let answering = tokio::spawn({
        let (handle, agent) = (handle.clone(), agent.clone());
        async move {
            handle
                .api(uid(), operator, false, answer(&agent, card, 0))
                .await
        }
    });
    let result = parked.await.expect("the core answered the wait");
    let WaitResult::Answer {
        card: got,
        answer,
        delivery,
    } = result
    else {
        panic!("the waiter got {result:?}");
    };
    assert_eq!(got, card);
    assert_eq!(answer.choice, Some(0));
    assert_eq!(answer.label.as_deref(), Some("Yes"));
    assert_eq!(delivery.agent, agent);
    assert_eq!(delivery.card, card);

    // The door wrote that response and says so; the clear follows it, and
    // so does the operator's `op.answer` reply, now truthful.
    handle.delivered(delivery.agent, delivery.card, true).await;
    let answered = answering
        .await
        .expect("the op.answer task lives")
        .expect("a custom card is not a guarded operation");
    assert_eq!(answered["delivered"], true, "{answered}");
    assert_eq!(answered["via"], "poll");
    live.op(is_card_clear).await;
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "answer → card clear took {elapsed:?}"
    );

    // The store agrees: the card is gone, not merely marked.
    let reply = handle
        .call(
            ctx(owner, &[Scope::Admin]),
            "agent.get",
            serde_json::json!({"agent": agent}),
        )
        .await;
    assert_eq!(reply.result.unwrap()["card"], serde_json::Value::Null);
}

/// Row 9: nobody is waiting, so the answer waits in the owner's mailbox and
/// the store's 4 s budget fails the card without a `card clear` (the device
/// reverts by itself). A later `agent.wait` picks the answer up and the
/// door's confirmation moves `failed → delivered` with exactly one clear.
/// A second answer before a pickup replaces the mailbox entry (§4.8), so a
/// late waiter and a second decision can never both fire.
#[test]
fn forced_delivery_failure_then_late_poll_pickup() {
    let (mut core, parts) = core_support::build("late-pickup", 4096);
    let t0 = Instant::now();
    let (open, _outbound) = live_session(&[Scope::Agent]);
    let owner = core.sessions.open(open, t0);
    let agent = agent_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::AgentRegister(poll_register("poller")),
        t0,
    ));
    let card = card_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::CardSet(poll_card(&agent, "run migrations?")),
        t0,
    ));
    parts.drain();

    // Answered with nobody parked: the decision waits in the mailbox.
    core.api_call(uid(), SessionId(9), false, answer(&agent, card, 0), t0)
        .unwrap();
    assert!(matches!(
        card_status(&core, &agent),
        Some(CardStatus::Answering {
            choice: Some(0),
            ..
        })
    ));
    assert_eq!(core.sessions.get(owner).unwrap().mailbox.len(), 1);
    assert!(
        !parts.taken().iter().any(is_card_clear),
        "nothing was delivered yet"
    );

    // 4 s of simulated time later the store gives up: `failed`, the
    // `card.failed` event, and still no clear.
    let failed_at = t0 + DELIVERY_BUDGET + Duration::from_millis(1);
    core.tick(failed_at);
    core.publish();
    assert!(matches!(
        card_status(&core, &agent),
        Some(CardStatus::Failed { .. })
    ));
    let (ring, _) = core.bus.replay(0, 500, None);
    assert!(
        ring.iter().any(|e| e.kind == EventKind::CardFailed),
        "card.failed is on the bus"
    );
    assert!(!parts.taken().iter().any(is_card_clear), "no card clear");
    assert_eq!(
        core.sessions.get(owner).unwrap().mailbox.len(),
        1,
        "the answer stays in the mailbox"
    );

    // The late `agent.wait` takes it out …
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    core.wait(
        owner,
        agent.clone(),
        Duration::from_secs(60),
        WaitReply::Result(tx),
        failed_at,
    );
    let WaitResult::Answer {
        card: got,
        answer: got_answer,
        delivery,
    } = rx.try_recv().expect("the mailbox answered at once")
    else {
        panic!("the late waiter did not get the answer");
    };
    assert_eq!((got, got_answer.choice), (card, Some(0)));
    assert!(core.sessions.get(owner).unwrap().mailbox.is_empty());

    // … and the door's confirmation makes store and device agree again.
    core.step(
        Command::Delivered {
            agent: delivery.agent.clone(),
            card: delivery.card,
            ok: true,
        },
        failed_at,
    );
    let clears = parts.taken().iter().filter(|op| is_card_clear(op)).count();
    assert_eq!(clears, 1, "exactly one card clear for the late pickup");
    assert!(
        core.store.snapshot().agents[&agent].card.is_none(),
        "the card is cleared after a late pickup"
    );

    // A second answer before any pickup drops the first mailbox entry.
    let second = card_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::CardSet(poll_card(&agent, "and the smoke tests?")),
        failed_at,
    ));
    parts.drain();
    core.api_call(
        uid(),
        SessionId(9),
        false,
        answer(&agent, second, 0),
        failed_at,
    )
    .unwrap();
    let gave_up = failed_at + DELIVERY_BUDGET + Duration::from_millis(1);
    core.tick(gave_up);
    assert!(matches!(
        card_status(&core, &agent),
        Some(CardStatus::Failed { .. })
    ));
    core.api_call(
        uid(),
        SessionId(9),
        false,
        answer(&agent, second, 1),
        gave_up,
    )
    .unwrap();
    let mailbox = &core.sessions.get(owner).unwrap().mailbox;
    assert_eq!(mailbox.len(), 1, "one decision per (agent, card)");
    let MailKind::Answer {
        answer: queued,
        card: queued_card,
    } = &mailbox[0].kind
    else {
        panic!("the mailbox holds {:?}", mailbox[0].kind);
    };
    assert_eq!((*queued_card, queued.choice), (second, Some(1)));
    assert_eq!(
        core.store.snapshot().agents[&agent]
            .card
            .as_ref()
            .map(|c| c.deliver),
        Some(Deliver::Poll)
    );

    // The waiter that finally comes gets the second decision, once.
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    core.wait(
        owner,
        agent.clone(),
        Duration::from_secs(60),
        WaitReply::Result(tx),
        gave_up,
    );
    let WaitResult::Answer { answer: last, .. } = rx.try_recv().unwrap() else {
        panic!("the waiter did not get the second answer");
    };
    assert_eq!(last.choice, Some(1));
    assert!(core.sessions.get(owner).unwrap().mailbox.is_empty());
    let ops: Vec<DeviceOp> = parts.taken();
    assert!(
        !ops.iter().any(is_card_clear),
        "nothing is cleared until the door confirms"
    );
    let _ = std::fs::remove_dir_all(&parts.dir);
}

/// §3.2: `agent.wait` is a call on an agent the session must own. An agent
/// owned by somebody else is `-32004 conflict{owner}`, an unknown agent and
/// an unknown session are `-32003` — never a parked wait that answers
/// `{"kind":"timeout"}`, which a virtual agent reads as "nothing happened"
/// and retries forever.
#[tokio::test]
async fn agent_wait_on_an_agent_the_session_does_not_own_is_refused() {
    let live = Running::start("wait-ownership", 4096);
    let handle = live.handle.clone();
    let (open, _owner_outbound) = live_session(&[Scope::Agent]);
    let owner = handle.open_session(open).await.unwrap();
    let (open, _other_outbound) = live_session(&[Scope::Agent]);
    let other = handle.open_session(open).await.unwrap();
    let agent = agent_of(
        &handle
            .api(
                uid(),
                owner,
                false,
                Request::AgentRegister(poll_register("owned")),
            )
            .await,
    );

    // Somebody else's agent: the owner is named, as `agent.claim` needs.
    let refused = handle
        .wait(other, agent.clone(), Duration::from_secs(30))
        .await;
    let WaitResult::Refused(error) = refused else {
        panic!("waiting on another session's agent answered {refused:?}");
    };
    assert_eq!(error.code, deck_model::ApiError::CONFLICT);
    assert_eq!(error.data["owner"]["session"], owner.to_string());
    assert_eq!(error.data["owner"]["principal"], "uid:501");

    // An agent that does not exist, and a session that does not exist.
    let unknown = handle
        .wait(
            owner,
            deck_model::AgentId("ag_nope".into()),
            Duration::from_secs(30),
        )
        .await;
    let WaitResult::Refused(error) = unknown else {
        panic!("waiting on an unknown agent answered {unknown:?}");
    };
    assert_eq!(error.code, deck_model::ApiError::NOT_FOUND);
    assert_eq!(error.data["what"], "agent");
    let gone = handle
        .wait(SessionId(9_999), agent.clone(), Duration::from_secs(30))
        .await;
    let WaitResult::Refused(error) = gone else {
        panic!("waiting from an unknown session answered {gone:?}");
    };
    assert_eq!(error.code, deck_model::ApiError::NOT_FOUND);
    assert_eq!(error.data["what"], "session");

    // The door turns the refusal into the JSON-RPC error, not a result.
    let reply = handle
        .call(
            ctx(other, &[Scope::Agent]),
            "agent.wait",
            serde_json::json!({"agent": agent, "timeout_s": 30}),
        )
        .await;
    let error = reply.result.unwrap_err();
    assert_eq!(error.code, deck_model::ApiError::CONFLICT);

    // The owner's own wait still parks and times out as before.
    let quiet = handle.wait(owner, agent, Duration::from_millis(1)).await;
    assert!(matches!(quiet, WaitResult::Timeout), "{quiet:?}");
}

/// Design §12, the "owner disappears with a card pending" row: a `poll`
/// answer nobody picked up outlives the session it was addressed to for
/// `mailbox_ttl_s`, and the session that `agent.claim`s the agent back
/// takes it over — answer with no waiter → `failed` → the session closes →
/// a new session claims → `agent.wait` returns the answer → one `card
/// clear` → `delivered`.
#[test]
fn a_closed_owners_poll_answer_is_claimed_by_the_next_session() {
    let (mut core, parts) = core_support::build("orphan-mail", 4096);
    let t0 = Instant::now();
    let (open, _outbound) = live_session(&[Scope::Agent]);
    let owner = core.sessions.open(open, t0);
    let agent = agent_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::AgentRegister(poll_register("reconnect")),
        t0,
    ));
    let card = card_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::CardSet(poll_card(&agent, "run migrations?")),
        t0,
    ));
    parts.drain();

    // Answered with nobody parked, then the store's 4 s budget fails it.
    core.api_call(uid(), SessionId(9), false, answer(&agent, card, 1), t0)
        .unwrap();
    let failed_at = t0 + DELIVERY_BUDGET + Duration::from_millis(1);
    core.tick(failed_at);
    core.publish();
    assert!(matches!(
        card_status(&core, &agent),
        Some(CardStatus::Failed { .. })
    ));
    assert!(!parts.taken().iter().any(is_card_clear), "no card clear");

    // The owning connection goes away. Before the fix its mailbox died
    // with it and §12's late pickup was unreachable.
    core.on_msg(CoreMsg::CloseSession(owner), failed_at);
    assert!(core.sessions.get(owner).is_none());
    assert_eq!(core.sessions.orphan_len(), 1, "the answer outlives it");
    assert!(
        core.store.snapshot().agents[&agent].card.is_some(),
        "a poll card is kept when its owner goes (§12)"
    );

    // The client reconnects and claims its agent back by key.
    let (open, _next_outbound) = live_session(&[Scope::Agent]);
    let next = core.sessions.open(open, failed_at);
    core.api_call(
        uid(),
        next,
        false,
        Request::AgentClaim {
            key: "b1:reconnect".into(),
        },
        failed_at,
    )
    .expect("an orphaned agent can be claimed");
    assert_eq!(core.sessions.orphan_len(), 0, "claimed with the agent");
    assert_eq!(core.sessions.get(next).unwrap().mailbox.len(), 1);

    // Its `agent.wait` returns the decision the old session never saw.
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    core.wait(
        next,
        agent.clone(),
        Duration::from_secs(60),
        WaitReply::Result(tx),
        failed_at,
    );
    let WaitResult::Answer {
        card: got,
        answer: got_answer,
        delivery,
    } = rx.try_recv().expect("the adopted mail answered at once")
    else {
        panic!("the reconnected owner did not get the answer");
    };
    assert_eq!((got, got_answer.choice), (card, Some(1)));
    assert_eq!(got_answer.label.as_deref(), Some("No"));

    // The door confirms the write: `failed → delivered`, exactly one clear.
    core.step(
        Command::Delivered {
            agent: delivery.agent.clone(),
            card: delivery.card,
            ok: true,
        },
        failed_at,
    );
    core.publish();
    let clears = parts.taken().iter().filter(|op| is_card_clear(op)).count();
    assert_eq!(clears, 1, "exactly one card clear for the late pickup");
    assert!(core.store.snapshot().agents[&agent].card.is_none());
    let (ring, _) = core.bus.replay(0, 500, None);
    assert!(
        ring.iter().any(|e| e.kind == EventKind::CardDelivered),
        "card.delivered is on the bus"
    );
    let _ = std::fs::remove_dir_all(&parts.dir);
}

/// … and it is not kept forever: orphaned mail is pruned on the same beat
/// and by the same `[api] mailbox_ttl_s` as a live session's own.
#[test]
fn orphaned_mail_expires_after_the_mailbox_ttl() {
    let (mut core, parts) = core_support::build("orphan-ttl", 4096);
    let mut config = deckd::config::Config::default();
    config.api.mailbox_ttl_s = 2;
    core.set_config(std::sync::Arc::new(config));
    let t0 = Instant::now();
    let (open, _outbound) = live_session(&[Scope::Agent]);
    let owner = core.sessions.open(open, t0);
    let agent = agent_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::AgentRegister(poll_register("gone")),
        t0,
    ));
    let card = card_of(&core.api_call(
        uid(),
        owner,
        false,
        Request::CardSet(poll_card(&agent, "still there?")),
        t0,
    ));
    core.api_call(uid(), SessionId(9), false, answer(&agent, card, 0), t0)
        .unwrap();
    core.on_msg(CoreMsg::CloseSession(owner), t0);
    assert_eq!(core.sessions.orphan_len(), 1);

    // The first beat is due at once and keeps it (2 s have not passed);
    // the next one, 10 s later, is past the ttl.
    core.tick(t0);
    assert_eq!(core.sessions.orphan_len(), 1);
    core.tick(t0 + deckd::core::HEARTBEAT_EVERY + Duration::from_secs(1));
    assert_eq!(core.sessions.orphan_len(), 0, "orphaned mail expires");
    let _ = std::fs::remove_dir_all(&parts.dir);
}
