//! The environment subset `deckd hook` reports (contract §4.2, design
//! §5.5): the few variables that say **which terminal** a CLI session runs
//! in, and nothing else.
//!
//! The list is closed on purpose (nine names). A hook process inherits the whole
//! launching environment — `codex`'s own dump shows `PATH`, `SHELL`,
//! `CODEX_HOME`, every `ANTHROPIC_*`/`OPENAI_*` key the user exports and
//! whatever their shell profile put there (`$S/w3/facts-codex.md` §3.4) —
//! and those are credentials and private paths. §11 says hook payloads are
//! the most sensitive data on the box; the same goes for the env beside
//! them. So the helper copies nine names by hand and the daemon never
//! sees the rest.
//!
//! `CLAUDE_CODE_REMOTE` / `CLAUDE_CODE_BRIDGE_SESSION_ID` are here because
//! a remote-controlled Claude session has no local terminal at all: their
//! presence is how the adapter knows not to promise a focus.
//!
//! `CLAUDE_CODE_MESSAGING_SOCKET` / `CLAUDE_CODE_MESSAGING_TOKEN` are the
//! session's own message inbox (cross-session messaging, Claude Code
//! v2.1.224+): the Unix socket the session reads peer messages from and the
//! per-session token that makes a post from this daemon count as the
//! session's own child. They are how a spoken order reaches a Claude
//! session without typing into its terminal (§5.5). The token is a secret
//! and is treated as one: the daemon keeps it in memory, never in a
//! snapshot, an event or a log line.
use std::collections::BTreeMap;

/// The variables the helper copies, in the order it reports them.
pub const CAPTURED: [&str; 9] = [
    "TERM_PROGRAM",
    "TMUX_PANE",
    "ITERM_SESSION_ID",
    "TERM_SESSION_ID",
    "GHOSTTY_RESOURCES_DIR",
    "CLAUDE_CODE_REMOTE",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
];

/// One variable may not be longer than this (a pathological value is cut
/// rather than refused: the terminal identity in the first bytes is still
/// worth having, and a 1 MiB `TERM_PROGRAM` must not reach the socket).
pub const VALUE_MAX: usize = 512;

/// The subset, read through `lookup` — the process environment in
/// production, a table in the tests. Empty values are dropped: an exported
/// but empty `TMUX_PANE` says nothing and would otherwise look like a pane.
#[must_use]
pub fn capture(lookup: impl Fn(&str) -> Option<String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for name in CAPTURED {
        let Some(value) = lookup(name) else { continue };
        if value.is_empty() {
            continue;
        }
        out.insert(name.to_string(), cut(&value));
    }
    out
}

/// The subset of this process's own environment.
#[must_use]
pub fn from_process() -> BTreeMap<String, String> {
    capture(|name| std::env::var(name).ok())
}

/// Cut to `VALUE_MAX` bytes on a character boundary.
fn cut(value: &str) -> String {
    if value.len() <= VALUE_MAX {
        return value.to_string();
    }
    let mut end = VALUE_MAX;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        capture(|name| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn only_the_listed_names_are_copied() {
        let got = table(&[
            ("TERM_PROGRAM", "ghostty"),
            ("TMUX_PANE", "%3"),
            ("ANTHROPIC_API_KEY", "sk-ant-secret"),
            ("PATH", "/usr/bin"),
            ("HOME", "/Users/someone"),
            ("CODEX_HOME", "/Users/someone/.codex"),
        ]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got["TERM_PROGRAM"], "ghostty");
        assert_eq!(got["TMUX_PANE"], "%3");
        for secret in ["ANTHROPIC_API_KEY", "PATH", "HOME", "CODEX_HOME"] {
            assert!(!got.contains_key(secret), "{secret} must never be reported");
        }
    }

    #[test]
    fn an_empty_value_is_not_a_terminal() {
        let got = table(&[("TMUX_PANE", ""), ("TERM_SESSION_ID", "w0t2p0")]);
        assert_eq!(got.keys().collect::<Vec<_>>(), vec!["TERM_SESSION_ID"]);
    }

    #[test]
    fn a_pathological_value_is_cut_on_a_character_boundary() {
        let long = "é".repeat(VALUE_MAX);
        let got = table(&[("ITERM_SESSION_ID", &long)]);
        let value = &got["ITERM_SESSION_ID"];
        assert!(value.len() <= VALUE_MAX, "{}", value.len());
        assert!(
            long.starts_with(value.as_str()) && value.chars().all(|c| c == 'é'),
            "the cut fell inside a character"
        );
    }

    #[test]
    fn the_process_environment_reads_through_the_same_door() {
        // Nothing is asserted about the *values* (the test runner's own
        // terminal is whatever it is); the call must simply not panic and
        // must stay inside the closed list.
        for name in from_process().keys() {
            assert!(CAPTURED.contains(&name.as_str()), "{name}");
        }
    }
}
