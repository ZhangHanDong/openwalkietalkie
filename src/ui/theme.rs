//! Every colour, the type ramp and the screen-level layout constants of the
//! deck faces. Colours are RGB888 sampled from the mockups (`mock/*.png`, most
//! frequent pixel of each flat surface); faces never invent colours or sizes.
//!
//! Near-blacks collapse in RGB565 (#0B0B0D and #0A0A0C are the same pixel), so
//! there is one card fill, not one per mockup.

// ---- surfaces ---------------------------------------------------------------
pub const CANVAS: u32 = 0x000000;
/// Selected roster/choice/window row, macro key, selected chip.
pub const CARD_FILL: u32 = 0x0B0B0D;
/// Face-manager thumbnail cards.
pub const THUMB_FILL: u32 = 0x050506;
/// Number chip in a choice row, usage bar track.
pub const CHIP_FILL: u32 = 0x17181B;
/// Schematic bars inside manager thumbnails.
pub const THUMB_BAR: u32 = 0x1D1F24;
pub const HINT_FILL: u32 = 0x040405;
/// One-pixel line on top of the hint bar (y = 427).
pub const HAIRLINE: u32 = 0x151619;
pub const KEYCAP_FILL: u32 = 0x101114;

// ---- outlines ---------------------------------------------------------------
/// Idle rows, choices and workspace chips.
pub const OUTLINE: u32 = 0x26282C;
/// Macro keys (solid and dashed) and keycaps.
pub const OUTLINE_KEY: u32 = 0x2C2E33;
/// Idle and dashed manager cards.
pub const OUTLINE_THUMB: u32 = 0x3A3D44;
/// The one selected element of a view. (#E8E9EA on the roster and window
/// rows, #FFFFFF on the question card and manager; one constant serves both.)
pub const OUTLINE_SELECTED: u32 = 0xE8E9EA;

// ---- text -------------------------------------------------------------------
pub const TEXT_PRIMARY: u32 = 0xFFFFFF;
/// Macro key labels, selected-thumbnail accents.
pub const TEXT_BRIGHT: u32 = 0xE8E9EA;
/// Unselected choice and window rows.
pub const TEXT_ROW: u32 = 0xC9CCD1;
/// Roster titles, idle chip numbers, thumbnail labels.
pub const TEXT_SECONDARY: u32 = 0x9AA0A6;
/// Header labels and tags, hint text, dashed key labels, reset lines.
pub const TEXT_LABEL: u32 = 0x8E9299;
/// Section labels ("WINDOWS"), "Last reply", the macropad source tag.
pub const TEXT_FAINT: u32 = 0x71767D;
pub const KEYCAP_TEXT: u32 = 0xCFD2D6;
/// The "+" of the manager's add card.
pub const PLUS: u32 = 0x5C6370;

// ---- state colours ----------------------------------------------------------
pub const RED: u32 = 0xFF5C5C;
pub const GREEN: u32 = 0x3DDC84;
pub const BLUE: u32 = 0x4C9AFF;
pub const AMBER: u32 = 0xFFB020;
pub const GRAY: u32 = 0x4A4D55;
/// LISTENING pill (mockup 7): dark green fill, mid green outline, GREEN text.
pub const LISTENING_FILL: u32 = 0x0A1F16;
pub const LISTENING_OUTLINE: u32 = 0x1E5C3F;

use crate::deck::msg::{AgentState, Tone};

pub const fn tone(tone: Tone) -> u32 {
    match tone {
        Tone::Blue => BLUE,
        Tone::Amber => AMBER,
        Tone::Green => GREEN,
        Tone::Red => RED,
        Tone::Gray => GRAY,
    }
}

/// Roster dot. A stale or offline agent is gray whatever its state.
pub const fn agent_state(state: AgentState, stale: bool) -> u32 {
    if stale {
        return GRAY;
    }
    match state {
        AgentState::Working => BLUE,
        AgentState::Question => RED,
        AgentState::Ready => GREEN,
        AgentState::Error => AMBER,
        AgentState::Idle => GRAY,
    }
}

/// Usage bar tone when the host names none.
pub const fn usage_tone(percent: u8) -> u32 {
    match percent {
        0..=49 => BLUE,
        50..=79 => GREEN,
        80..=94 => AMBER,
        _ => RED,
    }
}

// ---- type ramp --------------------------------------------------------------
/// Legibility-scaled ~1.33x from the mockups (11-16 px type is under 1.3 mm on
/// this 38.8 mm panel); the mockup layout rectangles are kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    Display,
    Title,
    Name,
    Macro,
    Text,
    Row,
    Body,
    Small,
    Label,
    Hint,
    Keycap,
    RowStrong,
    TextStrong,
    Reading,
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Regular,
    SemiBold,
    Bold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FontSpec {
    pub font: Font,
    /// Em size in pixels.
    pub px: u8,
    pub weight: Weight,
    /// Extra advance per glyph in half pixels (Label is tracked +1.5 px).
    pub tracking_half_px: u8,
    /// Rendered in capitals by convention (callers pass upper-case text).
    pub caps: bool,
}

const fn spec(font: Font, px: u8, weight: Weight) -> FontSpec {
    FontSpec {
        font,
        px,
        weight,
        tracking_half_px: 0,
        caps: false,
    }
}

/// The ONE size table; `tools/generate-ui-fonts.py` must agree with it.
/// Indexed by `Font as usize`.
pub const RAMP: [FontSpec; 15] = [
    spec(Font::Display, 48, Weight::Bold),
    spec(Font::Title, 32, Weight::SemiBold),
    spec(Font::Name, 27, Weight::SemiBold),
    spec(Font::Macro, 21, Weight::SemiBold),
    spec(Font::Text, 29, Weight::Regular),
    spec(Font::Row, 25, Weight::Regular),
    spec(Font::Body, 22, Weight::Regular),
    spec(Font::Small, 18, Weight::Regular),
    FontSpec {
        font: Font::Label,
        px: 18,
        weight: Weight::Bold,
        tracking_half_px: 3,
        caps: true,
    },
    spec(Font::Hint, 15, Weight::Regular),
    spec(Font::Keycap, 13, Weight::SemiBold),
    spec(Font::RowStrong, 25, Weight::SemiBold),
    spec(Font::TextStrong, 29, Weight::SemiBold),
    spec(Font::Reading, 32, Weight::Regular),
    spec(Font::Code, 25, Weight::Regular),
];

pub const fn font_spec(font: Font) -> FontSpec {
    RAMP[font as usize]
}

// ---- screen-level layout ----------------------------------------------------
pub const SCREEN: i32 = 480;
/// Radius of the panel's rounded corners; content stays inside the arcs.
pub const CORNER_R: i32 = 74;
/// Left edge of cards, rows and keys; their right edge is `SCREEN - MARGIN`.
pub const MARGIN: i32 = 32;
/// Left edge of free-standing text (header label, titles, usage names).
pub const TEXT_X: i32 = 38;
/// Right edge of right-aligned text (clock, tags, percentages).
pub const TEXT_RIGHT: i32 = 442;
pub const HEADER_BASELINE: i32 = 50;
/// First row of the hint bar fill; the hairline sits on the row above.
pub const HINT_Y: i32 = 428;
pub const HINT_BASELINE: i32 = 458;
/// Keycaps in the hint bar span y 444..464.
pub const KEYCAP_Y: i32 = 444;
pub const KEYCAP_H: i32 = 20;
/// Corner radius of rows and cards / macro keys / chips and keycaps.
pub const CARD_R: u32 = 14;
pub const KEY_R: u32 = 16;
pub const CHIP_R: u32 = 8;
pub const KEYCAP_R: u32 = 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_indexed_by_font_and_matches_the_contract() {
        for (index, spec) in RAMP.iter().enumerate() {
            assert_eq!(spec.font as usize, index);
            assert_eq!(font_spec(spec.font), *spec);
        }
        let px = RAMP.map(|spec| spec.px);
        assert_eq!(px, [48, 32, 27, 21, 29, 25, 22, 18, 18, 15, 13, 25, 29, 32, 25]);
        assert!(font_spec(Font::Label).caps);
    }

    #[test]
    fn usage_tones_follow_the_thresholds_and_the_mockup() {
        assert_eq!(usage_tone(12), BLUE);
        assert_eq!(usage_tone(62), GREEN);
        assert_eq!(usage_tone(81), AMBER);
        assert_eq!((usage_tone(49), usage_tone(50)), (BLUE, GREEN));
        assert_eq!((usage_tone(79), usage_tone(80)), (GREEN, AMBER));
        assert_eq!((usage_tone(94), usage_tone(95), usage_tone(255)), (AMBER, RED, RED));
    }

    #[test]
    fn stale_agents_are_gray() {
        assert_eq!(agent_state(AgentState::Question, false), RED);
        assert_eq!(agent_state(AgentState::Question, true), GRAY);
        assert_eq!(agent_state(AgentState::Error, false), AMBER);
    }
}
