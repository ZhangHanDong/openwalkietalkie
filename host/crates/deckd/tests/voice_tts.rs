//! §15 week 4, exit row 10 (HOST-CONTRACT-W4 §5): the TTS text sanitiser
//! cuts at a sentence and at a character boundary.
//!
//! What reaches this function is a **model's reply**, which is the most
//! hostile input in the system for a speech synthesiser: markdown emphasis,
//! fenced code, ANSI from a tool that echoed a coloured prompt, and as many
//! paragraphs as the model felt like. Read out verbatim that is a minute of
//! "star star done star star, backtick cargo test backtick" while the
//! operator holds a deck and waits.
//!
//! Every row here is a table of `(input, spoken)` rather than a walk
//! through the implementation: the contract is what the operator hears.

use deck_model::sanitize::speech;

/// The §9 default, and the number the rows below are written against.
const MAX: usize = 120;

#[test]
fn the_sanitiser_cuts_at_a_sentence_and_a_char_boundary() {
    // ── ANSI, which arrives whenever a tool echoed a coloured prompt ─────
    for (raw, spoken) in [
        ("\u{1b}[31mBuild failed.\u{1b}[0m", "Build failed."),
        ("\u{1b}]0;title\u{7}All good.", "All good."),
        ("plain", "plain"),
    ] {
        assert_eq!(speech(raw, MAX), spoken, "{raw:?}");
    }

    // ── markdown, which is most of what a model writes ───────────────────
    for (raw, spoken) in [
        ("**Done.**", "Done."),
        ("__Done.__", "Done."),
        ("*Done.*", "Done."),
        ("~~Done.~~", "Done."),
        ("`cargo test` passed.", "cargo test passed."),
        ("## Summary", "Summary"),
        ("> quoted line", "quoted line"),
        ("- first item", "first item"),
        ("1. first item", "first item"),
        (
            "See [the log](https://example.test/x) for more.",
            "See the log for more.",
        ),
        ("![a chart](chart.png) is attached.", "a chart is attached."),
        // …without eating the things that are not markdown.
        ("snake_case_name is fine.", "snake_case_name is fine."),
        ("2 * 3 = 6.", "2 * 3 = 6."),
        ("Use \\*literal\\* stars.", "Use *literal* stars."),
    ] {
        assert_eq!(speech(raw, MAX), spoken, "{raw:?}");
    }

    // ── code fences fold to their first line, as §5.6 already does ───────
    assert_eq!(
        speech("Run this:\n```bash\ncargo test --all\n```\n", MAX),
        "Run this: code: cargo test --all"
    );

    // ── the first sentence, and only the first ───────────────────────────
    assert_eq!(
        speech(
            "Tests pass. I also refactored the parser. Shall I push?",
            MAX
        ),
        "Tests pass."
    );
    assert_eq!(speech("Ready?  Next thing.", MAX), "Ready?");
    assert_eq!(speech("Stop! Now.", MAX), "Stop!");
    assert_eq!(speech("no terminator at all", MAX), "no terminator at all");
    // A decimal point is not the end of a sentence, and a version number is
    // not three of them. This is the one that bites: "the build took 3.5
    // seconds" spoken as "the build took 3." is worse than saying nothing.
    assert_eq!(
        speech("The build took 3.5 s on v1.2.3 of the toolchain.", MAX),
        "The build took 3.5 s on v1.2.3 of the toolchain."
    );
    // …and an abbreviation followed by a space is still a cut, because
    // nothing here can tell "e.g." from a sentence and guessing wrong in
    // the other direction reads out the whole reply.
    assert_eq!(speech("Line one.\nLine two.", MAX), "Line one.");

    // ── the cap, in characters, on a character boundary ──────────────────
    let long = "x".repeat(500);
    assert_eq!(speech(&long, MAX).chars().count(), MAX);
    assert_eq!(speech(&long, 7), "xxxxxxx");
    assert_eq!(speech(&long, 0), "");

    // Multi-byte safety: a budget that lands mid-character keeps whole
    // characters and never emits a replacement glyph. 120 characters of
    // Japanese is 360 bytes, and cutting by bytes would read out a third
    // of the sentence.
    let jp = "コーデックスは素晴らしい".repeat(40);
    let out = speech(&jp, MAX);
    assert_eq!(out.chars().count(), MAX);
    assert_eq!(out.len(), MAX * 3, "three bytes a character, all of them");
    assert!(jp.starts_with(&out));
    assert!(!out.contains('\u{fffd}'));
    // Emoji are four bytes and one character, including outside the BMP.
    let emoji = "🛠️🚀🧪".repeat(30);
    let out = speech(&emoji, 5);
    assert_eq!(out.chars().count(), 5);
    assert!(emoji.starts_with(&out));

    // ── and the whole pipeline at once, which is what actually arrives ───
    let reply = "\u{1b}[32m## Result\u{1b}[0m\n\n**All 621 tests pass.** \
                 The `deck-model` crate is unchanged.\n\n```rust\nfn main() {}\n```";
    assert_eq!(speech(reply, MAX), "Result All 621 tests pass.");

    // Nothing but markup is nothing to say — and says so with an empty
    // string rather than reading the punctuation out.
    assert_eq!(speech("**``**", MAX), "");
    assert_eq!(speech("   \n\t ", MAX), "");
    assert_eq!(speech("\u{1b}[2J\u{1b}[H", MAX), "");
}
