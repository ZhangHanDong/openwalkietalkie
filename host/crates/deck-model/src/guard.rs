//! Guarded operations (design §4.2, §11): an *allow* on a permission or
//! elicitation card grants an agent power, so from an untrusted principal
//! it is refused with `-32002 {need:"trusted"}` and `op.confirm_requested`.
//! Deny and snooze withhold power and are never guarded. The core runs this
//! right before `store.call` (contract §3.2); the reducer itself never
//! looks at trust.
use crate::{Card, OriginKind};

/// A chosen label that starts with one of these (trimmed, case-insensitive)
/// is an allow.
const ALLOW_PREFIXES: [&str; 3] = ["allow", "yes", "approve"];
/// Free text whose first word is one of these is a deny; anything else
/// typed or dictated on a guarded card counts as an allow (conservative:
/// "nope" or "now do it" need trust, "no, don't" does not).
const DENY_WORDS: [&str; 4] = ["no", "deny", "reject", "cancel"];

/// True iff answering `card` with `choice`/`text` would be an allow on a
/// `permission`/`elicitation` card: the chosen label starts with
/// `allow`/`yes`/`approve`, or `text` is given and does not start with
/// `no`/`deny`/`reject`/`cancel`.
pub fn is_guarded_allow(card: &Card, choice: Option<u8>, text: Option<&str>) -> bool {
    if !matches!(
        card.origin.kind,
        OriginKind::Permission | OriginKind::Elicitation
    ) {
        return false;
    }
    let label_allows = choice
        .and_then(|n| card.choices.get(usize::from(n)))
        .is_some_and(|c| label_is_allow(&c.label));
    let text_allows = text.is_some_and(|t| !text_is_deny(t));
    label_allows || text_allows
}

/// The label, trimmed and lowercased, starts with an allow word.
pub fn label_is_allow(label: &str) -> bool {
    let label = label.trim().to_lowercase();
    ALLOW_PREFIXES.iter().any(|p| label.starts_with(p))
}

/// The first word of the text (its leading run of letters, lowercased) is
/// a deny word.
pub fn text_is_deny(text: &str) -> bool {
    let first: String = text
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect();
    DENY_WORDS.contains(&first.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::fixture_agent;
    use crate::{AgentState, Choice};

    /// The fixture's held permission card: `Allow` / `Deny`, plus the
    /// labels given.
    fn card(kind: OriginKind, labels: &[&str]) -> Card {
        let mut card = fixture_agent("ag_1", 1, AgentState::Question).card.unwrap();
        card.origin.kind = kind;
        card.choices = labels
            .iter()
            .map(|l| Choice {
                label: (*l).into(),
                detail: None,
            })
            .collect();
        card.choices_n = labels.len() as u8;
        card
    }

    #[test]
    fn allow_labels_on_permission_and_elicitation_cards_are_guarded() {
        let labels = [
            "Allow",
            "Allow always",
            "Deny",
            "  yes  ",
            "APPROVE it",
            "Maybe",
        ];
        for kind in [OriginKind::Permission, OriginKind::Elicitation] {
            let c = card(kind, &labels);
            assert!(is_guarded_allow(&c, Some(0), None), "{kind:?} Allow");
            assert!(is_guarded_allow(&c, Some(1), None), "{kind:?} Allow always");
            assert!(!is_guarded_allow(&c, Some(2), None), "{kind:?} Deny");
            assert!(is_guarded_allow(&c, Some(3), None), "{kind:?} trimmed yes");
            assert!(is_guarded_allow(&c, Some(4), None), "{kind:?} APPROVE");
            assert!(!is_guarded_allow(&c, Some(5), None), "{kind:?} Maybe");
            assert!(!is_guarded_allow(&c, Some(9), None), "out of range");
            assert!(!is_guarded_allow(&c, None, None), "nothing chosen");
        }
    }

    #[test]
    fn free_text_is_an_allow_unless_it_starts_with_a_deny_word() {
        let c = card(OriginKind::Permission, &["Allow", "Deny"]);
        for allow in [
            "yes",
            "Yes please",
            "run it on staging",
            "",
            "   ",
            "nope",
            "now do it",
            "Denied",
        ] {
            assert!(is_guarded_allow(&c, None, Some(allow)), "{allow:?}");
        }
        for deny in ["no", "No, don't", "  DENY", "reject", "cancel that", "no."] {
            assert!(!is_guarded_allow(&c, None, Some(deny)), "{deny:?}");
        }
        // Either a chosen allow or a non-deny text is enough.
        assert!(is_guarded_allow(&c, Some(1), Some("yes")));
        assert!(is_guarded_allow(&c, Some(0), Some("no")));
        assert!(!is_guarded_allow(&c, Some(1), Some("no")));
    }

    #[test]
    fn custom_question_and_ready_cards_are_never_guarded() {
        for kind in [OriginKind::Custom, OriginKind::Question, OriginKind::Ready] {
            let c = card(kind, &["Allow", "Yes"]);
            assert!(!is_guarded_allow(&c, Some(0), None), "{kind:?}");
            assert!(!is_guarded_allow(&c, Some(1), Some("yes")), "{kind:?}");
            assert!(!is_guarded_allow(&c, None, Some("go ahead")), "{kind:?}");
        }
    }

    #[test]
    fn the_word_helpers_are_case_and_space_insensitive() {
        assert!(label_is_allow(" Allow "));
        assert!(label_is_allow("allowed"));
        assert!(!label_is_allow("disallow"));
        assert!(text_is_deny("No"));
        assert!(text_is_deny("cancel!"));
        assert!(!text_is_deny("nothing"));
        assert!(!text_is_deny(""));
        assert!(!text_is_deny("1 no"));
    }
}
