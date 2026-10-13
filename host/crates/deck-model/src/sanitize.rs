//! §5.6's card mapping rules, the half that is pure text (B9): what every
//! adapter — and every virtual agent that wants to look like one — runs a
//! tool name, a prompt or an option list through before it becomes a card.
//!
//! - `text`: ANSI stripped, a code fence folded to `code: <first line>`,
//!   whitespace collapsed, cut to 128 B **on a character boundary** with `…`.
//! - `label` / `labels`: the same, cut to 32 B, and colliding cut labels
//!   disambiguated with `#n` so the answer the operator sees is still the
//!   answer the adapter can map back.
//!
//! The **store keeps whatever it was given** (§4.3: the GUI always shows
//! all of it) and the wire encoder cuts bytes again for the device. This
//! module is what makes the deck's 128/32 bytes worth reading: a raw
//! `tool_input` is a JSON blob with escapes and colour codes in it, and
//! cutting *that* at 128 bytes shows the operator nothing they can decide
//! on.
//!
//! **`Answer.label` is what the deck drew**, which for an adapter that ran
//! its options through [`labels`] first is the *cut* label — not the
//! string the CLI sent. An adapter that has to hand its CLI back one of
//! its own options therefore maps `Answer.choice` through the list it
//! sent (and through `Card::choice_map` when more options were sent than
//! the deck can show, B10); it never files the drawn label as the answer.
//!
//! Everything here is a pure function of its input: no clock, no config, no
//! state. That is what lets the adapters be tested as tables.
use std::collections::{BTreeMap, BTreeSet};

/// §5.6: the card text the deck shows, in bytes.
pub const TEXT_MAX: usize = 128;
/// §5.6: one choice label, in bytes.
pub const LABEL_MAX: usize = 32;
/// What a cut string ends with (3 bytes).
pub const ELLIPSIS: &str = "…";

/// Drop ANSI escape sequences and the other control characters a terminal
/// would have eaten: CSI (`ESC [ … final`), OSC (`ESC ] … BEL`/`ESC \`),
/// the string escapes DCS/SOS/PM/APC, any two-character `ESC x`, and every
/// C0 control but tab/newline/carriage return (which `collapse` folds into
/// spaces) plus DEL.
///
/// Hook payloads carry these routinely: a `Bash` command echoing a coloured
/// prompt, a tool result with a spinner in it, an OSC 2 title stamp of our
/// own. Left in, they reach the deck as mojibake and eat the byte budget.
#[must_use]
pub fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // Tab/newline/CR survive for `collapse`; the rest of C0 and DEL
            // are terminal instructions, not content.
            if !c.is_control() || matches!(c, '\t' | '\n' | '\r') {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters, then intermediates, then one final byte.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC and the string escapes: up to BEL or ST (`ESC \`).
            Some(']' | 'P' | 'X' | '^' | '_') => {
                while let Some(c) = chars.next() {
                    match c {
                        '\u{7}' => break,
                        '\u{1b}' if chars.peek() == Some(&'\\') => {
                            chars.next();
                            break;
                        }
                        _ => {}
                    }
                }
            }
            // `ESC I… F`: intermediates then one final byte — a charset
            // designation like `ESC ( B` is three characters, not two.
            Some(c) if ('\u{20}'..='\u{2f}').contains(&c) => {
                for c in chars.by_ref() {
                    if !('\u{20}'..='\u{2f}').contains(&c) {
                        break;
                    }
                }
            }
            // `ESC x`: the escape and its one character.
            Some(_) => {}
            None => break,
        }
    }
    out
}

/// Replace every fenced code block by `code: <its first non-empty line>`.
///
/// A fence is three or more backticks at the start of a line, optionally
/// with an info string; the block ends at the next such fence or at the end
/// of the input (a model's reply is often cut mid-fence). The deck has 128
/// bytes: a fence spends all of them on `\`\`\`bash` and indentation, where
/// the first line of the block is the one thing that says what the code
/// would do.
#[must_use]
pub fn fold_code_fences(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut lines = raw.split('\n').peekable();
    while let Some(line) = lines.next() {
        if !is_fence(line) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            continue;
        }
        let mut first = String::new();
        for body in lines.by_ref() {
            if is_fence(body) {
                break;
            }
            if first.is_empty() && !body.trim().is_empty() {
                first = body.trim().to_string();
            }
        }
        if !out.is_empty() {
            out.push('\n');
        }
        if first.is_empty() {
            out.push_str("code:");
        } else {
            out.push_str("code: ");
            out.push_str(&first);
        }
    }
    out
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// Every run of whitespace becomes one space, and the ends are trimmed.
#[must_use]
pub fn collapse(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut space = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

/// Cut to at most `max` **bytes**, on a character boundary, ending in `…`
/// when anything was dropped.
///
/// The budget is in bytes because the device's line budget is (§6.1), and
/// the boundary matters because the deck renders UTF-8: cutting a 3-byte
/// CJK character or a 4-byte emoji in half is a replacement glyph, or a
/// line the firmware refuses. A budget with no room for the ellipsis
/// (`max <= 3`) spends it on content instead: three bytes of the string
/// say more than three bytes of "there was a string".
#[must_use]
pub fn cut(raw: &str, max: usize) -> String {
    if raw.len() <= max {
        return raw.to_string();
    }
    let budget = max.saturating_sub(ELLIPSIS.len());
    if budget == 0 {
        // No room for the ellipsis: three bytes of the string say more
        // than three bytes of "there was a string".
        return cut_bare(raw, max);
    }
    let mut out = String::with_capacity(max);
    out.push_str(&cut_bare(raw, budget));
    out.push_str(ELLIPSIS);
    out
}

/// Cut to at most `max` **bytes** on a character boundary and **say
/// nothing about it**: no ellipsis, no marker.
///
/// This is not a smaller [`cut`]; it is for the fields where an ellipsis
/// would be a lie. The spoken face name of §7.4 (`ptt ctx:name` →
/// `device.name`) is the case that forced it: 16 bytes is the firmware's
/// whole budget (`CAPS.spoken_name`), the deck **draws the string as the
/// face's name**, and a face called `Deploy sta…` is not a name anybody
/// said — it reads as a bug in the device. A truncation marker belongs on a
/// card's text, which is a quotation the operator can go and read in full;
/// it does not belong on an identifier.
///
/// `max` smaller than the first character yields the empty string rather
/// than a broken code point.
#[must_use]
pub fn cut_bare(raw: &str, max: usize) -> String {
    if raw.len() <= max {
        return raw.to_string();
    }
    let mut end = max.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].to_string()
}

/// Keep at most `max` **characters**, with no marker — the spoken
/// counterpart of [`cut_bare`].
///
/// Bytes are the device's budget and characters are speech's: §7.7's
/// "card text ≤80 chars" and §7.6's `announce_max_chars` are both about
/// how long a sentence the operator has to sit through, and a Japanese
/// question is no slower to say for being three bytes a character. Cutting
/// it by bytes would read out a quarter of it.
///
/// No ellipsis, because there is no such sound: a listener hears the
/// sentence stop, which is the same information.
#[must_use]
pub fn clip_chars(raw: &str, max: usize) -> String {
    match raw.char_indices().nth(max) {
        Some((end, _)) => raw[..end].to_string(),
        None => raw.to_string(),
    }
}

/// Upper-case the first letter of every whitespace-separated word, leaving
/// the rest of each word exactly as it was.
///
/// §7.4 asks for the spoken face name to be "title-cased for Latin
/// scripts". The rest of the word is deliberately **not** lower-cased: a
/// dictated `"API deploy"` must not come back as `"Api Deploy"`, and the
/// operator said the capitals.
///
/// The Latin restriction needs no script table, because the transform is
/// keyed on *case* rather than on script: `char::to_uppercase` is the
/// identity for every caseless script, so the CJK name §7.4 contrasts Latin
/// with (five characters in the 16-byte budget) passes through untouched,
/// and the other bicameral scripts are capitalised the way a reader of them
/// would expect.
#[must_use]
pub fn title_case(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut fresh = true;
    for c in raw.chars() {
        if c.is_whitespace() {
            fresh = true;
            out.push(c);
        } else if std::mem::take(&mut fresh) {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// §5.6's pipeline **without** the cut: ANSI out, fences folded,
/// whitespace collapsed, and every byte of the result kept.
///
/// This is what a field goes through when the store is meant to hold all
/// of it and the **projection** is where it is cut — `agent.reply` is the
/// one that matters (§4.3: the GUI shows the whole reply, and the device's
/// own 200-byte budget is applied by the wire encoder, `deck-proto`'s
/// `Caps::reply`). §5.6's 128 bytes are a rule about a *card*, whose text
/// has to fit a deck line; applying it to a reply throws away 90% of what
/// the model said before anything has a chance to show it.
#[must_use]
pub fn clean(raw: &str) -> String {
    collapse(&fold_code_fences(&strip_ansi(raw)))
}

/// Drop the markdown a model writes and a synthesiser would read out
/// loud: headings, emphasis, inline code, links, quotes and list markers.
///
/// Nothing here understands markdown; it removes the **characters a
/// listener should not hear**. "star star done star star" and "backtick
/// cargo test backtick" are the failure this prevents, and a model's reply
/// is full of both.
///
/// The restraint matters more than the coverage, because every rule here
/// can eat something that was not markup:
///
/// * `_` is dropped only next to whitespace or the ends, so
///   `snake_case_name` survives — identifiers are most of what an agent
///   talks about;
/// * `*` and `~` are dropped only when **not** spaced on both sides, so
///   `2 * 3` keeps its arithmetic;
/// * `\*` is an escape and yields a literal `*`.
///
/// A link keeps its label and loses its target: nobody wants a URL read
/// out, and the label is the part that was written for a human.
#[must_use]
pub fn strip_markdown(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for (n, line) in raw.lines().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        out.push_str(&strip_markdown_line(line));
    }
    out
}

/// The block-level markers of one line, then its inline ones.
fn strip_markdown_line(line: &str) -> String {
    let mut rest = line.trim_start();
    // Block quotes nest: `> > quoted`.
    while let Some(tail) = rest.strip_prefix('>') {
        rest = tail.trim_start();
    }
    // A heading is 1..=6 `#` and then a space.
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        rest = rest[hashes..].trim_start();
    }
    // A horizontal rule is nothing but `-`, `*` or `_`, at least three.
    let ruler = rest.len() >= 3
        && (rest.chars().all(|c| c == '-')
            || rest.chars().all(|c| c == '*')
            || rest.chars().all(|c| c == '_'));
    if ruler {
        return String::new();
    }
    // A list marker is `-`, `*`, `+` or `<digits>.` and then a space.
    if let Some(tail) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        rest = tail;
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(". ") {
            rest = &rest[digits + 2..];
        }
    }
    strip_inline_markdown(rest)
}

fn strip_inline_markdown(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            // An escape: the next character is literal, marker or not.
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '`' => i += 1,
            // `[label](target)` / `![alt](target)`: keep the label.
            '!' if chars.get(i + 1) == Some(&'[') => i += 1,
            '[' => {
                match link_at(&chars, i) {
                    Some((label, next)) => {
                        out.push_str(&strip_inline_markdown(&label));
                        i = next;
                    }
                    // Not a link: an ordinary bracket, which is spoken.
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            '*' | '~' | '_' if emphasis_marker(&chars, i, c) => {
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `[label](target)` starting at `i`: the label and the index just past
/// the closing `)`.
fn link_at(chars: &[char], i: usize) -> Option<(String, usize)> {
    let close = (i + 1..chars.len()).find(|n| chars[*n] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|n| chars[*n] == ')')?;
    Some((chars[i + 1..close].iter().collect(), end + 1))
}

/// Is the marker at `i` emphasis rather than content?
///
/// `_` counts only at a word edge, so identifiers survive; `*` and `~`
/// count unless they are spaced on both sides, which is how arithmetic and
/// a shell glob are written.
fn emphasis_marker(chars: &[char], i: usize, c: char) -> bool {
    // A run of the same marker (`**`, `__`, `~~`) is one marker, so both
    // ends of the run decide, not both ends of one character.
    let mut left = i;
    while left > 0 && chars[left - 1] == c {
        left -= 1;
    }
    let mut right = i;
    while right + 1 < chars.len() && chars[right + 1] == c {
        right += 1;
    }
    let edge = |ch: Option<char>| ch.is_none_or(char::is_whitespace);
    let (before, after) = (
        edge(left.checked_sub(1).map(|n| chars[n])),
        edge(chars.get(right + 1).copied()),
    );
    if c == '_' {
        // Identifiers are most of what an agent talks about, and
        // `snake_case_name` has an underscore between two letters. Only a
        // run that touches a word edge is emphasis.
        before || after
    } else {
        // `2 * 3` and `a ~ b` are content; `*x*` and `~~x~~` are markup.
        !(before && after)
    }
}

/// The first sentence of `text`, ending at `.`/`!`/`?` **followed by
/// whitespace or the end**, or at a line break.
///
/// The whitespace condition is what keeps `3.5 s on v1.2.3` whole. A naive
/// "stop at the first dot" reads "the build took 3." and stops, which is
/// worse than saying nothing at all — the operator hears a complete
/// sentence that is not true.
#[must_use]
pub fn first_sentence(text: &str) -> String {
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        let end = match c {
            '\n' | '\r' => i,
            '.' | '!' | '?' => {
                let next = bytes.get(i + 1);
                match next {
                    None => i + 1,
                    Some(b) if b.is_ascii_whitespace() => i + 1,
                    // `.` inside `3.5` or `v1.2.3` is not a full stop.
                    _ => continue,
                }
            }
            _ => continue,
        };
        return text[..end].trim().to_string();
    }
    text.trim().to_string()
}

/// §7.6's TTS text sanitiser: what the synthesiser is actually handed.
///
/// ANSI out, code fences folded to their first line, markdown stripped,
/// whitespace collapsed, the first sentence only, and at most `max_chars`
/// **characters**.
///
/// Characters and not bytes, because the budget is how long the operator
/// listens: 120 characters of Japanese is 360 bytes, and a byte cut would
/// read out a third of the sentence and then stop mid-word. The cut is on
/// a character boundary by construction, so no budget can produce a
/// replacement glyph — and no budget can produce a half-emoji either,
/// which is four bytes and one character.
///
/// It is applied to the **announcements the daemon composes** (§7.7's
/// ready, error, question and report lines), which is what
/// `[voice.tts] announce_max_chars` is named for. `voice.say` from an API
/// client is not sanitised this way: a caller that asked for two sentences
/// and silently got one is worse off than one that was refused, the same
/// argument §4.9's `ttl_s` already settled. Its text still has ANSI and
/// fences taken out, because those are never speech.
#[must_use]
pub fn speech(raw: &str, max_chars: usize) -> String {
    let cleaned = collapse(&strip_markdown(&fold_code_fences(&strip_ansi(raw))));
    clip_chars(&first_sentence(&cleaned), max_chars)
}

/// The whole pipeline for a card's `text`: ANSI out, fences folded,
/// whitespace collapsed, cut to `TEXT_MAX`.
#[must_use]
pub fn text(raw: &str) -> String {
    cut(&clean(raw), TEXT_MAX)
}

/// The same for one choice label, cut to `LABEL_MAX`. The result can be
/// empty (a label that was nothing but escapes); the store refuses a blank
/// label with `-32009 invalid{choices, "empty label"}`, so an adapter
/// checks rather than sending it.
#[must_use]
pub fn label(raw: &str) -> String {
    cut(&collapse(&fold_code_fences(&strip_ansi(raw))), LABEL_MAX)
}

/// Every label of one card, with colliding cut labels disambiguated `#1`,
/// `#2`, … in the order they were given.
///
/// Without this, two long options that differ past byte 32 arrive on the
/// deck as the same row — and since the answer payload carries `{choice,
/// label}` with the label *as shown* (§5.6, §4.6), an adapter mapping an
/// answer back by label would map both to the first one. Every member of a
/// colliding group is numbered, including the first: `Yes #1` next to `Yes
/// #2` says "these were different", where a bare `Yes` next to `Yes #2`
/// only says it about one of them.
#[must_use]
pub fn labels(raw: &[String]) -> Vec<String> {
    let shown: Vec<String> = raw.iter().map(|l| label(l)).collect();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &shown {
        *seen.entry(s.as_str()).or_default() += 1;
    }
    let colliding: BTreeSet<String> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(s, _)| (*s).to_string())
        .collect();
    if colliding.is_empty() {
        return shown;
    }
    let mut used: BTreeSet<String> = shown
        .iter()
        .filter(|s| !colliding.contains(*s))
        .cloned()
        .collect();
    let mut nth: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(shown.len());
    for s in shown {
        if !colliding.contains(&s) {
            out.push(s);
            continue;
        }
        let counter = nth.entry(s.clone()).or_insert(0);
        let numbered = loop {
            *counter += 1;
            let candidate = number(&s, *counter);
            if !used.contains(&candidate) {
                break candidate;
            }
        };
        used.insert(numbered.clone());
        out.push(numbered);
    }
    out
}

/// `<label cut to fit>#n`, still within `LABEL_MAX`.
fn number(base: &str, n: usize) -> String {
    let suffix = format!("#{n}");
    let mut out = cut(base, LABEL_MAX.saturating_sub(suffix.len()));
    out.push_str(&suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_and_control_characters_are_dropped_and_the_text_survives() {
        assert_eq!(strip_ansi("\u{1b}[31mDeny\u{1b}[0m"), "Deny");
        assert_eq!(strip_ansi("\u{1b}[1;32;40mok\u{1b}[m!"), "ok!");
        // OSC, both terminators — our own `deck:` title stamp is one.
        assert_eq!(strip_ansi("a\u{1b}]2;deck:9f3a\u{7}b"), "ab");
        assert_eq!(strip_ansi("a\u{1b}]0;title\u{1b}\\b"), "ab");
        // DCS/APC, a bare two-character escape, C0 and DEL.
        assert_eq!(strip_ansi("x\u{1b}Pq…\u{1b}\\y"), "xy");
        assert_eq!(strip_ansi("x\u{1b}(By"), "xy");
        assert_eq!(strip_ansi("a\u{0}b\u{7}c\u{7f}d"), "abcd");
        // Tab/newline/CR are content until `collapse` folds them.
        assert_eq!(strip_ansi("a\tb\nc\rd"), "a\tb\nc\rd");
        // A truncated escape at the end takes nothing with it.
        assert_eq!(strip_ansi("done\u{1b}"), "done");
        assert_eq!(strip_ansi("done\u{1b}[31"), "done");
        // Plain text, including non-ASCII, is untouched.
        assert_eq!(strip_ansi("naïve 日本語 🙂"), "naïve 日本語 🙂");
    }

    #[test]
    fn a_code_fence_becomes_its_first_line() {
        assert_eq!(
            fold_code_fences("```bash\nrm -rf /tmp/x\necho done\n```"),
            "code: rm -rf /tmp/x"
        );
        // No info string, leading blank line, and the tilde form.
        assert_eq!(fold_code_fences("```\n\n  ls -la\n```"), "code: ls -la");
        assert_eq!(fold_code_fences("~~~\nls\n~~~"), "code: ls");
        // Unterminated (a reply cut mid-block) still folds.
        assert_eq!(
            fold_code_fences("look:\n```py\nimport os"),
            "look:\ncode: import os"
        );
        // An empty block says so rather than inventing a line.
        assert_eq!(fold_code_fences("```\n```"), "code:");
        // Prose around two blocks keeps its order.
        assert_eq!(
            fold_code_fences("before\n```\none\n```\nmiddle\n```\ntwo\n```\nafter"),
            "before\ncode: one\nmiddle\ncode: two\nafter"
        );
        // No fence, no change.
        assert_eq!(fold_code_fences("a ``b`` c"), "a ``b`` c");
    }

    #[test]
    fn whitespace_collapses_to_single_spaces_and_the_ends_are_trimmed() {
        assert_eq!(collapse("  a \t\n  b  \r\n"), "a b");
        assert_eq!(collapse("\n\n\n"), "");
        assert_eq!(collapse("one"), "one");
        // Non-breaking space and friends are whitespace too.
        assert_eq!(collapse("a\u{a0}\u{2003}b"), "a b");
    }

    #[test]
    fn a_cut_never_splits_a_character_and_never_exceeds_the_budget() {
        assert_eq!(cut("hello", 5), "hello", "exactly the budget is not cut");
        assert_eq!(cut("hello", 4), "h…");
        // 2-byte, 3-byte and 4-byte characters, cut at every budget that
        // lands inside one of them.
        for raw in ["éééééééé", "日本語日本語日本語", "🙂🙂🙂🙂🙂🙂"] {
            for max in 0..=raw.len() + 2 {
                let out = cut(raw, max);
                assert!(out.len() <= max, "{raw:?} at {max}: {out:?}");
                assert!(
                    raw.starts_with(out.trim_end_matches(ELLIPSIS)),
                    "{raw:?} at {max}: {out:?}"
                );
            }
        }
        // The ellipsis is 3 bytes. At or under that there is no room for
        // both, and content wins; above it the cut lands on a boundary.
        assert_eq!(cut("日本語", 3), "日");
        assert_eq!(cut("日本語", 2), "", "not even one character fits");
        assert_eq!(cut("日本語", 6), "日…");
        assert_eq!(cut("日本語", 7), "日…", "4 bytes of budget, 3 usable");
        assert_eq!(cut("日本語", 9), "日本語", "exactly the budget");
        // An empty input is never decorated.
        assert_eq!(cut("", 0), "");
    }

    /// B15: the cut with **no** marker, and the title case that goes with
    /// it. Both exist for §7.4's spoken face name, where an ellipsis would
    /// read as a device fault rather than as a quotation.
    #[test]
    fn a_bare_cut_says_nothing_and_title_case_leaves_the_rest_of_the_word_alone() {
        assert_eq!(cut_bare("hello", 5), "hello");
        assert_eq!(cut_bare("hello", 4), "hell");
        assert_eq!(cut_bare("hello", 0), "");
        // The same boundary discipline as `cut`, with every byte spent on
        // content: never a split character, never over the budget, always a
        // prefix of the input and never decorated.
        for raw in ["éééééééé", "日本語日本語日本語", "🙂🙂🙂🙂🙂🙂"] {
            for max in 0..=raw.len() + 2 {
                let out = cut_bare(raw, max);
                assert!(out.len() <= max, "{raw:?} at {max}: {out:?}");
                assert!(raw.starts_with(&out), "{raw:?} at {max}: {out:?}");
                assert!(!out.contains(ELLIPSIS), "{raw:?} at {max}: {out:?}");
                // Where `cut` has to spend 3 of its bytes on the ellipsis,
                // this one spends them on the name.
                assert!(out.len() >= cut(raw, max).trim_end_matches(ELLIPSIS).len());
            }
        }
        assert_eq!(cut_bare("日本語", 8), "日本", "no half characters");
        assert_eq!(cut_bare("日本語", 2), "");

        assert_eq!(title_case("deploy staging"), "Deploy Staging");
        assert_eq!(title_case("Build"), "Build");
        // The rest of a word is the operator's: an acronym survives, and a
        // word already capitalised is not re-cased.
        assert_eq!(title_case("API deploy"), "API Deploy");
        assert_eq!(title_case("iPhone build"), "IPhone Build");
        // Runs of whitespace are preserved as given (`collapse` is a
        // separate step), and the first word after each run is capitalised.
        assert_eq!(title_case("  two   words"), "  Two   Words");
        // Caseless scripts pass through untouched, which is how the Latin
        // restriction of §7.4 is expressed without a script table.
        assert_eq!(title_case("厨房甲板"), "厨房甲板");
        assert_eq!(title_case("123 go"), "123 Go");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn the_text_pipeline_is_the_five_six_rule_end_to_end() {
        let raw =
            "\u{1b}[1mBash\u{1b}[0m:\n\n```bash\n  psql -f migrate.sql staging\n  echo ok\n```\n";
        assert_eq!(text(raw), "Bash: code: psql -f migrate.sql staging");

        // Over the budget: 128 bytes exactly, ending in the ellipsis.
        let long = "x".repeat(400);
        let out = text(&long);
        assert_eq!(out.len(), TEXT_MAX);
        assert!(out.ends_with(ELLIPSIS));
        assert_eq!(out.trim_end_matches(ELLIPSIS).len(), TEXT_MAX - 3);

        // …and with multi-byte content the cut lands short of the budget
        // rather than inside a character.
        let cjk = "日".repeat(200);
        let out = text(&cjk);
        assert!(out.len() <= TEXT_MAX, "{}", out.len());
        assert!(out.ends_with(ELLIPSIS));
        assert_eq!(out.chars().filter(|c| *c == '日').count(), 41);

        // Nothing but escapes is nothing.
        assert_eq!(text("\u{1b}[2J\u{1b}[H"), "");
    }

    #[test]
    fn colliding_cut_labels_are_numbered_and_still_fit() {
        let raw: Vec<String> = [
            "Allow this command in the current project only",
            "Allow this command in the current project and future ones",
            "Deny",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let out = labels(&raw);
        assert_eq!(out[2], "Deny", "an uncontested label is left alone");
        assert_ne!(out[0], out[1], "the deck must show two different rows");
        assert!(out[0].ends_with("#1") && out[1].ends_with("#2"));
        for l in &out {
            assert!(l.len() <= LABEL_MAX, "{l:?} is {} bytes", l.len());
        }

        // Three the same, and a fourth that already looks like a number:
        // the numbering must not collide with it either.
        let raw: Vec<String> = ["same", "same", "same", "same#1"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let out = labels(&raw);
        assert_eq!(out[3], "same#1", "not part of the colliding group");
        assert_eq!(out.len(), 4);
        let unique: BTreeSet<&String> = out.iter().collect();
        assert_eq!(unique.len(), 4, "{out:?}");
        assert!(out[0] != out[1] && out[1] != out[2]);

        // A multi-byte label that collides after the cut is re-cut to make
        // room for the suffix, still on a character boundary.
        let long = "日".repeat(20);
        let out = labels(&[long.clone(), format!("{long}x")]);
        for l in &out {
            assert!(l.len() <= LABEL_MAX, "{l:?} is {} bytes", l.len());
            assert!(l.ends_with("#1") || l.ends_with("#2"), "{l:?}");
        }
        assert_ne!(out[0], out[1]);

        // Nothing collides: nothing is numbered.
        let plain: Vec<String> = ["Allow", "Allow always", "Deny"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(labels(&plain), plain);
        assert!(labels(&[]).is_empty());
    }

    #[test]
    fn a_label_runs_the_same_pipeline_as_the_text() {
        assert_eq!(label("\u{1b}[32mAllow  always\u{1b}[0m"), "Allow always");
        assert_eq!(label("```\nrm -rf /\n```"), "code: rm -rf /");
        assert_eq!(
            label(" \n "),
            "",
            "the store refuses this; the adapter checks"
        );
        let out = label(&"option ".repeat(20));
        assert_eq!(out.len(), LABEL_MAX);
        assert!(out.ends_with(ELLIPSIS));
    }
}
