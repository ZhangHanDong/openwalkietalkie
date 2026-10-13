//! What the **Voice** screen shows, driven through the real reducer and the
//! real reply shapes (§8.2's Voice row).
//!
//! The row it is built to prove is the honesty one. Three of this screen's
//! four Test buttons run nothing, `perm.request` cannot show a prompt until
//! week 7's signed bundle, and an RMS of exactly zero is what a dead
//! microphone and a warming SCO link both report — so most of what follows
//! asserts that the screen says so, in the daemon's own words, and never
//! turns a reading into a verdict it cannot support.
use deck_gui_core::voice::{
    displayed_permissions, Engines, Prompt, TestReport, BLE_PERMISSIONS, PERMISSIONS, TESTS,
};
use deck_gui_core::{ApiEvent, AppEvent, HostState, UiAction};
use deck_model::{
    Event, EventKind, Permission, Principal, Route, Snapshot, Tone, Ts, VoiceTransport,
};
use openwalkietalkie_app::present;
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

fn live() -> HostState {
    let snapshot = Snapshot {
        rev: 10,
        ..Snapshot::default()
    };
    let mut state = HostState::detached();
    state.handle_event(AppEvent::Api(ApiEvent::Connected { snapshot }));
    state
}

#[test]
fn an_explicit_legacy_transport_keeps_its_route_diagram() {
    let mut state = live();
    assert_eq!(state.mirror().voice.route, Route::None);
    state.handle_event(event(
        11,
        EventKind::VoiceRoute,
        json!({"route": "a2dp", "output_uid": "deck-out", "input_uid": "deck-in",
               "transport": "coreaudio"}),
    ));
    assert_eq!(state.mirror().voice.route, Route::A2dp);
    assert_eq!(present::route_word(Route::A2dp), "A2DP");
    assert_eq!(present::route_tone(Route::A2dp), Tone::Green);
    assert_eq!(
        present::uid_line("output", state.mirror().voice.output_uid.as_deref()),
        "output: deck-out"
    );
    // The press: ours, then the 2.3 s back to A2DP.
    state.handle_event(event(
        12,
        EventKind::VoiceRoute,
        json!({"route": "sco_ours"}),
    ));
    assert_eq!(state.mirror().voice.route, Route::ScoOurs);
    state.handle_event(event(
        13,
        EventKind::VoiceRoute,
        json!({"route": "reopening"}),
    ));
    assert_eq!(state.mirror().voice.route, Route::Reopening);
    // §8.2 names five states and the machine has seven. `Reopening` is
    // where it spends the 2.3 s after *every* press and `SystemDefault` is
    // the sound not going to the deck at all: a diagram that could not draw
    // them would go blank exactly when it was needed.
    assert_eq!(present::ROUTE_STEPS.len(), 7);
    for route in present::ROUTE_STEPS {
        assert!(!present::route_word(route).is_empty());
        assert!(
            present::route_help(route, Some(VoiceTransport::Coreaudio)).len() > 40,
            "{route:?}"
        );
    }
    assert!(present::ROUTE_STEPS.contains(&Route::Reopening));
    assert!(present::ROUTE_STEPS.contains(&Route::SystemDefault));
    assert_eq!(
        present::route_steps(state.mirror().voice.transport),
        &present::ROUTE_STEPS[..]
    );
    // Only the one the machine cannot leave on its own is red.
    assert_eq!(present::route_tone(Route::ScoStuck), Tone::Red);
    assert_eq!(present::route_tone(Route::ScoExternal), Tone::Amber);
    // The screen asks for none of this: it is all events.
    assert_eq!(UiAction::VoiceEngines.request().0, "voice.engines");
}

/// Over the BLE voice link the diagram is that transport's own: its settled
/// state and our capture are named for it, nothing can flip the rate under
/// us so there is no `ScoExternal`, and the words that only make sense for a
/// Classic deck (A2DP, the SCO teardown) are not used.
#[test]
fn over_ble_the_diagram_and_its_words_are_the_ble_transports() {
    let mut state = live();
    state.handle_event(event(
        11,
        EventKind::VoiceRoute,
        json!({"route": "ble", "output_uid": "ble:FAKE", "input_uid": "ble:FAKE",
               "transport": "ble"}),
    ));
    let voice = &state.mirror().voice;
    assert_eq!(voice.route, Route::Ble);
    assert_eq!(voice.transport, Some(VoiceTransport::Ble));
    let steps = present::route_steps(voice.transport);
    assert_eq!(steps, &present::BLE_ROUTE_STEPS[..]);
    assert!(steps.contains(&Route::BleOurs));
    assert!(!steps.contains(&Route::A2dp) && !steps.contains(&Route::ScoExternal));
    assert_eq!(present::route_word(Route::Ble), "BLE");
    assert_eq!(present::route_tone(Route::Ble), Tone::Green);
    assert_eq!(present::route_word(Route::BleOurs), "BLE (ours)");
    assert_eq!(present::route_tone(Route::BleOurs), Tone::Blue);
    for route in steps {
        let help = present::route_help(*route, voice.transport);
        assert!(help.len() > 40, "{route:?}");
        assert!(
            !help.contains("A2DP") && !help.contains("SCO") && !help.contains("44.1"),
            "{route:?}: {help}"
        );
    }
}

#[test]
fn before_the_transport_report_the_current_deck_shows_ble_setup_and_mac_fallback_permissions() {
    let state = live();
    assert_eq!(state.mirror().voice.transport, None);
    assert_eq!(present::route_steps(None), &present::BLE_ROUTE_STEPS[..]);
    assert_eq!(displayed_permissions(None), &BLE_PERMISSIONS);
    assert_eq!(
        displayed_permissions(Some(VoiceTransport::Ble)),
        &BLE_PERMISSIONS
    );
    assert_eq!(
        present::permission_title("microphone", None),
        "Mac fallback microphone"
    );
    assert_eq!(
        present::permission_title("microphone", Some(VoiceTransport::Coreaudio)),
        "Microphone"
    );
    assert!(present::permission_note("microphone", None)
        .unwrap()
        .contains("only when capture falls back"));
    assert!(present::permission_note("bluetooth", None)
        .unwrap()
        .contains("custom Bluetooth LE protocol"));
    assert!(present::transport_line(None).contains("waiting"));
    assert!(present::transport_line(Some(VoiceTransport::Ble)).contains("custom audio protocol"));
    for route in present::route_steps(None) {
        let help = present::route_help(*route, None);
        assert!(!help.contains("A2DP") && !help.contains("SCO") && !help.contains("44.1"));
    }
}

#[test]
fn a_uid_the_daemon_has_not_selected_is_said_out_loud_and_never_drawn_as_an_empty_line() {
    assert_eq!(present::uid_line("input", None), "input: none selected");
    assert_eq!(
        present::uid_line("input", Some("BuiltInMicrophoneDevice")),
        "input: BuiltInMicrophoneDevice"
    );
}

#[test]
fn the_meter_reads_what_the_capture_reported_and_never_a_verdict_about_the_microphone() {
    let mut state = live();
    assert_eq!(state.level(), None);
    assert!(present::meter_line(None, TS).contains("No capture running"));
    // §7.1: `voice.level{rms}`, per buffer and opt-in on the bus.
    state.handle_event(event(11, EventKind::VoiceLevel, json!({"rms": 0.05})));
    let level = state.level().expect("a level");
    assert!((level.rms - 0.05).abs() < f32::EPSILON);
    assert!(present::meter_line(Some(level), TS).contains("dBFS"));
    // V2: a dead microphone and a warming SCO link both report exact digital
    // zero for about a second. The meter says what it saw and nothing more.
    state.handle_event(event(12, EventKind::VoiceLevel, json!({"rms": 0.0})));
    let silent = state.level().expect("a level");
    let line = present::meter_line(Some(silent), TS);
    assert!(line.contains("silence"), "{line}");
    // It names the ambiguity rather than resolving it: "a warming link and a
    // dead microphone read the same here" is the whole of what zero means.
    assert!(line.contains("read the same"), "{line}");
    assert!(
        !line.contains("no microphone") && !line.contains("unavailable"),
        "{line} draws a conclusion the number cannot support"
    );
    // The scale is dBFS: a linear meter spends its whole range on the top
    // 6 dB and shows speech as a twitch at the bottom.
    assert_eq!(present::meter_level(0.0), 0.0);
    assert_eq!(present::meter_level(1.0), 1.0);
    let quiet = present::meter_level(0.01); // -40 dBFS
    assert!((quiet - 1.0 / 3.0).abs() < 0.01, "{quiet}");
    assert!(present::meter_level(0.1) > present::meter_level(0.01));
    // Below the floor and nonsense both clamp rather than panic.
    assert_eq!(present::meter_level(0.0000001), 0.0);
    assert_eq!(present::meter_level(f32::NAN), 0.0);
    assert_eq!(present::meter_level(-1.0), 0.0);
}

#[test]
fn the_meter_goes_idle_when_the_last_buffer_is_old_and_when_the_session_ends() {
    let mut state = live();
    state.handle_event(event(11, EventKind::VoiceLevel, json!({"rms": 0.2})));
    let level = state.level().expect("a level");
    // Still the live one a moment later, and not a second later.
    assert!(present::fresh(level.at, Ts::from_millis(TS.millis() + 500)));
    assert!(!present::fresh(
        level.at,
        Ts::from_millis(TS.millis() + 2_000)
    ));
    assert!(
        present::meter_line(Some(level), Ts::from_millis(TS.millis() + 2_000))
            .contains("No capture running")
    );
    // And the session ending clears it outright: a meter still showing the
    // last buffer would be drawing a live level for a shut microphone.
    state.handle_event(event(
        12,
        EventKind::VoiceState,
        json!({"state": "ended", "utterance": "ut_1"}),
    ));
    assert_eq!(state.level(), None);
}

#[test]
fn the_engine_badges_report_locales_and_say_where_the_voice_pool_actually_lives() {
    let engines = Engines::parse(&json!({
        "stt": [{"id": "apple", "on_device": true, "locales": ["en-US", "en-GB", "de-DE"]}],
        "tts": [{"id": "apple", "on_device": true, "locales": ["en-US"]}],
        "selected": {"stt": "apple", "tts": "apple", "locale": "en-US"}
    }));
    let stt = engines.stt().expect("an stt engine");
    assert_eq!(present::engine_badge(stt), "apple \u{b7} on-device");
    let line = present::locales_line(&stt.locales, engines.locale.as_deref());
    assert!(line.contains("en-US"));
    assert!(line.contains("using en-US"));
    // A configured locale the engine never reported is the first place
    // §7.6's `-32009` shows up, so it is called out rather than hidden.
    let drifted = present::locales_line(&stt.locales, Some("fr-FR"));
    assert!(
        drifted.contains("configured fr-FR is not in this list"),
        "{drifted}"
    );
    assert_eq!(present::locales_line(&[], None), "no locales reported");
    // A server engine is not an on-device one and the badge says which.
    let server = deck_gui_core::voice::Engine {
        id: "whisper".into(),
        on_device: false,
        locales: vec!["en-US".into()],
    };
    assert_eq!(present::engine_badge(&server), "whisper \u{b7} server");
    // No engine at all is a state, not a failure, and the sentence says
    // which states produce it.
    let missing = present::engines_missing_line("-32013 degraded: voice_unavailable");
    assert!(missing.contains("--no-device"));
    assert!(missing.contains("voice_unavailable"));
}

#[test]
fn a_permission_is_shown_with_the_one_thing_that_can_honestly_be_done_about_it() {
    let mut state = live();
    assert_eq!(
        state.mirror().voice.permissions.microphone,
        Permission::Unknown
    );
    state.handle_event(event(
        11,
        EventKind::PermChanged,
        json!({"microphone": "granted", "speech": "denied"}),
    ));
    let permissions = state.mirror().voice.permissions;
    assert_eq!(permissions.microphone, Permission::Granted);
    assert_eq!(permissions.speech, Permission::Denied);
    assert_eq!(present::permission_word(Permission::Granted), "granted");
    assert_eq!(present::permission_tone(Permission::Granted), Tone::Green);
    assert_eq!(present::permission_tone(Permission::Denied), Tone::Red);
    // §7.8: macOS will not prompt twice, so a denied permission's only
    // honest offer is System Settings.
    assert!(present::permission_help(Permission::Denied).contains("will not prompt twice"));
    // `unknown` is not `not_determined`, and the help says which is which.
    assert!(present::permission_help(Permission::Unknown).contains("not the same"));
    assert!(present::permission_help(Permission::NotDetermined).contains("has not been asked"));
    // §10.4's standing caveat, wherever a permission is drawn.
    assert!(present::PERMISSION_CAVEAT.contains("responsible process"));
    assert_eq!(PERMISSIONS, ["microphone", "speech"]);
    for which in PERMISSIONS {
        assert_eq!(
            UiAction::PermRequest {
                which: which.into()
            }
            .request()
            .0,
            "perm.request"
        );
    }
}

#[test]
fn a_grant_button_that_cannot_prompt_says_so_in_the_daemons_words_and_is_not_offered_twice() {
    // §10.4: `perm.request` is stubbed at the prompt until the signed
    // bundle of §10.1, and the reason travels on the wire in `limits`.
    let reply = json!({
        "which": "microphone",
        "status": "prompt_not_shown",
        "limits": [
            "the prompt is not shown: it needs the signed bundle of \u{a7}10.1, because TCC keys \
             a grant to the client's code-signing identifier",
            "`prompt_not_shown` is \u{a7}7.8's own answer for a prompt that did not surface"
        ]
    });
    let prompt = Prompt::parse(&reply);
    assert!(prompt.not_shown());
    let line = present::prompt_line(&prompt);
    assert!(line.contains("nothing changed"), "{line}");
    assert!(
        !line.contains("failed") && !line.contains("error"),
        "{line}: `prompt_not_shown` is an answer, not a failure"
    );
    // The screen shows the daemon's own sentences underneath, verbatim —
    // and those are what stands where the button was.
    assert_eq!(prompt.limits.len(), 2);
    assert!(prompt.limits[0].contains("signed bundle"));
    // A daemon that one day does prompt is a different case entirely.
    let granted = Prompt::parse(&json!({"which": "speech", "status": "granted"}));
    assert!(!granted.not_shown());
    assert_eq!(present::prompt_line(&granted), "speech: granted");
}

#[test]
fn the_speaker_test_claims_a_queue_and_never_a_sound_anybody_heard() {
    let queued = "`played` means the chime was queued on the audio thread, not that anybody heard \
                  it";
    let report = TestReport::parse(&json!({
        "what": "speaker",
        "route": "a2dp",
        "output_uid": "deck-out",
        "played": true,
        "chime": "ready",
        "limits": [queued]
    }));
    let headline = present::test_headline(&report);
    assert_eq!(headline, "Chime queued on the audio thread");
    assert!(
        !headline.contains("played") && !headline.contains("heard"),
        "{headline}"
    );
    let lines = present::test_lines(&report);
    assert!(lines.iter().any(|l| l == "route: a2dp"));
    assert!(lines.iter().any(|l| l == "chime: ready"));
    // The caveat is the daemon's and is rendered as it arrived.
    assert_eq!(report.limits, vec![queued]);
    // A refused queue is the honest failure.
    let refused = TestReport::parse(&json!({"what": "speaker", "played": false}));
    assert_eq!(
        present::test_headline(&refused),
        "The audio thread refused the chime"
    );
    // And the standing note over the buttons is true before any of them is
    // pressed: §7 never touches the system default device or the volume.
    assert!(present::TEST_NOTE.contains("110 ms"));
    assert!(present::TEST_NOTE.contains("system default device"));
    assert_eq!(
        UiAction::VoiceTest {
            what: "speaker".into()
        }
        .request()
        .1["what"],
        "speaker"
    );
}

#[test]
fn the_other_three_tests_report_and_the_screen_never_calls_one_of_them_an_exercise() {
    let mic = TestReport::parse(&json!({
        "what": "mic",
        "route": "none",
        "input_uid": "deck-in",
        "mic_warm": "card_pending",
        "mic_warm_effective": "never",
        "transport": "ble",
        "limits": ["the microphone is not opened: a capture is a PTT session with an owner"]
    }));
    assert_eq!(
        present::test_headline(&mic),
        "Input as the route machine sees it"
    );
    assert!(present::test_lines(&mic)
        .iter()
        .any(|l| l == "mic_warm: card_pending"));
    let lines = present::test_lines(&mic);
    assert!(lines.iter().any(|l| l == "transport: ble"));
    assert!(lines.iter().any(|l| l == "mic_warm_effective: never"));
    assert!(!mic.made_a_sound());
    assert!(mic.limits[0].contains("not opened"));
    for (what, expected) in [
        ("stt", "The recogniser, as it reported itself at start-up"),
        ("tts", "The synthesiser, as it reported itself at start-up"),
    ] {
        let report = TestReport::parse(&json!({
            "what": what,
            "engine": {"id": "apple", "on_device": true, "locales": ["en-US"]},
            "limits": ["this is what the engine reported when it came up, not a run"]
        }));
        assert_eq!(present::test_headline(&report), expected);
        assert!(present::test_lines(&report)
            .iter()
            .any(|l| l.starts_with("engine: apple")));
        assert!(!report.made_a_sound());
    }
    assert_eq!(TESTS.len(), 4);
    assert_eq!(TESTS[0], "speaker", "the only one that makes a sound");
}

#[test]
fn a_daemon_without_the_two_voice_methods_is_told_apart_from_a_call_that_failed() {
    // Contract §3: `voice.engines`/`voice.test` were `-32013` until this
    // week. A screen that reported "tested" on a call the daemon never
    // answered would be claiming something that did not happen.
    for method in ["voice.engines", "voice.test", "perm.request"] {
        let line = present::missing_method_line(method);
        assert!(line.contains(method));
        assert!(line.contains("nothing happened"));
    }
}
