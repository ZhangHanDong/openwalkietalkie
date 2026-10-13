//! Self-contained, allocation-free JSON for the channel-5 protocol: a reader
//! over one complete line and a writer that escapes strings. Must not use
//! `crate::wire` (it is mounted under different names per consumer).
//!
//! The reader is a scanner, not a validator: it finds members and elements by
//! bracket depth and quote rules (iterative, checked indexing, never a panic)
//! and the typed accessors validate the scalars a message actually uses.
//! Strings are decoded straight into a `Text`, so nothing is ever copied into
//! a buffer sized by the host.
use super::msg::ErrWhy;
use super::text::Text;

const PARSE: ErrWhy = ErrWhy::Parse;

fn is_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(|&b| is_ws(b)) {
        at += 1;
    }
    at
}

/// End (exclusive) of the string whose opening quote is at `at`.
fn skip_string(bytes: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    loop {
        match bytes.get(i)? {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
}

/// End (exclusive) of the value starting at `at`.
fn skip_value(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'"' => skip_string(bytes, at),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = at;
            loop {
                match bytes.get(i)? {
                    b'"' => {
                        i = skip_string(bytes, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        b',' | b'}' | b']' | b':' => None,
        _ => {
            let mut i = at;
            while bytes
                .get(i)
                .is_some_and(|&b| !matches!(b, b',' | b'}' | b']') && !is_ws(b))
            {
                i += 1;
            }
            Some(i)
        }
    }
}

/// One JSON value: the exact bytes of a string (with its quotes), number,
/// literal, array or object.
#[derive(Clone, Copy, Debug)]
pub struct Value<'a>(&'a [u8]);

impl<'a> Value<'a> {
    /// The bytes between the quotes, escapes intact; `None` for a non-string.
    pub fn raw_str(&self) -> Option<&'a [u8]> {
        match self.0 {
            [b'"', body @ .., b'"'] => Some(body),
            _ => None,
        }
    }

    /// The string equals `s` byte for byte (enum members are plain ASCII).
    pub fn is_str(&self, s: &str) -> bool {
        self.raw_str() == Some(s.as_bytes())
    }

    /// Look the string up in `table`: `Parse` for a non-string, `Unknown` for
    /// a member the firmware does not know.
    pub fn member<T: Copy>(&self, table: &[(&str, T)]) -> Result<T, ErrWhy> {
        let raw = self.raw_str().ok_or(PARSE)?;
        table
            .iter()
            .find(|(name, _)| name.as_bytes() == raw)
            .map(|(_, value)| *value)
            .ok_or(ErrWhy::Unknown)
    }

    /// Decode the string into `out` (see `decode_into`).
    pub fn text_into<const N: usize>(&self, out: &mut Text<N>) -> Result<(), ErrWhy> {
        decode_into(self.raw_str().ok_or(PARSE)?, out)
    }

    /// A non-negative integer literal. A negative, fractional, exponent or
    /// overflowing number is `Range`; anything else is `Parse`.
    pub fn u32(&self) -> Result<u32, ErrWhy> {
        let mut value: u32 = 0;
        let mut digits = 0;
        for &byte in self.0 {
            match byte {
                b'0'..=b'9' => {
                    value = value
                        .checked_mul(10)
                        .and_then(|v| v.checked_add((byte - b'0') as u32))
                        .ok_or(ErrWhy::Range)?;
                    digits += 1;
                }
                b'-' | b'+' | b'.' | b'e' | b'E' if digits > 0 || byte == b'-' => {
                    return Err(ErrWhy::Range)
                }
                _ => return Err(PARSE),
            }
        }
        if digits == 0 {
            return Err(PARSE);
        }
        Ok(value)
    }

    pub fn u16(&self) -> Result<u16, ErrWhy> {
        u16::try_from(self.u32()?).map_err(|_| ErrWhy::Range)
    }

    pub fn u8(&self) -> Result<u8, ErrWhy> {
        u8::try_from(self.u32()?).map_err(|_| ErrWhy::Range)
    }

    /// `true`/`false`, or an integer (0 = false): the tables write `gone:1`.
    pub fn bool(&self) -> Result<bool, ErrWhy> {
        match self.0 {
            b"true" => Ok(true),
            b"false" => Ok(false),
            _ => self.u32().map(|n| n != 0).map_err(|_| PARSE),
        }
    }

    pub fn array(&self) -> Result<Items<'a>, ErrWhy> {
        match self.0 {
            [b'[', body @ .., b']'] => Ok(Items {
                body,
                at: 0,
                pending: false,
                done: false,
            }),
            _ => Err(PARSE),
        }
    }

    pub fn object(&self) -> Result<Object<'a>, ErrWhy> {
        match self.0 {
            [b'{', body @ .., b'}'] => Ok(Object { body }),
            _ => Err(PARSE),
        }
    }
}

/// Elements of an array, in order. A malformed array yields one `Err` and
/// then ends.
pub struct Items<'a> {
    body: &'a [u8],
    at: usize,
    /// A comma was consumed: another element is owed.
    pending: bool,
    done: bool,
}

impl<'a> Iterator for Items<'a> {
    type Item = Result<Value<'a>, ErrWhy>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let body = self.body;
        let start = skip_ws(body, self.at);
        if start >= body.len() {
            self.done = true;
            // `[1,]` owes an element; `[]` and `[1]` end cleanly.
            return self.pending.then_some(Err(PARSE));
        }
        let Some(end) = skip_value(body, start) else {
            self.done = true;
            return Some(Err(PARSE));
        };
        let after = skip_ws(body, end);
        match body.get(after) {
            None => self.done = true,
            Some(b',') => {
                self.at = after + 1;
                self.pending = true;
            }
            Some(_) => {
                self.done = true;
                return Some(Err(PARSE));
            }
        }
        body.get(start..end).map(|raw| Ok(Value(raw)))
    }
}

/// The members of one object.
#[derive(Clone, Copy)]
pub struct Object<'a> {
    body: &'a [u8],
}

impl<'a> Object<'a> {
    /// A whole line: one object, nothing but whitespace around it. Every
    /// member is checked for shape (`"key": value`, comma-separated).
    pub fn parse(line: &'a [u8]) -> Result<Self, ErrWhy> {
        let start = skip_ws(line, 0);
        let end = skip_value(line, start).ok_or(PARSE)?;
        if skip_ws(line, end) != line.len() {
            return Err(PARSE);
        }
        let object = Value(line.get(start..end).ok_or(PARSE)?).object()?;
        for member in object.members() {
            member?;
        }
        Ok(object)
    }

    pub fn members(&self) -> Members<'a> {
        Members {
            body: self.body,
            at: 0,
            pending: false,
            done: false,
        }
    }

    /// The member named `key`, if present. A malformed object is `Parse`.
    pub fn get(&self, key: &str) -> Result<Option<Value<'a>>, ErrWhy> {
        for member in self.members() {
            let (name, value) = member?;
            if name == key.as_bytes() {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// The member named `key`; absent is `Parse`.
    pub fn need(&self, key: &str) -> Result<Value<'a>, ErrWhy> {
        self.get(key)?.ok_or(PARSE)
    }
}

/// Members of an object as (raw key bytes, value). A malformed object yields
/// one `Err` and then ends. Keys are compared raw: a key spelled with escapes
/// never matches, which is the same as an unknown field.
pub struct Members<'a> {
    body: &'a [u8],
    at: usize,
    pending: bool,
    done: bool,
}

impl<'a> Iterator for Members<'a> {
    type Item = Result<(&'a [u8], Value<'a>), ErrWhy>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let body = self.body;
        let key_start = skip_ws(body, self.at);
        if key_start >= body.len() {
            self.done = true;
            return self.pending.then_some(Err(PARSE));
        }
        let parsed = (|| {
            if body.get(key_start) != Some(&b'"') {
                return None;
            }
            let key_end = skip_string(body, key_start)?;
            let colon = skip_ws(body, key_end);
            if body.get(colon) != Some(&b':') {
                return None;
            }
            let start = skip_ws(body, colon + 1);
            let end = skip_value(body, start)?;
            let key = body.get(key_start + 1..key_end - 1)?;
            let value = body.get(start..end)?;
            Some((key, value, end))
        })();
        let Some((key, value, end)) = parsed else {
            self.done = true;
            return Some(Err(PARSE));
        };
        let after = skip_ws(body, end);
        match body.get(after) {
            None => self.done = true,
            Some(b',') => {
                self.at = after + 1;
                self.pending = true;
            }
            Some(_) => {
                self.done = true;
                return Some(Err(PARSE));
            }
        }
        Some(Ok((key, Value(value))))
    }
}

/// Decode a string body (between the quotes, escapes intact) into `out`,
/// with the rules of `Text::set_lossy`: invalid UTF-8 becomes U+FFFD, C0
/// controls and DEL become a space, and the text is cut on a character
/// boundary once `out` is full. `\uXXXX` and surrogate pairs decode to UTF-8;
/// a lone surrogate is U+FFFD. A malformed escape is `Parse`, even past the
/// cut: the whole message is validated before any of it is applied.
pub fn decode_into<const N: usize>(raw: &[u8], out: &mut Text<N>) -> Result<(), ErrWhy> {
    if !raw.contains(&b'\\') {
        out.set_lossy(raw);
        return Ok(());
    }
    out.clear();
    let mut full = false;
    for chunk in raw.utf8_chunks() {
        let valid = chunk.valid();
        let mut at = 0;
        while let Some(c) = valid.get(at..).and_then(|rest| rest.chars().next()) {
            let (c, used) = if c == '\\' {
                unescape(valid.as_bytes().get(at..).unwrap_or(&[]))?
            } else {
                (c, c.len_utf8())
            };
            at += used;
            let c = if c.is_ascii_control() { ' ' } else { c };
            full = full || !out.push_char(c);
        }
        if !chunk.invalid().is_empty() {
            full = full || !out.push_char(char::REPLACEMENT_CHARACTER);
        }
    }
    Ok(())
}

/// The character an escape starting at `bytes[0] == '\\'` stands for, and the
/// bytes it used.
fn unescape(bytes: &[u8]) -> Result<(char, usize), ErrWhy> {
    let simple = match bytes.get(1).ok_or(PARSE)? {
        b'"' => '"',
        b'\\' => '\\',
        b'/' => '/',
        b'b' => '\u{8}',
        b'f' => '\u{c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        b'u' => {
            let unit = hex4(bytes.get(2..6))?;
            return Ok(match unit {
                0xD800..=0xDBFF => {
                    // A high surrogate needs its low half; alone it is U+FFFD.
                    let low = match bytes.get(6..8) {
                        Some(b"\\u") => hex4(bytes.get(8..12)).ok(),
                        _ => None,
                    };
                    match low {
                        Some(low @ 0xDC00..=0xDFFF) => {
                            let cp = 0x10000 + ((unit as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
                            (char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER), 12)
                        }
                        _ => (char::REPLACEMENT_CHARACTER, 6),
                    }
                }
                0xDC00..=0xDFFF => (char::REPLACEMENT_CHARACTER, 6),
                _ => (char::from_u32(unit as u32).unwrap_or(char::REPLACEMENT_CHARACTER), 6),
            });
        }
        _ => return Err(PARSE),
    };
    Ok((simple, 2))
}

fn hex4(digits: Option<&[u8]>) -> Result<u16, ErrWhy> {
    let mut value = 0u16;
    for &d in digits.ok_or(PARSE)? {
        let nibble = match d {
            b'0'..=b'9' => d - b'0',
            b'a'..=b'f' => d - b'a' + 10,
            b'A'..=b'F' => d - b'A' + 10,
            _ => return Err(PARSE),
        };
        value = (value << 4) | nibble as u16;
    }
    Ok(value)
}

/// Appends to a caller-owned buffer; output that does not fit is dropped and
/// remembered, never a panic.
pub struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> Writer<'a> {
    pub fn new(out: &'a mut [u8]) -> Self {
        Self {
            out,
            len: 0,
            overflow: false,
        }
    }

    /// Bytes written, or 0 when anything was dropped: a truncated message must
    /// never reach the host as if it were whole.
    pub fn finish(self) -> usize {
        if self.overflow {
            0
        } else {
            self.len
        }
    }

    pub fn raw(&mut self, bytes: &[u8]) {
        match self.out.get_mut(self.len..self.len + bytes.len()) {
            Some(slot) => {
                slot.copy_from_slice(bytes);
                self.len += bytes.len();
            }
            None => self.overflow = true,
        }
    }

    /// `"key":`, preceded by a comma unless it opens a container.
    pub fn key(&mut self, key: &str) -> &mut Self {
        if !matches!(self.out.get(self.len.wrapping_sub(1)), Some(b'{' | b'[') | None) {
            self.raw(b",");
        }
        self.string(key);
        self.raw(b":");
        self
    }

    /// A quoted JSON string with `"`, `\` and control characters escaped;
    /// valid UTF-8 passes through unchanged.
    pub fn string(&mut self, s: &str) {
        self.raw(b"\"");
        for &byte in s.as_bytes() {
            match byte {
                b'"' => self.raw(b"\\\""),
                b'\\' => self.raw(b"\\\\"),
                0..=0x1F | 0x7F => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    self.raw(b"\\u00");
                    self.raw(&[HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]]);
                }
                _ => self.raw(&[byte]),
            }
        }
        self.raw(b"\"");
    }

    pub fn number(&mut self, mut value: u32) {
        let mut digits = [0; 10];
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.raw(&digits[at..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text<const N: usize>(json: &str) -> Result<Text<N>, ErrWhy> {
        let mut out = Text::new();
        Value(json.as_bytes()).text_into(&mut out)?;
        Ok(out)
    }

    #[test]
    fn writer_escapes_and_reports_overflow_as_nothing() {
        let mut buffer = [0; 40];
        let mut writer = Writer::new(&mut buffer);
        writer.string("a\"b\\c\n\u{7f}");
        writer.number(0);
        writer.number(4_294_967_295);
        let n = writer.finish();
        assert_eq!(&buffer[..n], b"\"a\\\"b\\\\c\\u000a\\u007f\"04294967295");
        let mut small = [0; 4];
        let mut writer = Writer::new(&mut small);
        writer.string("toolong");
        assert_eq!(writer.finish(), 0);
    }

    #[test]
    fn writer_keys_take_their_comma_from_context() {
        let mut buffer = [0; 64];
        let mut writer = Writer::new(&mut buffer);
        writer.raw(b"{");
        writer.key("a").number(1);
        writer.key("b").raw(b"[");
        writer.raw(b"{");
        writer.key("c").string("d");
        writer.raw(b"}]}");
        let n = writer.finish();
        assert_eq!(&buffer[..n], br#"{"a":1,"b":[{"c":"d"}]}"#);
    }

    #[test]
    fn object_members_and_lookup() {
        let object = Object::parse(br#" {"m":"hello", "v" : 1,"list":[1, {"x":"}"}],"s":"a,b:c"} "#).unwrap();
        assert!(object.get("m").unwrap().unwrap().is_str("hello"));
        assert_eq!(object.get("v").unwrap().unwrap().u32(), Ok(1));
        assert!(object.get("s").unwrap().unwrap().is_str("a,b:c"));
        assert!(object.get("missing").unwrap().is_none());
        assert_eq!(object.need("missing").err(), Some(ErrWhy::Parse));
        let items: std::vec::Vec<_> = object
            .need("list")
            .unwrap()
            .array()
            .unwrap()
            .map(|item| item.map(|v| v.0.to_vec()))
            .collect();
        assert_eq!(items, [Ok(b"1".to_vec()), Ok(br#"{"x":"}"}"#.to_vec())]);
        assert_eq!(object.members().count(), 4);
        assert_eq!(Object::parse(b"{}").unwrap().members().count(), 0);
    }

    #[test]
    fn malformed_objects_and_arrays_are_parse_errors_not_panics() {
        for line in [
            &b""[..],
            b"{",
            b"}",
            b"{\"a\":1}x",
            b"{\"a\"}",
            b"{\"a\":}",
            b"{a:1}",
            b"{\"a\":1,}",
            b"{\"a\":1 \"b\":2}",
            b"{\"a\":\"unterminated}",
            b"{\"a\":\"esc\\\"}",
            b"[1]",
            b"\"s\"",
            b"{\"a\":1}{\"b\":2}",
            b"{\"a\":[1,,2]}",
            b"{\"a\":[1,]}",
            b"{\"a\":[1, ]}",
            b"{\"a\":1, }",
            b"{\"a\":[1 2]}",
            b"{\"a\":[}",
        ] {
            let result = Object::parse(line).and_then(|o| {
                for member in o.members() {
                    let (_, value) = member?;
                    if let Ok(items) = value.array() {
                        for item in items {
                            item?;
                        }
                    }
                }
                Ok(())
            });
            assert_eq!(result.err(), Some(ErrWhy::Parse), "{:?}", core::str::from_utf8(line));
        }
        // Deep nesting is iterative: no stack to blow.
        let mut deep = std::vec::Vec::new();
        deep.extend_from_slice(b"{\"a\":");
        deep.extend(core::iter::repeat_n(b'[', 2000));
        deep.extend(core::iter::repeat_n(b']', 2000));
        deep.push(b'}');
        assert!(Object::parse(&deep).is_ok());
        deep.pop();
        assert_eq!(Object::parse(&deep).err(), Some(ErrWhy::Parse));
    }

    #[test]
    fn numbers_and_bools_validate_their_shape() {
        assert_eq!(Value(b"0").u32(), Ok(0));
        assert_eq!(Value(b"4294967295").u32(), Ok(4294967295));
        assert_eq!(Value(b"4294967296").u32(), Err(ErrWhy::Range));
        assert_eq!(Value(b"-1").u32(), Err(ErrWhy::Range));
        assert_eq!(Value(b"1.5").u32(), Err(ErrWhy::Range));
        assert_eq!(Value(b"1e3").u32(), Err(ErrWhy::Range));
        assert_eq!(Value(b"\"1\"").u32(), Err(ErrWhy::Parse));
        assert_eq!(Value(b"true").u32(), Err(ErrWhy::Parse));
        assert_eq!(Value(b"").u32(), Err(ErrWhy::Parse));
        assert_eq!(Value(b"256").u8(), Err(ErrWhy::Range));
        assert_eq!(Value(b"255").u8(), Ok(255));
        assert_eq!(Value(b"65536").u16(), Err(ErrWhy::Range));
        assert_eq!(Value(b"true").bool(), Ok(true));
        assert_eq!(Value(b"false").bool(), Ok(false));
        assert_eq!(Value(b"1").bool(), Ok(true));
        assert_eq!(Value(b"0").bool(), Ok(false));
        assert_eq!(Value(b"\"yes\"").bool(), Err(ErrWhy::Parse));
        assert_eq!(Value(b"null").bool(), Err(ErrWhy::Parse));
        assert_eq!(Value(b"[1]").array().unwrap().count(), 1);
        assert_eq!(Value(b"[]").array().unwrap().count(), 0);
        assert_eq!(Value(b"1").array().err(), Some(ErrWhy::Parse));
        assert_eq!(Value(b"1").object().err(), Some(ErrWhy::Parse));
        assert_eq!(Value(b"[1]").raw_str(), None);
    }

    #[test]
    fn enum_members_distinguish_unknown_from_malformed() {
        const TABLE: &[(&str, u8)] = &[("a", 1), ("b", 2)];
        assert_eq!(Value(b"\"b\"").member(TABLE), Ok(2));
        assert_eq!(Value(b"\"c\"").member(TABLE), Err(ErrWhy::Unknown));
        assert_eq!(Value(b"2").member(TABLE), Err(ErrWhy::Parse));
        assert_eq!(Value(b"\"\\u0061\"").member(TABLE), Err(ErrWhy::Unknown));
    }

    #[test]
    fn strings_decode_escapes_surrogates_and_raw_utf8() {
        assert_eq!(text::<32>(r#""plain""#).unwrap(), "plain");
        assert_eq!(text::<32>(r#""a\"b\\c\/d""#).unwrap(), "a\"b\\c/d");
        assert_eq!(text::<32>(r#""x\ny\tz\b\f\r""#).unwrap(), "x y z   ");
        assert_eq!(text::<32>(r#""\u00e9\u2026""#).unwrap(), "\u{e9}\u{2026}");
        assert_eq!(text::<32>(r#""\u00E9""#).unwrap(), "\u{e9}");
        assert_eq!(text::<32>(r#""\ud83d\ude00!""#).unwrap(), "\u{1F600}!");
        assert_eq!(text::<32>(r#""\ud83d""#).unwrap(), "\u{FFFD}");
        assert_eq!(text::<32>(r#""\ude00""#).unwrap(), "\u{FFFD}");
        assert_eq!(text::<32>(r#""\ud83d\u0041""#).unwrap(), "\u{FFFD}A");
        assert_eq!(text::<32>(r#""\ud83dx""#).unwrap(), "\u{FFFD}x");
        assert_eq!(text::<32>(r#""\u0000\u001f\u007f""#).unwrap(), "   ");
        assert_eq!(text::<32>("\"caf\u{e9} \u{1F600}\"").unwrap(), "caf\u{e9} \u{1F600}");
        assert_eq!(text::<32>("\"raw\ttab\"").unwrap(), "raw tab");
        let mut out = Text::<32>::new();
        assert_eq!(decode_into(b"a\x80b\\u00e9\xff", &mut out), Ok(()));
        assert_eq!(out, "a\u{FFFD}b\u{e9}\u{FFFD}");
        assert_eq!(decode_into(b"\\u00", &mut out), Err(ErrWhy::Parse));
        for bad in [r#""\x""#, r#""\""#, r#""\u12""#, r#""\u12G4""#, r#""\ud83d\u""#, r#""a\""#] {
            assert_eq!(text::<32>(bad).err(), Some(ErrWhy::Parse), "{bad}");
        }
        assert_eq!(text::<32>("x").err(), Some(ErrWhy::Parse));
    }

    #[test]
    fn strings_truncate_on_a_character_boundary_and_keep_validating() {
        assert_eq!(text::<6>(r#""abcdefgh""#).unwrap(), "abcdef");
        assert_eq!(text::<6>(r#""abcde\u00e9x""#).unwrap(), "abcde");
        assert_eq!(text::<6>(r#""abcd\u00e9x""#).unwrap(), "abcd\u{e9}");
        assert_eq!(text::<6>(r#""abc\ud83d\ude00""#).unwrap(), "abc");
        assert_eq!(text::<6>(r#""ab\ud83d\ude00x""#).unwrap(), "ab\u{1F600}");
        assert_eq!(text::<6>("\"abcde\u{e9}x\"").unwrap(), "abcde");
        // Past the cut, escapes are still checked (the message is all or nothing).
        assert_eq!(text::<3>(r#""abcdef\x""#).err(), Some(ErrWhy::Parse));
        assert_eq!(text::<3>(r#""abcdef\n""#).unwrap(), "abc");
        assert_eq!(text::<0>(r#""abc\n""#).unwrap(), "");
    }
}
