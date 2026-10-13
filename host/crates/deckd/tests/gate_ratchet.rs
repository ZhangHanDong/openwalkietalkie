//! The gate's test ratchet and the counts `docs/host-service.md` publishes are
//! one fact written twice: `tools/host-test.sh` refuses a workspace that runs
//! fewer than `OWT_MIN_HOST_WS_TESTS` tests, and the design's status line tells
//! a reader what that number is made of. Nothing but this row keeps the two in
//! step, and they have drifted apart before (the ratchet sat two tests below
//! the real count, which is two tests that could have been deleted unnoticed).
//!
//! Raising the ratchet after adding tests therefore means editing both places,
//! and this row says so by name when only one of them moved. The gate itself
//! checks the third leg — that the workspace really runs that many.

use std::path::PathBuf;

/// The repository root: this crate lives at `<repo>/host/crates/deckd`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the crate sits three levels under the repository root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The run of ASCII digits immediately after `marker`.
fn number_after(text: &str, what: &str, marker: &str) -> u32 {
    let at = text
        .find(marker)
        .unwrap_or_else(|| panic!("{what} no longer contains {marker:?}"));
    text[at + marker.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("{what}: {marker:?} is not followed by a number"))
}

/// The run of ASCII digits immediately before `marker`.
fn number_before(text: &str, what: &str, marker: &str) -> u32 {
    let at = text
        .find(marker)
        .unwrap_or_else(|| panic!("{what} no longer contains {marker:?}"));
    let head = &text[..at];
    let start = head.len() - head.chars().rev().take_while(char::is_ascii_digit).count();
    head[start..]
        .parse()
        .unwrap_or_else(|_| panic!("{what}: {marker:?} is not preceded by a number"))
}

#[test]
fn the_test_ratchet_and_the_published_counts_are_the_same_number() {
    let script = read("tools/host-test.sh");
    let ratchet = number_after(
        &script,
        "tools/host-test.sh",
        "min_tests=${OWT_MIN_HOST_WS_TESTS:-",
    );

    // Scope the search to the gate's own sentence: the file names `deckd` and
    // plenty of other numbers elsewhere.
    let docs = read("docs/host-service.md");
    let at = docs
        .find("the gate is `tools/host-test.sh`")
        .expect("docs/host-service.md still introduces the gate");
    let sentence = &docs[at..];
    let what = "docs/host-service.md";

    let published = number_before(sentence, what, " tests — ");
    assert_eq!(
        published, ratchet,
        "docs/host-service.md publishes {published} tests but the ratchet in \
         tools/host-test.sh is {ratchet}; raising one means raising the other"
    );

    // Every crate the gate counts is broken out in that sentence, and the
    // parts make the total.
    let crates = script
        .lines()
        .find_map(|l| l.strip_prefix("crates=(").and_then(|l| l.strip_suffix(')')))
        .expect("tools/host-test.sh still lists the crates it tests")
        .split_whitespace()
        .collect::<Vec<_>>();
    assert!(!crates.is_empty(), "tools/host-test.sh tests no crates");

    let per_crate: Vec<(&str, u32)> = crates
        .iter()
        .map(|c| (*c, number_after(sentence, what, &format!("`{c}` "))))
        .collect();
    let sum: u32 = per_crate.iter().map(|(_, n)| n).sum();
    assert_eq!(
        sum, published,
        "the published per-crate counts {per_crate:?} add up to {sum}, not the \
         {published} tests the same sentence claims"
    );
}
