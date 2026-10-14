//! The global LISTENING overlay (mockup 7): the latched agent, the pill, a
//! synthetic waveform, the live transcript in straight quotes with a caret
//! (its last six lines, behind a "…", once it outgrows the box), and one line
//! of context ("Last reply · …", the question being answered, or what the
//! naming session is for). Opaque: it starts with a canvas fill and owns
//! every pixel, so the faces beneath are neither drawn nor signed.
//!
//! The waveform is synthesised, not mic-driven (the mic stays on the host):
//! 23 bars under the mockup's envelope, each scaled by a hash of (bar, phase,
//! agent) where the phase is `wave_phase(now_ms)`; phase 0 (the first frame
//! of a session) is the envelope itself. The grammar reports a tick dirty
//! when the phase changes; only the waveform lane repaints. Without a deck
//! host (and not demo) the pill reads NO HOST and the bars lie flat and gray.
//!
//! Five fixed lanes: title (name + pill), waveform, transcript, context and
//! the hint bar, each hashed by exactly what it draws.
use crate::deck::model::Agent;
use crate::deck::msg::PttCtx;
use crate::deck::text::Text;
use crate::display::Band;
use crate::ui::faces::agents::{kind_name, offline, text, HINT_LANE};
use crate::ui::kit::Hint;
use crate::ui::signature::{Hasher, SigBuilder};
use crate::ui::state::{Overlay, PttSession, UiState};
use crate::ui::{font, font::Align, font::Font, kit, layout, layout::Rect, theme};

pub const CHANNEL_TIP: &str = "Tip: enable Channels in Mac Integrations";

pub const HINTS: [Hint<'static>; 2] = [Hint { key: "", label: "release" }, Hint { key: "PTT", label: "to send" }];
/// NO HOST: nothing will be sent, the release only ends the session.
pub const NO_HOST_HINTS: [Hint<'static>; 2] =
    [Hint { key: "", label: "no host \u{b7} release" }, Hint { key: "PTT", label: "to end" }];
pub const TRANSCRIBING_HINTS: [Hint<'static>; 2] =
    [Hint { key: "PTT", label: "talk again" }, Hint { key: "ESC", label: "cancel" }];

/// The hint bar of a session: what the release will do.
fn hints(live: bool, pending: bool) -> &'static [Hint<'static>] {
    if pending {
        &TRANSCRIBING_HINTS
    } else if live {
        &HINTS
    } else {
        &NO_HOST_HINTS
    }
}

/// Name and pill.
pub const TITLE_LANE: Rect = Rect::new(32, 30, 416, 40);
/// The wrapped transcript and its caret (which may pass the text box's edge).
pub const TRANSCRIPT_LANE: Rect = Rect::new(32, layout::PTT_TRANSCRIPT.y as i32, 416, layout::PTT_TRANSCRIPT.h as i32);
/// "Last reply · …" / "Answering · …" / the naming help.
pub const CONTEXT_LANE: Rect = Rect::new(32, 390, 416, 30);
pub const TRANSCRIPT_LINES: usize = 5;
/// The waveform advances ten times a second.
pub const WAVE_STEP_MS: u32 = 100;
const BAR_W: i32 = 5;
const BAR_PITCH: i32 = 9;
const FLAT_BAR: i32 = 4;
/// Bar heights of mockup 7, 1x, left to right.
const ENVELOPE: [u8; 23] = [
    14, 24, 36, 50, 60, 42, 28, 52, 62, 38, 24, 44, 56, 32, 18, 40, 54, 28, 16, 34, 46, 22, 14,
];

/// Which frame of the waveform `now_ms` shows; the grammar compares this
/// between ticks.
pub const fn wave_phase(now_ms: u32) -> u32 {
    now_ms / WAVE_STEP_MS
}

/// Height of bar `bar` (0..23) for `phase` and the latched agent.
pub fn bar_height(bar: u8, phase: u32, agent: u8) -> i32 {
    let full = ENVELOPE.get(bar as usize).copied().unwrap_or(0) as i32;
    if phase == 0 {
        return full;
    }
    let noise = Hasher::new().u8(bar).u32(phase).u8(agent).finish();
    (full * (9 + (noise % 8) as i32) / 16).max(FLAT_BAR)
}

/// "Claude · web-ui", "New Macropad" (naming), "Key label" (the key
/// editor), "Channel 3" (agent unknown).
fn title(ui: &UiState, session: &PttSession, agent: Option<&Agent>) -> Text<40> {
    match (session.ctx, agent) {
        (PttCtx::Name, _) if ui.global.overlay == Overlay::MacroEdit => {
            let mut out = Text::new();
            out.push_str("Key label");
            out
        }
        (PttCtx::Name, _) => {
            let mut out = Text::new();
            out.push_str("New ");
            out.push_str(ui.manager.naming_template.label());
            out
        }
        (_, Some(agent)) => kind_name(agent),
        (_, None) => {
            let mut out = Text::new();
            out.push_str("Channel ");
            out.push_char((b'0' + session.agent % 10) as char);
            out
        }
    }
}

fn channel_tip(ui: &UiState, session: &PttSession, agent: Option<&Agent>) -> bool {
    !offline(ui) && session.ctx == PttCtx::Talk && ui.deck.transcript_channel_tip
        && agent.is_some_and(|a| a.kind == crate::deck::msg::AgentKind::Claude)
}

/// The context line: (prefix, text).
fn context<'a>(ui: &UiState, session: &PttSession, agent: Option<&'a Agent>) -> (&'static str, &'a str) {
    if channel_tip(ui, session, agent) { return (CHANNEL_TIP, ""); }
    match (session.ctx, agent) {
        (PttCtx::Name, _) if ui.global.overlay == Overlay::MacroEdit => ("Say a label for the key", ""),
        (PttCtx::Name, _) => ("Say a name for the new face", ""),
        (PttCtx::Answer, Some(agent)) if agent.has_card => ("Answering \u{b7} ", agent.card.text.as_str()),
        (_, Some(agent)) if !agent.last_reply.is_empty() => ("Last reply \u{b7} ", agent.last_reply.as_str()),
        _ => ("Last reply \u{b7} none yet", ""),
    }
}

pub fn draw(band: &mut Band, ui: &UiState) {
    let Some(session) = ui.global.ptt.or(ui.global.ptt_pending) else {
        return;
    };
    band.fill(kit::rgb(theme::CANVAS));
    let live = !offline(ui);
    let pending = ui.global.ptt.is_none();
    let agent = ui.deck.agent(session.agent);
    if kit::visible(band, TITLE_LANE) {
        let name = title(ui, &session, agent);
        text(band, Font::Name, theme::TEXT_X, layout::PTT_NAME_BASELINE, name.as_str(), theme::TEXT_PRIMARY, theme::CANVAS, layout::PTT_NAME.width(), Align::Left);
        if live {
            kit::pill(band, layout::PTT_PILL, if pending { "TRANSCRIBING" } else { "LISTENING" }, theme::GREEN, theme::LISTENING_FILL, theme::LISTENING_OUTLINE);
        } else {
            kit::pill(band, layout::PTT_PILL, "NO HOST", theme::AMBER, theme::CHIP_FILL, theme::OUTLINE_KEY);
        }
    }
    if pending {
        draw_progress(band, ui.now_ms);
    } else {
        draw_wave(band, ui.now_ms, session.agent, live);
    }
    draw_transcript(band, ui, live);
    if kit::visible(band, CONTEXT_LANE) {
        let (prefix, rest) = context(ui, &session, agent);
        let face = if channel_tip(ui, &session, agent) { Font::Hint } else { Font::Body };
        let x = layout::PTT_LAST_REPLY.left();
        let used = text(band, face, x, layout::PTT_LAST_REPLY_BASELINE, prefix, theme::TEXT_FAINT, theme::CANVAS, layout::PTT_LAST_REPLY.width(), Align::Left);
        text(band, face, x + used, layout::PTT_LAST_REPLY_BASELINE, rest, theme::TEXT_FAINT, theme::CANVAS, layout::PTT_LAST_REPLY.width() - used, Align::Left);
    }
    kit::hint_bar(band, hints(live, pending));
}

fn draw_progress(band: &mut Band, now_ms: u32) {
    if !kit::visible(band, layout::PTT_WAVE) { return; }
    let (cx, cy) = layout::PTT_WAVE.center();
    for dot in 0..3 {
        let color = if (now_ms / 300) % 3 == dot as u32 { theme::GREEN } else { theme::OUTLINE_KEY };
        kit::round(band, Rect::new(cx - 25 + dot * 20, cy - 5, 10, 10), 5, color);
    }
}

fn draw_wave(band: &mut Band, now_ms: u32, agent: u8, live: bool) {
    let lane = layout::PTT_WAVE;
    if !kit::visible(band, lane) {
        return;
    }
    let (_, cy) = lane.center();
    let phase = wave_phase(now_ms);
    for bar in 0..layout::PTT_WAVE_BARS {
        let (height, color) = if live { (bar_height(bar, phase, agent), theme::GREEN) } else { (FLAT_BAR, theme::GRAY) };
        let x = lane.left() + bar as i32 * BAR_PITCH;
        kit::round(band, Rect::new(x, cy - height / 2, BAR_W, height), 2, color);
    }
}

/// The transcript in straight quotes.
fn quoted(ui: &UiState) -> Text<204> {
    let mut out = Text::new();
    if !ui.deck.transcript.is_empty() {
        out.push_char('"');
        out.push_str(ui.deck.transcript.as_str());
        out.push_char('"');
    }
    out
}

/// Lines are wrapped this much narrower than the box so a leading "…" fits
/// in front of the first shown line when the earlier ones scroll away.
fn transcript_width() -> i32 {
    layout::PTT_TRANSCRIPT.width() - font::measure(Font::Text, "\u{2026}")
}

/// (lines the transcript wraps to, lines scrolled off the top): the live
/// tail is what matters, so the LAST lines that fit are shown.
pub fn transcript_lines(quoted: &str) -> (usize, usize) {
    let mut total = 0;
    font::wrap(Font::Text, quoted, transcript_width(), TRANSCRIPT_LINES + 26, &mut |index, _, _| total = index + 1);
    (total, total.saturating_sub(TRANSCRIPT_LINES))
}

/// The transcript's tail, a caret after the last character while the host
/// is still listening.
fn draw_transcript(band: &mut Band, ui: &UiState, live: bool) {
    if !kit::visible(band, TRANSCRIPT_LANE) {
        return;
    }
    let lane = layout::PTT_TRANSCRIPT;
    let lh = font::line_height(Font::Text);
    let ascent = font::ascent(Font::Text);
    let first = lane.top() + ascent;
    if !live {
        text(band, Font::Text, lane.left(), first, "No host is listening.", theme::TEXT_FAINT, theme::CANVAS, lane.width(), Align::Left);
        return;
    }
    let quoted = quoted(ui);
    let (total, skip) = transcript_lines(quoted.as_str());
    let width = transcript_width();
    let mut end = (lane.left(), first);
    font::wrap(Font::Text, quoted.as_str(), width, total.max(1), &mut |index, line, _| {
        if index < skip {
            return;
        }
        let baseline = first + (index - skip) as i32 * lh;
        let mut x = lane.left();
        if index == skip && skip > 0 {
            x += text(band, Font::Text, x, baseline, "\u{2026}", theme::TEXT_FAINT, theme::CANVAS, lane.width() - width, Align::Left);
        }
        x += text(band, Font::Text, x, baseline, line, theme::TEXT_PRIMARY, theme::CANVAS, width, Align::Left);
        end = (x, baseline);
    });
    if !ui.deck.transcript_final {
        kit::fill(band, Rect::new(end.0 + 3, end.1 - ascent, 2, ascent + 4), theme::GREEN);
    }
}

pub fn signature(ui: &UiState, sig: &mut SigBuilder) {
    let Some(session) = ui.global.ptt.or(ui.global.ptt_pending) else {
        return;
    };
    let live = !offline(ui);
    let pending = ui.global.ptt.is_none();
    let agent = ui.deck.agent(session.agent);
    sig.region_with(TITLE_LANE, |h| {
        h.text(&title(ui, &session, agent)).bool(live).bool(pending);
    });
    sig.region_with(layout::PTT_WAVE, |h| {
        h.bool(live).bool(pending).u8(session.agent).u32(if pending { ui.now_ms / 300 } else if live { wave_phase(ui.now_ms) } else { 0 });
    });
    sig.region_with(TRANSCRIPT_LANE, |h| {
        h.bool(live).text(&ui.deck.transcript).bool(ui.deck.transcript_final);
    });
    sig.region_with(CONTEXT_LANE, |h| {
        let (prefix, rest) = context(ui, &session, agent);
        h.str(prefix).str(rest);
    });
    sig.region_with(HINT_LANE, |h| {
        h.bool(live).bool(pending);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::msg::Template;

    fn session_ui(agent: u8, ctx: PttCtx) -> std::boxed::Box<UiState> {
        let mut ui = std::boxed::Box::new(UiState::new());
        ui.reset();
        ui.load_demo();
        ui.deck.host_present = true;
        ui.global.ptt = Some(PttSession::new(agent, ctx, 0, 0));
        ui
    }

    #[test]
    fn channel_setup_tip_is_small_and_only_for_online_claude_talk_sessions() {
        let mut ui = session_ui(1, PttCtx::Talk);
        ui.deck.agent_mut(1).unwrap().kind = crate::deck::msg::AgentKind::Claude;
        let session = ui.global.ptt.unwrap();
        assert_ne!(context(&ui, &session, ui.deck.agent(1)).0, CHANNEL_TIP);
        ui.deck.transcript_channel_tip = true;
        assert_eq!(context(&ui, &session, ui.deck.agent(1)), (CHANNEL_TIP, ""));
        assert!(font::measure(Font::Hint, CHANNEL_TIP) <= layout::PTT_LAST_REPLY.width());
        // Releasing PTT retains the same hint while the final transcript is pending.
        ui.global.ptt_pending = ui.global.ptt.take();
        assert!(channel_tip(&ui, &ui.global.ptt_pending.unwrap(), ui.deck.agent(1)));
        for ctx in [PttCtx::Answer, PttCtx::Name] {
            assert!(!channel_tip(&ui, &PttSession::new(1, ctx, 0, 0), ui.deck.agent(1)));
        }
        ui.deck.agent_mut(1).unwrap().kind = crate::deck::msg::AgentKind::Codex;
        assert!(!channel_tip(&ui, &session, ui.deck.agent(1)));
        ui.deck.agent_mut(1).unwrap().kind = crate::deck::msg::AgentKind::Claude;
        ui.deck.settings.demo = false;
        ui.deck.host_present = false;
        assert!(!channel_tip(&ui, &session, ui.deck.agent(1)));
    }

    #[test]
    fn bars_stay_under_the_envelope_and_move_with_the_phase() {
        for bar in 0..23 {
            assert_eq!(bar_height(bar, 0, 2), ENVELOPE[bar as usize] as i32);
            for phase in 1..40 {
                let h = bar_height(bar, phase, 2);
                assert!(h >= FLAT_BAR && h <= ENVELOPE[bar as usize] as i32, "{bar} {phase} {h}");
            }
        }
        assert!((1..40).any(|phase| bar_height(8, phase, 2) != bar_height(8, phase + 1, 2)));
        assert!((1..40).any(|phase| bar_height(8, phase, 2) != bar_height(8, phase, 3)));
        assert_eq!(bar_height(40, 0, 1), 0, "a bar past the envelope draws nothing");
        assert_eq!((wave_phase(0), wave_phase(99), wave_phase(100), wave_phase(u32::MAX)), (0, 0, 1, u32::MAX / WAVE_STEP_MS));
        assert_eq!(layout::PTT_WAVE.width(), 22 * BAR_PITCH + BAR_W);
        assert!(ENVELOPE.iter().all(|&h| h as i32 <= layout::PTT_WAVE.height()));
    }

    #[test]
    fn the_title_and_context_follow_the_session() {
        let mut ui = session_ui(2, PttCtx::Talk);
        let session = ui.global.ptt.unwrap();
        assert_eq!(title(&ui, &session, ui.deck.agent(2)).as_str(), "Claude \u{b7} web-ui");
        assert_eq!(context(&ui, &session, ui.deck.agent(2)), ("Last reply \u{b7} ", "Header refactor done, 34 files changed"));
        let answer = PttSession::new(1, PttCtx::Answer, 1, 0);
        assert_eq!(context(&ui, &answer, ui.deck.agent(1)).0, "Answering \u{b7} ");
        let gone = PttSession::new(7, PttCtx::Talk, 0, 0);
        assert_eq!(title(&ui, &gone, None).as_str(), "Channel 7");
        assert_eq!(context(&ui, &gone, None), ("Last reply \u{b7} none yet", ""));
        ui.manager.naming_template = Template::Macropad;
        let naming = PttSession::new(0, PttCtx::Name, 0, 0);
        assert_eq!(title(&ui, &naming, None).as_str(), "New Macropad");
        assert_eq!(context(&ui, &naming, None).0, "Say a name for the new face");
        // The same press from the key editor dictates a label, not a name.
        ui.global.overlay = Overlay::MacroEdit;
        assert_eq!(title(&ui, &naming, None).as_str(), "Key label");
        assert_eq!(context(&ui, &naming, None).0, "Say a label for the key");
        ui.global.overlay = Overlay::None;
        let mut idle = *ui.deck.agent(4).unwrap();
        idle.last_reply.clear();
        assert_eq!(context(&ui, &session, Some(&idle)).0, "Last reply \u{b7} none yet");
    }

    #[test]
    fn the_transcript_keeps_its_tail() {
        let mut ui = session_ui(2, PttCtx::Talk);
        assert_eq!(quoted(&ui).as_str(), "", "no quotes around nothing");
        ui.deck.transcript.set(crate::deck::demo::TRANSCRIPT);
        assert_eq!(quoted(&ui).as_str(), "\"Refactor the auth module, then run the test suite.\"");
        assert_eq!(transcript_lines(quoted(&ui).as_str()), (2, 0));
        assert_eq!(transcript_lines(""), (0, 0));
        let mut long = std::string::String::new();
        while transcript_lines(&long).0 < 9 {
            long.push_str("staging migrations ");
        }
        let (total, skip) = transcript_lines(&long);
        assert_eq!((total, skip), (9, 4));
        assert!(transcript_width() + font::measure(Font::Text, "\u{2026}") == layout::PTT_TRANSCRIPT.width());
    }

    #[test]
    fn the_hint_and_lanes_fit_the_layout() {
        assert_eq!(kit::hints_that_fit(&HINTS).0, HINTS.len());
        assert_eq!(kit::hints_that_fit(&NO_HOST_HINTS).0, NO_HOST_HINTS.len());
        assert_eq!((hints(true, false)[1].label, hints(false, false)[1].label), ("to send", "to end"));
        assert!(!NO_HOST_HINTS[1].label.contains("send"), "nothing is sent without a host");
        assert!(!TITLE_LANE.intersects(&layout::PTT_WAVE));
        assert!(!layout::PTT_WAVE.intersects(&TRANSCRIPT_LANE));
        assert!(!TRANSCRIPT_LANE.intersects(&CONTEXT_LANE));
        assert!(!CONTEXT_LANE.intersects(&HINT_LANE));
        assert!(TRANSCRIPT_LANE.top() + TRANSCRIPT_LINES as i32 * font::line_height(Font::Text) <= TRANSCRIPT_LANE.bottom());
        assert!(layout::in_safe_area(&TITLE_LANE) && layout::in_safe_area(&CONTEXT_LANE));
    }
}
