//! The **Welcome** flow, run end to end over the real reducer (§8.2's
//! Welcome row, contract §6's third exit test).
//!
//! The exit test is "a Welcome run from a fresh account ends with a live
//! transcript", and
//! [`a_welcome_from_a_fresh_account_ends_with_a_live_transcript`] is that run
//! scripted: an empty mirror and a dead socket, then the events a real
//! first run produces in the order it produces them — the service answering
//! its hello, the deck linking, the permissions arriving, the PTT session
//! opening and a `transcript.partial` stream ending in a
//! `transcript.final`. At each step the screen's own words are asserted, so
//! what passes is the flow and not a pile of parsers.
//!
//! What it does **not** prove is stated where it matters: nothing here
//! registers a login item, nothing opens a microphone, and no daemon is
//! running. The live half is a separate run with a deck on the desk.
use deck_gui_core::admin::Info;
use deck_gui_core::voice::{displayed_permissions, BLE_PERMISSIONS};
use deck_gui_core::{ApiEvent, AppEvent, HostState};
use deck_model::{Event, EventKind, Link, Permission, Principal, SessionState, Snapshot, Ts};
use openwalkietalkie_app::present::{self, Launchd, LoginItem, Step};
use serde_json::{json, Value};

const TS: Ts = Ts::from_millis(1_789_760_523_120);

fn event(rev: u64, kind: EventKind, data: Value) -> AppEvent {
    AppEvent::Api(ApiEvent::Event(Event {
        rev,
        ts: TS,
        kind,
        principal: Principal::Device,
        data,
    }))
}

/// A fresh account: no service, no deck, no permissions, nothing folded.
fn fresh() -> HostState {
    HostState::detached()
}

#[test]
fn a_welcome_from_a_fresh_account_ends_with_a_live_transcript() {
    let mut state = fresh();
    let mut step = Step::Service;

    // ---- step 1: the service ------------------------------------------
    assert_eq!(step.counter(), "1 of 3");
    assert!(!state.connected(), "a fresh account has no service");
    assert!(
        present::launchd_line(&Launchd::NotLoaded).contains("not registered"),
        "and launchd has never heard of the agent"
    );
    // The operator presses Start; the daemon comes up and the client's hello
    // is answered with the whole snapshot.
    state.handle_event(AppEvent::Api(ApiEvent::Connected {
        snapshot: Snapshot {
            rev: 1,
            ..Snapshot::default()
        },
    }));
    assert!(state.connected());
    step = step.next().expect("a second step");

    // ---- step 2: the deck and the voice --------------------------------
    assert_eq!(step, Step::Deck);
    assert_eq!(step.counter(), "2 of 3");
    assert_eq!(
        displayed_permissions(state.mirror().voice.transport),
        &BLE_PERMISSIONS
    );
    assert!(step
        .blurb()
        .contains("Pair OpenWalkieTalkie in Bluetooth settings"));
    assert!(step.blurb().contains("custom Bluetooth LE audio protocol"));
    let device = &state.mirror().device;
    assert_eq!(device.link, Link::Offline);
    assert!(!present::deck_ready(device));
    assert!(present::deck_line(device).contains("No deck yet"));
    // `device.link` is what the step waits for, and it is an event.
    state.handle_event(event(
        2,
        EventKind::DeviceLink,
        json!({"link": "connected", "session": "live", "sid": 4, "fw": "0.4.1"}),
    ));
    state.handle_event(event(
        3,
        EventKind::DeviceFaces,
        json!({"list": [
            {"f": 1, "t": "agents", "n": "agents"},
            {"f": 2, "t": "usage", "n": "usage"}
        ]}),
    ));
    let device = &state.mirror().device;
    assert!(present::deck_ready(device), "the deck is linked and live");
    let line = present::deck_line(device);
    assert!(line.contains("0.4.1"), "{line} shows the firmware");
    assert!(line.contains("2 faces"), "{line} shows the strip");
    // The BLE permissions arrive on the stream like everything else.
    state.handle_event(event(
        4,
        EventKind::PermChanged,
        json!({"bluetooth": "granted", "speech": "granted"}),
    ));
    let permissions = state.mirror().voice.permissions;
    assert_eq!(permissions.bluetooth, Permission::Granted);
    assert_eq!(permissions.microphone, Permission::Unknown);
    assert_eq!(permissions.speech, Permission::Granted);
    // Test: the route it took, which is a `voice.route` and not the reply.
    state.handle_event(event(
        5,
        EventKind::VoiceRoute,
        json!({"route": "ble", "transport": "ble", "output_uid": "ble:deck",
               "input_uid": "ble:deck"}),
    ));
    assert_eq!(present::route_word(state.mirror().voice.route), "BLE");
    step = step.next().expect("a third step");

    // ---- step 3: the hooks, and the exit test --------------------------
    assert_eq!(step, Step::Integrations);
    assert_eq!(step.counter(), "3 of 3");
    assert_eq!(step.next(), None, "three steps and no fourth");
    assert!(present::SAY_HELLO.contains("Hold PTT"));
    // Nothing has been said yet, and the screen says exactly that.
    assert_eq!(state.transcript(), None);
    assert_eq!(
        present::transcript_line(state.transcript()),
        present::NO_TRANSCRIPT
    );
    // The press.
    state.handle_event(event(
        6,
        EventKind::VoiceState,
        json!({"state": "listening", "ctx": "talk", "utterance": "ut_1", "principal": "device"}),
    ));
    assert!(state.mirror().voice.ptt.is_some(), "a capture is running");
    // …and the recogniser's stream, partial by partial.
    for (rev, text) in [(7, "hello"), (8, "hello there")] {
        state.handle_event(event(
            rev,
            EventKind::TranscriptPartial,
            json!({"utterance": "ut_1", "text": text}),
        ));
        let line = present::transcript_line(state.transcript());
        assert!(line.starts_with("hearing: "), "{line}");
        assert!(line.contains(text), "{line}");
        assert!(
            !line.contains("final"),
            "{line}: a partial that read as final would say the recogniser had settled"
        );
    }
    // The final, which is what the exit test is about.
    state.handle_event(event(
        9,
        EventKind::TranscriptFinal,
        json!({"utterance": "ut_1", "text": "hello there"}),
    ));
    let transcript = state.transcript().expect("a final transcript");
    assert!(transcript.final_);
    assert_eq!(transcript.text, "hello there");
    let line = present::transcript_line(state.transcript());
    assert_eq!(line, "final: hello there");
    // And the session closing takes the live line away with it: a transcript
    // left on screen after the press is a window claiming to still be
    // listening.
    state.handle_event(event(
        10,
        EventKind::VoiceState,
        json!({"state": "ended", "utterance": "ut_1"}),
    ));
    assert_eq!(state.transcript(), None);
    assert_eq!(
        present::transcript_line(state.transcript()),
        present::NO_TRANSCRIPT
    );
}

#[test]
fn a_matched_intent_is_shown_beside_the_words_it_was_matched_from() {
    // §5.6: the deck answers a card by voice, and the operator has to be
    // able to see *which* choice the words were read as before it lands.
    let mut state = fresh();
    state.handle_event(AppEvent::Api(ApiEvent::Connected {
        snapshot: Snapshot {
            rev: 1,
            ..Snapshot::default()
        },
    }));
    state.handle_event(event(
        2,
        EventKind::TranscriptFinal,
        json!({"utterance": "ut_9", "text": "yes go ahead", "intent": "allow"}),
    ));
    let line = present::transcript_line(state.transcript());
    assert_eq!(line, "final: yes go ahead \u{b7} matched allow");
}

#[test]
fn a_transcript_with_no_words_yet_is_drawn_as_a_capture_and_not_as_an_empty_line() {
    let mut state = fresh();
    state.handle_event(AppEvent::Api(ApiEvent::Connected {
        snapshot: Snapshot {
            rev: 1,
            ..Snapshot::default()
        },
    }));
    state.handle_event(event(
        2,
        EventKind::TranscriptPartial,
        json!({"utterance": "ut_2", "text": ""}),
    ));
    assert_eq!(
        present::transcript_line(state.transcript()),
        "hearing: \u{2026}"
    );
}

#[test]
fn the_deck_step_tells_a_link_apart_from_a_session_that_is_live() {
    // `device.link` carries both, and they are not the same thing: a deck
    // that is linked but whose session is not up yet cannot answer, and a
    // step that called that "connected" would send the operator on to hold
    // PTT at a deck that will not hear them.
    let mut state = fresh();
    state.handle_event(AppEvent::Api(ApiEvent::Connected {
        snapshot: Snapshot {
            rev: 1,
            ..Snapshot::default()
        },
    }));
    state.handle_event(event(
        2,
        EventKind::DeviceLink,
        json!({"link": "connected", "session": "syncing", "sid": 4, "fw": "0.4.1"}),
    ));
    let device = &state.mirror().device;
    assert_eq!(device.session, SessionState::Syncing);
    assert!(!present::deck_ready(device));
    assert!(present::deck_line(device).contains("session"));
    state.handle_event(event(
        3,
        EventKind::DeviceLink,
        json!({"link": "connected", "session": "live", "sid": 4}),
    ));
    assert!(present::deck_ready(&state.mirror().device));
}

#[test]
fn the_doctor_summary_says_what_this_window_can_see_and_names_what_it_cannot() {
    let mut state = fresh();
    state.handle_event(AppEvent::Api(ApiEvent::Connected {
        snapshot: Snapshot {
            rev: 1,
            ..Snapshot::default()
        },
    }));
    let info = Info::parse(&json!({"daemon": "0.1.0", "port": 47201}));
    let lines = present::doctor_lines(
        state.connected(),
        &info,
        Some(&Launchd::Running(Some(1234))),
        &state.mirror().device,
        &state.mirror().service,
    );
    let joined = lines.join("\n");
    assert!(joined.contains("deckd 0.1.0"), "{joined}");
    assert!(joined.contains("pid 1234"), "{joined}");
    assert!(joined.contains("47201"), "{joined}");
    assert!(joined.contains("No deck yet"), "{joined}");
    // The one thing it must be honest about: this is the window's own view
    // and not what the CLI's `doctor` runs.
    assert!(joined.contains("`deckd doctor` checks more"), "{joined}");
    // A degraded service is in it, because that is the first thing the
    // "Not seeing it?" question is usually really about.
    let mut degraded = Snapshot {
        rev: 2,
        ..Snapshot::default()
    };
    degraded.service.degraded = Some("hooks_hijack_risk".into());
    degraded.service.port = 47_201;
    let lines = present::doctor_lines(
        true,
        &Info::parse(&json!({"port": null})),
        Some(&Launchd::NotLoaded),
        &degraded.device,
        &degraded.service,
    );
    let joined = lines.join("\n");
    assert!(joined.contains("hooks_hijack_risk"), "{joined}");
    assert!(joined.contains("nowhere to POST"), "{joined}");
}

#[test]
fn the_login_item_reports_what_the_status_says_and_never_what_register_returned() {
    // Contract §1 M2, measured: `status` 0 -> `register()` **Ok** -> `status`
    // 1 (enabled) -> `unregister()` -> 0; but in a fresh state the first
    // `register()` returned **Ok while `status` was 3 (notFound)**. So the
    // return value proves nothing and only these two statuses mean "on".
    assert_eq!(LoginItem::from_code(0), LoginItem::NotRegistered);
    assert_eq!(LoginItem::from_code(1), LoginItem::Enabled);
    assert_eq!(LoginItem::from_code(2), LoginItem::RequiresApproval);
    assert_eq!(LoginItem::from_code(3), LoginItem::NotFound);
    assert_eq!(LoginItem::from_code(9), LoginItem::Unknown(9));
    for status in [LoginItem::Enabled, LoginItem::RequiresApproval] {
        assert!(status.registered(), "{status:?}");
    }
    for status in [
        LoginItem::NotRegistered,
        LoginItem::NotFound,
        LoginItem::Unknown(9),
    ] {
        assert!(!status.registered(), "{status:?}");
    }
    // The M2 trap in one sentence: `notFound` must read as the failure it
    // is, and never as "on".
    let line = present::login_item_line(LoginItem::NotFound);
    assert!(line.contains("nothing is registered"), "{line}");
    assert!(
        line.contains("whatever the last attempt returned"),
        "{line}"
    );
    assert!(present::login_item_line(LoginItem::Enabled).contains("on."));
    assert!(present::login_item_line(LoginItem::RequiresApproval).contains("Login Items"));
    assert!(present::login_item_line(LoginItem::Unknown(9)).contains("status 9"));
    // Every code round-trips, so a status this build cannot name is still
    // reported as itself.
    for code in [0, 1, 2, 3, 9, -1] {
        assert_eq!(LoginItem::from_code(code).code(), code);
    }
    // The pane that offers the toggle points at the pane macOS owns.
    assert!(present::LOGIN_ITEMS_SETTINGS.starts_with("x-apple.systempreferences:"));
    assert!(present::BLUETOOTH_SETTINGS.ends_with("com.apple.BluetoothSettings"));
}
