//! Fixed-capacity UTF-8 string. Host text is arbitrary and longer than any
//! buffer, so every setter truncates on a character boundary instead of
//! failing, and the bytes are valid UTF-8 at all times.

/// At most `N` bytes of UTF-8. Plain old data: `Copy`, const-constructible,
/// and small enough (`N <= 64`) to pass by value.
#[derive(Clone, Copy)]
pub struct Text<const N: usize> {
    len: u16,
    bytes: [u8; N],
}

impl<const N: usize> Text<N> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            bytes: [0; N],
        }
    }

    /// For statics and tables. Truncates like `set`.
    pub const fn from_str(s: &str) -> Self {
        let src = s.as_bytes();
        let mut end = if src.len() < N { src.len() } else { N };
        // A continuation byte at the cut belongs to a character that did not fit.
        while end > 0 && end < src.len() && src[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        let mut bytes = [0; N];
        let mut i = 0;
        while i < end {
            bytes[i] = src[i];
            i += 1;
        }
        Self {
            len: end as u16,
            bytes,
        }
    }

    pub fn as_str(&self) -> &str {
        // Every writer keeps the prefix valid; never trust that with `unsafe`.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Replace the contents, truncating on a character boundary.
    pub fn set(&mut self, s: &str) {
        self.len = 0;
        self.push_str(s);
    }

    /// Replace the contents with untrusted bytes: each invalid UTF-8 sequence
    /// becomes U+FFFD, C0 controls and DEL become a space.
    pub fn set_lossy(&mut self, raw: &[u8]) {
        self.len = 0;
        for chunk in raw.utf8_chunks() {
            for c in chunk.valid().chars() {
                let c = if c.is_ascii_control() { ' ' } else { c };
                if !self.push_char(c) {
                    return;
                }
            }
            if !chunk.invalid().is_empty() && !self.push_char(char::REPLACEMENT_CHARACTER) {
                return;
            }
        }
    }

    /// Keep the END of `s`: a live transcript shows its newest words.
    pub fn set_tail(&mut self, s: &str) {
        let mut start = s.len().saturating_sub(N);
        while !s.is_char_boundary(start) {
            start += 1;
        }
        self.set(&s[start..]);
    }

    /// Append as many whole characters as fit.
    pub fn push_str(&mut self, s: &str) {
        let room = N - self.len as usize;
        let mut end = s.len().min(room);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let at = self.len as usize;
        self.bytes[at..at + end].copy_from_slice(&s.as_bytes()[..end]);
        self.len += end as u16;
    }

    /// False when the character did not fit (nothing is written then).
    pub fn push_char(&mut self, c: char) -> bool {
        let mut utf8 = [0; 4];
        let encoded = c.encode_utf8(&mut utf8).as_bytes();
        let at = self.len as usize;
        if at + encoded.len() > N {
            return false;
        }
        self.bytes[at..at + encoded.len()].copy_from_slice(encoded);
        self.len += encoded.len() as u16;
        true
    }

    /// Remove the last character, for local editing.
    pub fn pop(&mut self) -> Option<char> {
        let c = self.as_str().chars().next_back()?;
        self.len -= c.len_utf8() as u16;
        Some(c)
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize, const M: usize> PartialEq<Text<M>> for Text<N> {
    fn eq(&self, other: &Text<M>) -> bool {
        self.as_str() == other.as_str()
    }
}
impl<const N: usize> Eq for Text<N> {}
impl<const N: usize> PartialEq<str> for Text<N> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl<const N: usize> PartialEq<&str> for Text<N> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl<const N: usize> core::fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}
impl<const N: usize> core::fmt::Write for Text<N> {
    /// Truncates silently, so `write!` into a `Text` never fails.
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_truncates_on_a_character_boundary_for_every_width() {
        // 1-, 2-, 3- and 4-byte characters straddling the cut.
        for (s, expected) in [
            ("abcdefgh", "abcdef"),
            ("abcde\u{e9}x", "abcde"),
            ("abcd\u{e9}x", "abcd\u{e9}"),
            ("abcd\u{2026}", "abcd"),
            ("abc\u{2026}x", "abc\u{2026}"),
            ("abc\u{1F600}", "abc"),
            ("ab\u{1F600}x", "ab\u{1F600}"),
            ("\u{1F600}\u{1F600}", "\u{1F600}"),
            ("", ""),
        ] {
            let mut text = Text::<6>::new();
            text.set(s);
            assert_eq!(text.as_str(), expected, "{s:?}");
            assert_eq!(Text::<6>::from_str(s).as_str(), expected, "const {s:?}");
            assert_eq!(text.len(), expected.len());
            assert_eq!(text.is_empty(), expected.is_empty());
        }
        let mut exact = Text::<6>::new();
        exact.set("ab\u{1F600}");
        assert_eq!(exact.as_str(), "ab\u{1F600}");
    }

    #[test]
    fn set_replaces_and_clear_empties() {
        let mut text = Text::<8>::new();
        text.set("longer");
        text.set("ab");
        assert_eq!(text, "ab");
        text.clear();
        assert!(text.is_empty());
        assert_eq!(text.as_str(), "");
        assert_eq!(text.capacity(), 8);
    }

    #[test]
    fn set_lossy_replaces_invalid_sequences_and_controls() {
        let mut text = Text::<32>::new();
        for (raw, expected) in [
            (&b"plain"[..], "plain"),
            (b"a\x80b", "a\u{FFFD}b"),             // lone continuation byte
            (b"\x80\x80", "\u{FFFD}\u{FFFD}"),     // each is its own error
            (b"a\xC3", "a\u{FFFD}"),               // truncated 2-byte sequence
            (b"a\xE2\x80", "a\u{FFFD}"),           // truncated 3-byte sequence
            (b"\xF0\x9F\x98x", "\u{FFFD}x"),       // truncated 4-byte sequence
            (b"\xC0\xAF", "\u{FFFD}\u{FFFD}"),     // overlong encoding
            (b"\xED\xA0\x80", "\u{FFFD}\u{FFFD}\u{FFFD}"), // UTF-16 surrogate
            (b"\xFF\xFE", "\u{FFFD}\u{FFFD}"),
            (b"tab\there\r\nnul\0del\x7Fesc\x1B", "tab here  nul del esc "),
            ("caf\u{e9} \u{b7} \u{2026}".as_bytes(), "caf\u{e9} \u{b7} \u{2026}"),
            (b"", ""),
        ] {
            text.set_lossy(raw);
            assert_eq!(text.as_str(), expected, "{raw:?}");
        }
    }

    #[test]
    fn set_lossy_truncates_whole_characters_including_replacements() {
        let mut text = Text::<4>::new();
        text.set_lossy(b"ab\x80\x80");
        assert_eq!(text.as_str(), "ab"); // U+FFFD needs three bytes
        text.set_lossy(b"a\x80\x80");
        assert_eq!(text.as_str(), "a\u{FFFD}");
        text.set_lossy("abc\u{e9}".as_bytes());
        assert_eq!(text.as_str(), "abc");
        // Every prefix of every byte string stays valid UTF-8.
        let noise: [u8; 12] = [0xF0, 0x9F, 0x98, 0x80, 0xE2, 0x80, 0xA6, 0xC3, 0xA9, 0x41, 0x80, 0x09];
        for start in 0..noise.len() {
            for end in start..=noise.len() {
                let mut text = Text::<5>::new();
                text.set_lossy(&noise[start..end]);
                assert!(core::str::from_utf8(text.as_str().as_bytes()).is_ok());
                assert!(text.len() <= 5);
                assert!(!text.as_str().chars().any(|c| c.is_ascii_control()));
            }
        }
    }

    #[test]
    fn set_tail_keeps_the_end_on_a_character_boundary() {
        let mut text = Text::<6>::new();
        for (s, expected) in [
            ("short", "short"),
            ("0123456789", "456789"),
            ("0123\u{e9}6789", "\u{e9}6789"),
            ("012\u{e9}56789", "56789"), // the cut would split the e-acute
            ("01\u{1F600}6789", "6789"),
            ("\u{1F600}\u{1F600}", "\u{1F600}"),
            ("", ""),
        ] {
            text.set_tail(s);
            assert_eq!(text.as_str(), expected, "{s:?}");
        }
    }

    #[test]
    fn push_appends_whole_characters_until_full() {
        let mut text = Text::<6>::new();
        text.push_str("ab");
        text.push_str("c\u{e9}\u{e9}");
        assert_eq!(text.as_str(), "abc\u{e9}");
        assert!(!text.push_char('\u{e9}'));
        assert!(text.push_char('d'));
        assert!(!text.push_char('e'));
        assert_eq!(text.as_str(), "abc\u{e9}d");
        text.push_str("more");
        assert_eq!(text.as_str(), "abc\u{e9}d");
        assert_eq!(text.pop(), Some('d'));
        assert_eq!(text.pop(), Some('\u{e9}'));
        assert_eq!(text.as_str(), "abc");
        let mut empty = Text::<5>::new();
        assert_eq!(empty.pop(), None);
    }

    #[test]
    fn equality_compares_contents_not_stale_bytes_or_capacity() {
        let mut a = Text::<8>::new();
        a.set("abcdefgh");
        a.set("abc");
        let mut b = Text::<8>::new();
        b.set("abc");
        assert_eq!(a, b);
        assert_eq!(a, Text::<16>::from_str("abc"));
        assert_ne!(a, Text::<8>::from_str("abd"));
        assert_eq!(a, "abc");
        assert_eq!(std::format!("{a:?}"), "\"abc\"");
    }

    #[test]
    fn formatting_into_a_text_truncates_instead_of_failing() {
        use core::fmt::Write;
        let mut text = Text::<6>::new();
        assert!(write!(text, "{}%", 100).is_ok());
        assert!(write!(text, " {}", 12345).is_ok());
        assert_eq!(text.as_str(), "100% 1");
    }

    #[test]
    fn zero_capacity_is_harmless() {
        let mut text = Text::<0>::new();
        text.set("x");
        text.set_lossy(b"\x80");
        text.set_tail("abc");
        assert!(!text.push_char('x'));
        assert!(text.is_empty());
    }
}
