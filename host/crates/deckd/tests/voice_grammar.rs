//! §15 week 4, row 6: one grammar answers both CLIs (HOST-CONTRACT-W4 §3
//! B13).
//!
//! The regression this pins is the one that was **measured** on the tree
//! before the fix: `adapters::claude` carried an intent list with
//! `yep|yeah|nope` and `adapters::codex` carried one with `sure`, so the
//! operator holding one deck in front of two terminals got opposite answers
//! from the same word —
//!
//! | said     | Claude (before)      | Codex (before)       |
//! |----------|----------------------|----------------------|
//! | `"sure"` | free text (an order) | **allow**            |
//! | `"yeah"` | **allow**            | free text (an order) |
//!
//! These rows drive each adapter through its real `answer_to_output` rather
//! than calling `deck_model::voice::intent` twice: a shared function that
//! one adapter quietly stopped calling would pass a unit test and still fail
//! the operator. The two adapters sit on different frameworks
//! (`adapters::Adapter` for Claude, `adapters::rt::Adapter` for Codex) and
//! are built from different card types, which is exactly why their grammars
//! drifted apart in the first place.

use deck_model::voice::{intent, talk_intent, Intent, TalkIntent};
use deck_model::{
    Answer, AnswerSource, Card, CardId, CardKind, CardSet, CardStatus, Choice, Deliver, Origin,
    OriginKind, Page, Principal, Ts,
};
use deckd::adapters::rt::Adapter as RtAdapter;
use deckd::adapters::Adapter as HookAdapter;

/// Whether an adapter turned this spoken text into an allow, a deny, or a
/// deny that carries the operator's own words back to the model.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Allow,
    Deny,
    /// A deny whose `message` is the transcript: "that was an instruction".
    Instruction(String),
}

fn spoken(text: &str) -> Answer {
    Answer {
        choice: None,
        label: None,
        text: Some(text.to_string()),
        source: AnswerSource::Ptt,
        principal: Principal::Device,
        session: None,
        utterance: None,
    }
}

fn choices(labels: &[&str]) -> Vec<Choice> {
    labels
        .iter()
        .map(|label| Choice {
            label: (*label).to_string(),
            detail: None,
        })
        .collect()
}

fn classify(decision: &serde_json::Value, denied: &str) -> Decision {
    match decision["behavior"].as_str() {
        Some("allow") => Decision::Allow,
        Some("deny") => {
            let message = decision["message"].as_str().unwrap_or_default();
            if message == denied {
                Decision::Deny
            } else {
                Decision::Instruction(message.to_string())
            }
        }
        other => panic!("neither allow nor deny: {other:?} in {decision}"),
    }
}

/// Claude's real path: the `PermissionRequest` card it built, answered by
/// voice.
fn claude(text: &str) -> Decision {
    use deckd::adapters::claude::{Claude, ALLOW, ALLOW_ALWAYS, DENIED, DENY};
    let note = serde_json::json!({"event": "PermissionRequest", "suggestions": null});
    let cmd = CardSet {
        agent: deck_model::AgentId("ag_1".into()),
        id: None,
        kind: CardKind::Q,
        text: "Bash: ls".into(),
        choices: choices(&[ALLOW, ALLOW_ALWAYS, DENY]),
        rec: Some(0),
        free_text: true,
        deliver: Deliver::Hold,
        timeout_s: Some(120),
        origin: Origin {
            kind: OriginKind::Permission,
            tool: Some("Bash".into()),
            request_id: None,
            group: None,
        },
    };
    let out = Claude::default()
        .answer_to_output(&note, &cmd, Some(&spoken(text)))
        .json();
    classify(&out["hookSpecificOutput"]["decision"], DENIED)
}

/// Codex's real path: the same spoken word on its own permission card.
fn codex(text: &str) -> Decision {
    use deckd::adapters::codex::{Codex, ALLOW, DENIED, DENY};
    let card = Card {
        id: CardId(1),
        kind: CardKind::Q,
        status: CardStatus::Pending,
        deliver: Deliver::Hold,
        origin: Origin {
            kind: OriginKind::Permission,
            tool: Some("Bash".into()),
            request_id: None,
            group: None,
        },
        text: Some("Bash: ls".into()),
        choices: choices(&[ALLOW, DENY]),
        choices_n: 2,
        choice_map: Vec::new(),
        rec: Some(0),
        free_text: true,
        page: Page { index: 0, count: 1 },
        created: Ts(0),
        expires: None,
        redacted: false,
    };
    let out = Codex::default()
        .answer_to_output(&card, Some(&spoken(text)))
        .value();
    classify(&out["hookSpecificOutput"]["decision"], DENIED)
}

#[test]
fn one_grammar_answers_both_clis() {
    // The measured divergence, both directions, plus the words §5.6 names.
    for (text, want) in [
        ("sure", Decision::Allow),
        ("yeah", Decision::Allow),
        ("yep", Decision::Allow),
        ("nope", Decision::Deny),
        ("go ahead", Decision::Allow),
        ("stop", Decision::Deny),
        // Punctuation and case come from the dictation, not the operator.
        ("Sure!", Decision::Allow),
        ("Go ahead.", Decision::Allow),
        ("DON'T", Decision::Deny),
    ] {
        let (a, b) = (claude(text), codex(text));
        assert_eq!(a, b, "{text:?}: Claude said {a:?}, Codex said {b:?}");
        assert_eq!(a, want, "{text:?}");
    }
    // Anything the grammar does not know is an instruction both CLIs get in
    // full — that is the whole point of the `Say` arm.
    for text in ["run the tests first", "report", "maybe"] {
        let a = claude(text);
        let b = codex(text);
        assert_eq!(a, b, "{text:?}");
        assert!(
            matches!(&a, Decision::Instruction(m) if m == text),
            "{text:?} → {a:?}"
        );
    }
}

#[test]
fn the_talk_verbs_do_what_the_design_says() {
    // §5.6 `ctx:talk`: **exactly** these words, nothing looser.
    for report in ["report", "status", "Status.", "  REPORT  "] {
        assert_eq!(talk_intent(report), TalkIntent::Report, "{report}");
    }
    for quiet in ["stop", "quiet", "Quiet!"] {
        assert_eq!(talk_intent(quiet), TalkIntent::Stop, "{quiet}");
    }
    for order in [
        "report on the build",
        "status of the deploy",
        "stop the server",
        "",
        "yes",
    ] {
        assert_eq!(talk_intent(order), TalkIntent::Order, "{order}");
    }
    // `stop` is a deny when answering a card and a barge-in when talking:
    // the two grammars are keyed by `ctx`, not by the word.
    assert_eq!(intent("stop"), Intent::Deny);
    assert_eq!(talk_intent("stop"), TalkIntent::Stop);
}

/// `deck_model::guard` deliberately keeps its own, wider pair and is **not**
/// folded into the grammar (B13 called it "a third list"; it answers a
/// different question).
///
/// The grammar asks what the operator meant. The guard asks whether the
/// answer could possibly grant power, so that an **unrecognised** word still
/// demands trust — `"nope"` is a deny to the grammar and a possible allow to
/// the guard, and that asymmetry is the safe direction on both sides.
#[test]
fn the_trust_guard_stays_wider_than_the_grammar() {
    use deck_model::guard::text_is_deny;
    assert_eq!(intent("nope"), Intent::Deny);
    assert!(
        !text_is_deny("nope"),
        "the guard must still demand trust for a word it does not know"
    );
    // Where they do agree, they agree.
    for word in ["no", "deny", "reject"] {
        assert_eq!(intent(word), Intent::Deny, "{word}");
        assert!(text_is_deny(word), "{word}");
    }
}
