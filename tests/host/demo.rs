//! The demo host seen from the runtime: deck events leave `UiState::apply`,
//! `demo_loopback` hands them to `deck::demo::host` instead of the wire, and
//! `tick` delivers the script's follow-ups. The message-level rules live next
//! to the responder (`deck::demo::tests`); this file covers the glue.
use crate::control::{Button, Control, Edge};
use crate::deck::demo::TRANSCRIPT;
use crate::deck::msg::{AgentState, Event, HostMsg, KeyId};
use crate::ui::{UiState, Update};

fn fresh() -> Box<UiState> {
    let mut ui = Box::new(UiState::new());
    ui.reset();
    // Off at power-on; these tests follow a deck that has it on.
    ui.load_demo();
    assert!(ui.deck.demo_active());
    ui
}

/// The runtime's loop body after an input: loop the deck events back.
fn loopback(ui: &mut UiState, out: &mut Update, now_ms: u32) {
    let mut scratch = Box::new(HostMsg::None);
    ui.demo_loopback(now_ms, out, &mut scratch);
    assert!(!out.wake, "the demo never wakes the screen");
    assert!(
        out.deck_events().next().is_none(),
        "looped-back events never reach the wire"
    );
}

fn tick(ui: &mut UiState, now_ms: u32, screen_on: bool) -> Update {
    let mut out = Update::new();
    ui.tick(now_ms, 0, screen_on, &mut out);
    assert!(!out.wake);
    out
}

#[test]
fn ptt_on_the_roster_dictates_to_the_highlighted_agent_through_the_loopback() {
    let mut ui = fresh();
    let agent = ui.agents.highlight;
    assert_eq!(agent, 1);
    let mut out = Update::new();
    ui.apply(
        Control::Button(Button::PushToTalk, Edge::Press),
        1000,
        &mut out,
    );
    assert!(out.deck_events().any(|event| matches!(
        event,
        Event::Ptt {
            act: 1,
            agent: 1,
            ..
        }
    )));
    loopback(&mut ui, &mut out, 1000);
    assert!(ui.global.ptt.is_some() && ui.demo.typing());
    assert!(ui.animating(), "the transcript is being typed");
    let mut now = 1000;
    let mut longest = 0;
    while now < 6000 {
        now += 20;
        tick(&mut ui, now, true);
        let text = ui.deck.transcript.as_str();
        assert!(TRANSCRIPT.starts_with(text));
        longest = longest.max(text.len());
    }
    assert_eq!(longest, TRANSCRIPT.len());
    assert!(!ui.deck.transcript_final);
    let mut out = Update::new();
    ui.apply(
        Control::Button(Button::PushToTalk, Edge::Release),
        6000,
        &mut out,
    );
    assert!(out.deck_events().any(|event| matches!(
        event,
        Event::Ptt {
            act: 0,
            agent: 1,
            ..
        }
    )));
    loopback(&mut ui, &mut out, 6000);
    assert!(ui.deck.transcript_final && ui.global.ptt.is_none());
    assert!(ui.animating(), "the reply is due within a second");
    let reply_before = ui.deck.agent(agent).unwrap().last_reply;
    for at in (6000..9000).step_by(50) {
        tick(&mut ui, at, true);
    }
    let after = ui.deck.agent(agent).unwrap();
    assert_ne!(after.last_reply, reply_before);
    assert_eq!(after.state, AgentState::Working);
    assert!(
        after.has_card == false,
        "orders replace the pending question"
    );
    assert!(
        !ui.animating(),
        "the Ready report 35 s out is polled, not animated"
    );
}

#[test]
fn an_answer_looped_back_resolves_the_card_and_the_script_follows() {
    let mut ui = fresh();
    let mut out = Update::new();
    out.push_deck(Event::Answer {
        agent: 1,
        card: 1,
        choice: 0,
    });
    loopback(&mut ui, &mut out, 2000);
    assert!(ui.deck.card(1).is_none());
    assert!(out.dirty);
    assert!(ui.animating());
    assert!(!tick(&mut ui, 2100, true).dirty);
    assert!(tick(&mut ui, 2400, true).dirty);
    assert_eq!(ui.deck.agent(1).unwrap().state, AgentState::Working);
    assert!(!ui.animating());
    // Ready reachable again: the answered agent reports back later.
    let mut ready = false;
    for at in (2400..40_000).step_by(500) {
        tick(&mut ui, at, true);
        ready |= ui.deck.agent(1).unwrap().state == AgentState::Ready;
    }
    assert!(ready);
    assert!(ui
        .deck
        .card(1)
        .is_some_and(|card| card.kind == crate::deck::msg::CardKind::Ready));
}

#[test]
fn demo_script_does_not_advance_while_off() {
    let mut ui = fresh();
    // The displayed clock keeps its own (dirty) minute steps; not the script's.
    ui.deck.clock_valid = false;
    let mut out = Update::new();
    out.push_deck(Event::Answer {
        agent: 1,
        card: 1,
        choice: 0,
    });
    loopback(&mut ui, &mut out, 1000);
    for at in (1100..200_000).step_by(1000) {
        let out = tick(&mut ui, at, false);
        assert!(!out.dirty);
        assert!(!ui.animating());
    }
    assert_eq!(ui.deck.agent(1).unwrap().state, AgentState::Question);
    assert_eq!(ui.demo.queued(), 3);
    // Lit: the follow-ups resume from where they stopped.
    tick(&mut ui, 200_000, true);
    assert!(ui.animating());
    assert!(tick(&mut ui, 200_400, true).dirty);
    assert_eq!(ui.deck.agent(1).unwrap().state, AgentState::Working);
}

#[test]
fn a_real_hello_ends_the_loopback_and_the_script() {
    let mut ui = fresh();
    let mut out = Update::new();
    out.push_deck(Event::Answer {
        agent: 1,
        card: 1,
        choice: 0,
    });
    loopback(&mut ui, &mut out, 1000);
    assert_eq!(ui.demo.queued(), 3);
    let mut out = Update::new();
    ui.host(
        &HostMsg::Hello {
            v: 1,
            sid: 42,
            minute_of_day: None,
        },
        1100,
        &mut out,
    );
    assert!(!ui.deck.demo_active());
    // Events now go to the wire, and the queued demo follow-ups stay put.
    let mut out = Update::new();
    out.push_deck(Event::Refresh);
    let mut scratch = Box::new(HostMsg::None);
    ui.demo_loopback(1200, &mut out, &mut scratch);
    assert_eq!(out.deck_events().next(), Some(Event::Refresh));
    for at in (1200..60_000).step_by(1000) {
        tick(&mut ui, at, true);
    }
    assert_eq!(ui.demo.queued(), 3);
    assert!(!ui.animating());
}

#[test]
fn omarchy_keys_and_a_full_event_queue_loop_back_in_order() {
    let mut ui = fresh();
    let mut out = Update::new();
    for _ in 0..6 {
        out.push_deck(Event::Key {
            key: KeyId::Right,
            act: 1,
            shift: false,
            face: 2,
        });
    }
    out.push_deck(Event::Refresh);
    out.push_deck(Event::Snooze { agent: 1, card: 1 });
    assert!(!out.overflow);
    loopback(&mut ui, &mut out, 500);
    // 2 -> 3 -> 4 -> 1 -> 2 -> 3 -> 4.
    assert_eq!(ui.deck.workspaces.active, 4);
    assert_ne!(ui.deck.usage.items[0].percent, 62);
    assert!(
        ui.deck.card(1).is_some(),
        "a snooze is the device's business"
    );
    assert_eq!(ui.demo.queued(), 0);
    assert!(!ui.animating());
}
