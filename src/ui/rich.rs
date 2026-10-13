//! Compact, host-laid-out rich lines. Header: block kind + indent digit.
//! U+E000 followed by A..U+00C0 changes inline flags and code token colors.
//! Every line is independent; untrusted control bytes never become commands.
use super::{font, font::Align, theme, theme::Font};
use crate::display::Band;
use embedded_graphics::{pixelcolor::Rgb888, prelude::*, primitives::{PrimitiveStyle, Rectangle}};

pub const BOLD: u8 = 1;
pub const CODE: u8 = 2;
pub const LINK: u8 = 4;
pub const EMPHASIS: u8 = 8;
pub const SYNTAX_MASK: u8 = 0x70;
pub const KEYWORD: u8 = 0x10;
pub const STRING: u8 = 0x20;
pub const NUMBER: u8 = 0x30;
pub const COMMENT: u8 = 0x40;
pub const FUNCTION: u8 = 0x50;
pub const TYPE: u8 = 0x60;
pub const ESC: char = '\u{e000}';

/// Tool disclosure row: t/T, indent digit, two hex digits naming a message.
/// The message index is valid only within the enclosing document revision.
pub fn tool(line: &str) -> Option<(u8, bool)> {
    let b = line.as_bytes();
    if b.len() < 4 || !matches!(b[0], b't' | b'T') || b[1] != b'0' { return None; }
    let hex = |b: u8| match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), _ => None };
    let index = hex(b[2])? * 16 + hex(b[3])?;
    (index <= 65).then_some((index, b[0] == b'T'))
}

/// A bounded plan progress row, carrying completed/total as two hex bytes.
pub fn progress(line: &str) -> Option<(u8, u8)> {
    if !line.starts_with("w0") || line.len() != 6 { return None; }
    let bytes = line.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
    let done = hex(bytes[2])? * 16 + hex(bytes[3])?;
    let total = hex(bytes[4])? * 16 + hex(bytes[5])?;
    (total > 0 && total <= 64 && done <= total).then_some((done, total))
}

pub fn header(line: &str) -> (u8, i32, &str) {
    if progress(line).is_some() { return (b'w', 0, ""); }
    if tool(line).is_some() { return (line.as_bytes()[0], 24, &line[4..]); }
    let b = line.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'p' | b'h' | b'c' | b'q' | b'r' | b'b') && b[1].is_ascii_digit() {
        (b[0], (b[1] - b'0') as i32 * 12, &line[2..])
    } else { (b'p', 0, line) }
}

pub fn runs(mut text: &str, mut visit: impl FnMut(u8, &str)) {
    let mut flags = 0;
    while !text.is_empty() {
        let end = text.find(ESC).unwrap_or(text.len());
        if end != 0 { visit(flags, &text[..end]); }
        text = &text[end..];
        if text.is_empty() { break; }
        text = &text[ESC.len_utf8()..];
        if let Some(c) = text.chars().next() {
            if ('A'..='\u{c0}').contains(&c) { flags = c as u8 - b'A'; }
            text = &text[c.len_utf8()..];
        }
    }
}

pub fn face(base: Font, flags: u8) -> Font {
    if flags & CODE != 0 { Font::Code }
    else if flags & BOLD != 0 {
        match base { Font::Row => Font::RowStrong, Font::Text => Font::TextStrong, _ => Font::Title }
    } else { base }
}

pub fn measure(base: Font, line: &str) -> i32 {
    let (_, indent, body) = header(line);
    let mut width = indent;
    runs(body, |flags, text| width += font::measure(face(base, flags), text));
    width
}

fn fill(band: &mut Band, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let rgb = Rgb888::new((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let _ = Rectangle::new(Point::new(x, y), Size::new(w.max(0) as u32, h.max(0) as u32))
        .into_styled(PrimitiveStyle::with_fill(rgb.into())).draw(band);
}

pub fn draw(band: &mut Band, base: Font, x: i32, baseline: i32, width: i32, line: &str) {
    let (kind, indent, body) = header(line);
    let top = baseline - font::ascent(base);
    let height = font::line_height(base);
    if let Some((done, total)) = progress(line) {
        fill(band, x, top + height / 2, width, 5, theme::OUTLINE);
        fill(band, x, top + height / 2, width * done as i32 / total as i32, 5, theme::BLUE);
        return;
    }
    let disclosure = tool(line);
    let bg = if kind == b'c' { theme::OUTLINE } else if disclosure.is_some() { theme::CHIP_FILL } else { theme::CANVAS };
    if kind == b'c' || disclosure.is_some() { fill(band, x, top, width, height, bg); }
    if let Some((_, expanded)) = disclosure {
        let cy = top + height / 2;
        for i in 0..5 {
            if expanded {
                fill(band, x + 6 + i, cy - 2 + i, 2, 2, theme::TEXT_SECONDARY);
                fill(band, x + 14 - i, cy - 2 + i, 2, 2, theme::TEXT_SECONDARY);
            } else {
                fill(band, x + 7 + i, cy - 4 + i, 2, 2, theme::TEXT_SECONDARY);
                fill(band, x + 7 + i, cy + 4 - i, 2, 2, theme::TEXT_SECONDARY);
            }
        }
    }
    if kind == b'q' { fill(band, x, top + 3, 2, height - 6, theme::BLUE); }
    if kind == b'r' { fill(band, x, top + height / 2, width, 1, theme::OUTLINE_THUMB); }
    let mut pen = indent;
    runs(body, |flags, text| {
        let color = if flags & CODE != 0 && flags & SYNTAX_MASK != 0 {
            match flags & SYNTAX_MASK {
                KEYWORD => 0xC792EA,
                STRING => theme::GREEN,
                NUMBER => theme::AMBER,
                COMMENT => 0x8B98A5,
                FUNCTION => theme::BLUE,
                TYPE => 0x89DDFF,
                _ => theme::TEXT_ROW,
            }
        } else if flags & LINK != 0 { theme::BLUE }
            else if flags & (BOLD | EMPHASIS) != 0 { theme::TEXT_PRIMARY }
            else { theme::TEXT_ROW };
        let advance = font::draw(band, face(base, flags), x + pen, baseline, text, color, Some(bg), width - pen, Align::Left);
        if flags & EMPHASIS != 0 { fill(band, x + pen, baseline + 2, advance, 1, color); }
        pen += advance;
    });
}

#[cfg(test)]
mod workflow_tests {
    use super::*;
    #[test]
    fn progress_rows_validate_counts_and_utf8_without_panicking() {
        assert_eq!(progress("w00004"), Some((0, 4)));
        assert_eq!(progress("w00404"), Some((4, 4)));
        for invalid in ["w00504", "w00000", "w000ff", "w0界x", "w0😀", "w0zz01"] {
            assert_eq!(progress(invalid), None);
        }
        assert_eq!(tool("t040Full plan"), Some((64, false)));
        assert_eq!(tool("T041Recent tools"), Some((65, true)));
        assert_eq!(tool("t042Invalid"), None);
    }
}
