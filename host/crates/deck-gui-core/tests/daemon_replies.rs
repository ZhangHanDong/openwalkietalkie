//! The GUI's parsers against the daemon's **own** output.
//!
//! Every other test in this crate feeds the parsers a reply somebody wrote by
//! hand from reading the design, which means both sides of the seam can be
//! wrong together and stay green — that is how week 5 shipped a resolver
//! looking for a beacon nothing wrote, and it is the failure mode this file
//! exists to close. The fixtures beside it were captured from a real `deckd`
//! on 2026-09-22 (`deckd ctl` over its own UDS, engines up, deck attached);
//! regenerate them the same way if a reply shape changes on purpose.

use deck_gui_core::voice::{Engines, TestReport};

fn fixture(name: &str) -> serde_json::Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/daemon/");
    let text = std::fs::read_to_string(format!("{path}{name}.json"))
        .unwrap_or_else(|e| panic!("{path}{name}.json: {e}"));
    serde_json::from_str(&text).expect("the captured reply is json")
}

#[test]
fn the_engines_the_daemon_really_reports_are_the_ones_this_screen_parses() {
    let engines = Engines::parse(&fixture("voice_engines"));
    // The daemon answers from a cached `EngineInfo` report, and what it
    // carries is **locales**, not a voice list — §4.9 said `[{id, voices}]`
    // and the engine has no voices to give.
    let stt = engines.stt.first().expect("an stt engine");
    assert_eq!(stt.id, "apple");
    assert!(
        stt.locales.iter().any(|l| l == "en-US"),
        "the locale the selection names must be one the engine reported: {:?}",
        &stt.locales[..stt.locales.len().min(4)]
    );
    assert!(
        stt.locales.len() > 40,
        "a real Apple recogniser reports dozens of locales, not a handful"
    );
    let tts = engines.tts.first().expect("a tts engine");
    assert_eq!(tts.id, "apple");
    assert!(!tts.locales.is_empty());
    assert_eq!(engines.selected_stt.as_deref(), Some("apple"));
    assert_eq!(engines.locale.as_deref(), Some("en-US"));
}

#[test]
fn the_speaker_test_the_daemon_really_answers_keeps_its_own_honesty_note() {
    let report = TestReport::parse(&fixture("voice_test"));
    assert_eq!(report.what, "speaker");
    assert_eq!(report.chime.as_deref(), Some("ready"));
    // The daemon says on the wire that `played` is a queue and not a sound
    // anybody heard. The screen must carry that sentence rather than
    // paraphrase it into a claim the daemon did not make.
    assert!(
        report
            .limits
            .iter()
            .any(|l| l.contains("not that anybody heard it")),
        "the daemon's own limits text is gone: {:?}",
        report.limits
    );
    assert!(
        report
            .limits
            .iter()
            .any(|l| l.contains("system default device") && l.contains("never touched")),
        "the note that no system device or volume moves is gone"
    );
}

#[test]
fn a_config_set_reply_tells_a_live_change_from_one_that_wants_a_restart() {
    let reply: serde_json::Value = fixture("config_set");
    // `hooks.enabled` is a hot key (week 6 put it in `HOT_KEYS`), so the real
    // daemon reports it applied with nothing left to restart for. A screen
    // that read only `changed` could not tell that from a port change.
    assert_eq!(reply["applied"][0], "hooks.enabled");
    assert_eq!(reply["changed"][0], "hooks.enabled");
    assert_eq!(reply["keys"][0], "hooks.enabled");
    assert!(
        reply["restart_needed"]
            .as_array()
            .expect("an array")
            .is_empty(),
        "a hot key must not ask for a restart"
    );
}
