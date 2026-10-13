//! The **firmware's own** text wrapping, mounted for the host.
//!
//! The deck no longer wraps a question body it was sent: the host measures the
//! text, breaks it into lines and ships a window of them (`HostLine::CardText`).
//! That only works if the two sides break lines in exactly the same places, so
//! this crate does not reimplement the algorithm — it mounts
//! `src/ui/font.rs` and the glyph atlas it measures with, the way `native/core`
//! and `deck-proto`'s contract tests already mount firmware source.
//!
//! Two wrap implementations that can disagree is the defect the whole
//! host-side-layout design exists to avoid, and this project has been bitten by
//! that exact shape before (week 5: a resolver and its tests sharing the same
//! wrong helper, so the tests could not see the bug).
//!
//! The mounted files keep the firmware tree's formatting and lint profile,
//! hence the skips. The module names are not free: the firmware files name
//! `crate::display` and `crate::deck::msg` absolutely, so this crate's root has
//! to offer them under those names — which is why the mount lives in a crate of
//! its own rather than inside `deck-proto`.

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../src/display/orientation.rs"]
mod orientation;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../src/display/framebuffer.rs"]
mod display;

mod deck;
mod ui;
mod rich;
mod syntax;
pub use rich::{activity_row, tool_disclosure, tool_header, wrap_rich_reply};
pub use ui::rich::tool as tool_entry;

use deck_proto::{DocFont, DocKind};
use ui::theme::Font;

/// The face the `text` size setting selects, and how many lines of each lane
/// it shows at once.
///
/// **These are the firmware's own numbers**, read from the mounted
/// `deck::msg::DocFont` rather than copied: `for_size`, `reply_lines` and
/// `question_lines` live in firmware source precisely so the host and the deck
/// cannot end up with different answers. `fw_font` maps the wire's `DocFont`
/// onto the mounted one, and `the_two_docfonts_agree` pins the two enums
/// together.
fn fw_font(font: DocFont) -> deck::msg::DocFont {
    match font {
        DocFont::Title => deck::msg::DocFont::Title,
        DocFont::Name => deck::msg::DocFont::Name,
        DocFont::Row => deck::msg::DocFont::Row,
        DocFont::RichRow => deck::msg::DocFont::RichRow,
        DocFont::RichText => deck::msg::DocFont::RichText,
        DocFont::RichTitle => deck::msg::DocFont::RichTitle,
        DocFont::Text => deck::msg::DocFont::Text,
    }
}

fn wire_font(font: deck::msg::DocFont) -> DocFont {
    match font {
        deck::msg::DocFont::Title => DocFont::Title,
        deck::msg::DocFont::Name => DocFont::Name,
        deck::msg::DocFont::Row => DocFont::Row,
        deck::msg::DocFont::RichRow => DocFont::RichRow,
        deck::msg::DocFont::RichText => DocFont::RichText,
        deck::msg::DocFont::RichTitle => DocFont::RichTitle,
        deck::msg::DocFont::Text => DocFont::Text,
    }
}

/// The reading face for a `text` size setting (0 small, 1 medium, 2 large).
pub fn face_for_size(size: u8) -> DocFont {
    wire_font(deck::msg::DocFont::for_size(size))
}

/// Lines of a **reply** (and of the transcript, which shares the lane) the deck
/// shows at once in this face.
pub fn reply_visible(font: DocFont) -> usize {
    fw_font(font).reply_lines()
}

/// Lines of a **question body** the card lane shows at once in this face.
pub fn question_visible(font: DocFont) -> usize {
    fw_font(font).question_lines()
}

/// How many lines of `kind` the deck shows at once in `font`.
pub fn visible(kind: DocKind, font: DocFont) -> usize {
    match kind {
        DocKind::Card => question_visible(font),
        DocKind::Reply | DocKind::Tr => reply_visible(font),
    }
}

/// The lane each document is wrapped to, read from the firmware's own layout
/// rather than copied as a number.
pub fn lane_width(kind: DocKind) -> i32 {
    match kind {
        DocKind::Card => ui::layout::CARD_QUESTION.width(),
        DocKind::Reply | DocKind::Tr => ui::layout::CHANNEL_REPLY.width(),
    }
}

/// The card question lane, kept for the callers that name it directly.
pub fn question_width() -> i32 {
    lane_width(DocKind::Card)
}

fn face(font: DocFont) -> Font {
    match font {
        DocFont::Title => Font::Title,
        DocFont::Name => Font::Name,
        DocFont::Row => Font::Row,
        DocFont::Text | DocFont::RichText => Font::Text,
        DocFont::RichRow => Font::Row,
        DocFont::RichTitle => Font::Reading,
    }
}

fn wrap_all(font: DocFont, text: &str, width: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    ui::font::wrap(face(font), text, width, usize::MAX, &mut |_, line, _| {
        lines.push(line.to_string());
    });
    lines
}

/// Break `text` the way the deck would in the **card question** lane, and say
/// which font it was measured with.
///
/// The font rule is the firmware's: a body that fits two `Title` lines is drawn
/// in `Title`, anything longer drops to `Name`. The deck applies the same rule
/// only when it has no window; once the host sends one, the deck draws with the
/// `font` it is told, which is this one.
///
/// Unlike the deck's own call there is **no line cap** — the whole body is
/// wrapped. Bounding the result is the wire's job, not the layout's.
pub fn wrap_question(text: &str) -> (DocFont, Vec<String>) {
    let width = question_width();
    let mut title: Vec<String> = Vec::new();
    let mut cut = false;
    ui::font::wrap(
        Font::Title,
        text,
        width,
        question_visible(DocFont::Title),
        &mut |_, line, forced| {
            title.push(line.to_string());
            cut |= forced;
        },
    );
    if !cut {
        return (DocFont::Title, title);
    }
    (DocFont::Name, wrap_all(DocFont::Name, text, width))
}

/// Break `text` the way the deck would in the **channel reply** lane, which is
/// also where the live transcript is read, at the operator's chosen size.
pub fn wrap_reply(text: &str, size: u8) -> (DocFont, Vec<String>) {
    let font = face_for_size(size);
    (font, wrap_all(font, text, lane_width(DocKind::Reply)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lanes are read from the firmware's layout, not copied. This only
    /// pins that they are being read at all, and are plausible widths.
    #[test]
    fn the_lanes_are_the_decks_own() {
        assert_eq!(question_width(), 406);
        assert_eq!(lane_width(DocKind::Reply), 404);
    }

    /// The wire's `DocFont` and the firmware's must map onto each other both
    /// ways, or the host measures with one face and the deck draws with
    /// another.
    #[test]
    fn the_two_docfonts_agree() {
        for font in [DocFont::Title, DocFont::Name, DocFont::Row, DocFont::Text, DocFont::RichRow, DocFont::RichText, DocFont::RichTitle] {
            assert_eq!(wire_font(fw_font(font)), font);
        }
        // And the size ramp is the firmware's, read not copied.
        assert_eq!(face_for_size(0), DocFont::Row);
        assert_eq!(face_for_size(1), DocFont::Text);
        assert_eq!(face_for_size(2), DocFont::Title);
        assert_eq!(
            face_for_size(9),
            DocFont::Title,
            "out of range clamps large"
        );
    }

    /// Bigger text means fewer lines on screen and more lines in the body, and
    /// both sides have to agree on both numbers.
    #[test]
    fn a_bigger_size_shows_fewer_lines_of_more() {
        let body = "word ".repeat(300);
        let mut last_visible = usize::MAX;
        let mut last_total = 0;
        for size in 0..=2u8 {
            let (font, lines) = wrap_reply(&body, size);
            let visible = reply_visible(font);
            assert!(visible <= last_visible, "size {size}: {visible} on screen");
            assert!(
                lines.len() >= last_total,
                "size {size}: {} lines",
                lines.len()
            );
            last_visible = visible;
            last_total = lines.len();
        }
    }

    #[test]
    fn a_short_question_stays_in_the_big_font() {
        let (font, lines) = wrap_question("Ship it?");
        assert_eq!(font, DocFont::Title);
        assert_eq!(lines, ["Ship it?"]);
    }

    /// Two Title lines is the boundary: the third line is what drops the whole
    /// body to `Name`, exactly as `faces::agents::question_font` does it.
    #[test]
    fn a_long_question_drops_to_the_small_font_and_is_not_cut() {
        let text = "Claude wants to run `rm -rf /tmp/build` in the project \
                    directory. This will delete the whole build tree and cannot \
                    be undone. Allow this command to run?";
        let (font, lines) = wrap_question(text);
        assert_eq!(font, DocFont::Name);
        assert!(
            lines.len() > question_visible(font),
            "got {} lines",
            lines.len()
        );
        let rejoined = lines.join(" ");
        let got: Vec<&str> = rejoined.split_whitespace().collect();
        let want: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(got, want, "the wrap must not lose or add a word");
        assert!(!rejoined.contains('\u{2026}'), "no ellipsis in a host wrap");
    }

    /// The reply lane has one face and no cap. A real agent reply measured
    /// 2347 characters against `CAPS.reply`'s 200, which is the ellipsis this
    /// exists to remove.
    #[test]
    fn a_long_reply_is_wrapped_whole_in_the_reply_face() {
        let text = "word ".repeat(470);
        let (font, lines) = wrap_reply(&text, 0);
        assert_eq!(font, DocFont::Row);
        assert!(
            lines.len() > reply_visible(font) * 4,
            "2347 chars should be many screens, got {} lines",
            lines.len()
        );
        assert_eq!(
            lines.join(" ").split_whitespace().count(),
            470,
            "every word survives"
        );
    }

    #[test]
    fn there_is_no_line_cap() {
        let (_, lines) = wrap_question(&"word ".repeat(400));
        assert!(lines.len() > 40, "got {} lines", lines.len());
    }

    #[test]
    fn explicit_newlines_break_lines() {
        let (_, lines) = wrap_question("one\ntwo\nthree\nfour\nfive");
        assert_eq!(lines, ["one", "two", "three", "four", "five"]);
    }

    #[test]
    fn an_empty_body_wraps_to_nothing() {
        let (font, lines) = wrap_question("");
        assert_eq!(font, DocFont::Title);
        assert!(lines.is_empty());
        assert!(wrap_reply("", 1).1.is_empty());
    }
}
