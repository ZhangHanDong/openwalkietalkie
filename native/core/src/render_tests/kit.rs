//! Widget kit and font rendering (`ui::kit`, `ui::font`): the same pixels
//! through any band or clip partition and at any origin, the hint bar fit
//! rule, ellipsis, wrap, the replacement box and both background paths.
use super::*;
use crate::ui::{
    font::{self, Align, Font},
    kit::{self, HeaderRight, Hint},
    theme, Rect,
};
use std::time::Instant;

const LONG: &str = "Refactor the auth module, then run the whole test suite and report";

/// Every widget and text path on one canvas.
fn kit_scene(band: &mut display::Band) {
    band.fill(kit::rgb(theme::CANVAS));
    kit::header(band, "AGENTS", HeaderRight::Clock(19 * 60 + 42));
    kit::card(band, layout::roster_row(0), true, false);
    kit::card(band, layout::roster_row(1), false, false);
    kit::card(band, layout::roster_row(2), false, true);
    kit::dot(band, 53, 107, 5, theme::RED);
    kit::dot(band, 53, 185, 5, theme::GREEN);
    kit::bar(band, Rect::new(37, 300, 406, 10), 62, theme::GREEN);
    kit::bar(band, Rect::new(37, 320, 406, 10), 1, theme::AMBER);
    kit::chip(band, layout::omarchy_chip(0), "1", false);
    kit::chip(band, layout::omarchy_chip(1), "2", true);
    kit::pill(band, layout::PTT_PILL, "LISTENING", theme::GREEN, theme::LISTENING_FILL, theme::LISTENING_OUTLINE);
    kit::fill(band, Rect::new(200, 340, 240, 40), theme::CARD_FILL);
    font::draw(band, Font::Name, 72, 103, "Codex \u{b7} cohdl-api", theme::TEXT_PRIMARY, Some(theme::CARD_FILL), 360, Align::Left);
    font::draw(band, Font::Body, 72, 126, LONG, theme::TEXT_SECONDARY, Some(theme::CARD_FILL), 300, Align::Left);
    font::draw(band, Font::Title, 240, 260, "Tests fail on main", theme::TEXT_PRIMARY, None, 400, Align::Center);
    font::draw(band, Font::Small, 442, 360, "blocked 2m \u{2192} \u{2713} \u{4e2d}", theme::AMBER, None, 220, Align::Right);
    font::draw(band, Font::Display, 38, 410, "Workspace 2", theme::TEXT_PRIMARY, Some(theme::CANVAS), 300, Align::Left);
    font::draw(band, Font::Label, 240, 380, "\u{c9}\u{d7}\u{153}", theme::TEXT_LABEL, Some(theme::CANVAS), 100, Align::Center);
    kit::hint_bar(band, &[
        Hint { key: "SCROLL", label: "choose" },
        Hint { key: "ENTER", label: "confirm" },
        Hint { key: "PTT", label: "reply" },
        Hint { key: "CANCEL", label: "snooze" },
    ]);
}

/// Draw through sub-band clips of `h` rows by `w` columns, gathering only what
/// each clip admitted.
fn partitioned(h: i32, w: i32, draw: &mut dyn FnMut(&mut display::Band)) -> Vec<u8> {
    let mut pixels = vec![0; 480 * 480 * 2];
    let mut band = display::Band::new();
    for index in 0..display::BANDS {
        let y0 = index as i32 * display::BAND_H as i32;
        let mut y = y0;
        while y < y0 + display::BAND_H as i32 {
            let rows = h.min(y0 + display::BAND_H as i32 - y);
            let mut x = 0;
            while x < 480 {
                let cols = w.min(480 - x);
                band.set_row(index);
                band.fill(Rgb565::MAGENTA);
                band.clip_to(Rectangle::new(Point::new(x, y), Size::new(cols as u32, rows as u32)));
                draw(&mut band);
                for row in y..y + rows {
                    let src = ((row - y0) as usize * 480 + x as usize) * 2;
                    let dst = (row as usize * 480 + x as usize) * 2;
                    pixels[dst..dst + cols as usize * 2].copy_from_slice(&band.data()[src..src + cols as usize * 2]);
                }
                x += cols;
            }
            y += rows;
        }
    }
    pixels
}

fn at_origin(dx: i32, draw: &mut dyn FnMut(&mut display::Band)) -> Vec<u8> {
    full_draw(|band| {
        band.set_origin_x(dx);
        draw(band);
    })
}

/// A complete frame of `draw` over a cleared canvas (the harness band keeps
/// the previous band's pixels where nothing is drawn).
fn clean_draw(mut draw: impl FnMut(&mut display::Band)) -> Vec<u8> {
    full_draw(|band| {
        band.fill(kit::rgb(theme::CANVAS));
        draw(band);
    })
}

fn pixel(frame: &[u8], x: i32, y: i32) -> u16 {
    let at = (y as usize * 480 + x as usize) * 2;
    u16::from_le_bytes([frame[at], frame[at + 1]])
}

fn lit_in(frame: &[u8], rect: Rect) -> impl Iterator<Item = (i32, i32)> + '_ {
    (rect.top()..rect.bottom())
        .flat_map(move |y| (rect.left()..rect.right()).map(move |x| (x, y)))
        .filter(move |&(x, y)| pixel(frame, x, y) != 0)
}

fn rgb888(raw: u16) -> [i32; 3] {
    [((raw >> 11) & 31) as i32 * 255 / 31, ((raw >> 5) & 63) as i32 * 255 / 63, (raw & 31) as i32 * 255 / 31]
}

#[test]
fn text_and_widgets_are_the_same_through_any_band_or_clip_partition() {
    let reference = full_draw(kit_scene);
    assert!(lit_in(&reference, Rect::SCREEN).count() > 5000);
    for (h, w) in [(48, 480), (7, 480), (1, 480), (48, 13), (5, 3)] {
        let frame = partitioned(h, w, &mut kit_scene);
        assert_frame(&frame, &reference, &format!("{h}x{w} clips must give the banded frame"));
    }
}

#[test]
fn text_and_widgets_survive_every_origin_and_shift_exactly() {
    let reference = full_draw(kit_scene);
    for dx in -480..=480 {
        let frame = at_origin(dx, &mut kit_scene);
        if ![-480, -479, -241, -1, 0, 1, 33, 240, 479, 480].contains(&dx) {
            continue;
        }
        for y in 0..480 {
            for x in 0..480 {
                let expected = if (0..480).contains(&(x - dx)) { pixel(&reference, x - dx, y) } else { 0 };
                assert_eq!(pixel(&frame, x, y), expected, "origin {dx} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn nothing_panics_for_any_text_geometry_or_utf8() {
    let mut band = display::Band::new();
    let mut seed = 0x9E37_79B9u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    let fonts = [Font::Display, Font::Title, Font::Name, Font::Macro, Font::Text, Font::Row, Font::Body, Font::Small, Font::Label, Font::Hint, Font::Keycap];
    for step in 0..400 {
        let bytes: Vec<u8> = (0..(next() % 64) as usize).map(|_| next() as u8).collect();
        let text = String::from_utf8_lossy(&bytes);
        let font = fonts[(next() % 11) as usize];
        let x = (next() % 1200) as i32 - 600;
        let y = (next() % 1200) as i32 - 600;
        let max_w = (next() % 1200) as i32 - 200;
        let align = [Align::Left, Align::Center, Align::Right][(next() % 3) as usize];
        let bg = if step % 2 == 0 { Some(theme::CARD_FILL) } else { None };
        band.set_row((next() % display::BANDS as u32) as u16);
        band.set_origin_x((next() % 1000) as i32 - 500);
        let width = font::draw(&mut band, font, x, y, &text, theme::TEXT_PRIMARY, bg, max_w, align);
        assert!(width >= 0 && width <= max_w.max(0));
        assert!(font::measure(font, &text) >= 0);
        font::wrap(font, &text, max_w, (next() % 5) as usize, &mut |_, line, _| {
            assert!(!line.starts_with(' '));
        });
        let rect = Rect::new(x, y, (next() % 500) as i32 - 20, (next() % 500) as i32 - 20);
        kit::round(&mut band, rect, next() % 40, theme::RED);
        kit::outline(&mut band, rect, next() % 40, theme::GREEN, next() % 6);
        kit::dashed(&mut band, rect, next() % 40, theme::BLUE, theme::CANVAS);
        kit::card(&mut band, rect, step % 3 == 0, step % 3 == 1);
        kit::bar(&mut band, rect, next() as u8, theme::AMBER);
        kit::chip(&mut band, rect, &text, step % 2 == 0);
        kit::pill(&mut band, rect, &text, theme::GREEN, theme::LISTENING_FILL, theme::LISTENING_OUTLINE);
        kit::dot(&mut band, x, y, (next() % 40) as i32 - 5, theme::GRAY);
        kit::keycap(&mut band, x, &text);
        kit::hint_bar(&mut band, &[Hint { key: &text, label: &text }, Hint { key: "", label: &text }]);
        kit::header(&mut band, &text, HeaderRight::Text(&text));
        kit::header_tag(&mut band, HeaderRight::Battery(next() as u8));
        kit::header_tag(&mut band, HeaderRight::Clock(next() as u16));
    }
    // Every scalar value, in every font, at the panel edge and outside it.
    let all: String = ('\0'..=char::MAX).step_by(997).collect();
    for font in fonts {
        band.set_row(3);
        band.set_origin_x(0);
        assert!(font::draw(&mut band, font, -30, 170, &all, theme::TEXT_PRIMARY, None, i32::MAX, Align::Left) > 0);
        font::draw(&mut band, font, 470, 170, &all, theme::TEXT_PRIMARY, Some(0), i32::MIN, Align::Right);
        font::draw(&mut band, font, i32::MIN, i32::MAX, &all, 0, None, i32::MAX, Align::Center);
        font::draw(&mut band, font, i32::MAX, i32::MIN, &all, 0, Some(0), 5, Align::Center);
        font::wrap(font, &all, i32::MAX, usize::MAX, &mut |_, _, _| {});
        font::wrap(font, &all, i32::MIN, 3, &mut |_, _, _| {});
    }
}

#[test]
fn the_hint_bar_keeps_its_content_between_32_and_448() {
    let hints = [
        Hint { key: "SCROLL", label: "choose" },
        Hint { key: "ENTER", label: "confirm" },
        Hint { key: "PTT", label: "reply" },
        Hint { key: "CANCEL", label: "snooze" },
        Hint { key: "SHIFT SHIFT", label: "manage faces" },
        Hint { key: "LEFT RIGHT", label: "switch the channel" },
    ];
    let (count, total) = kit::hints_that_fit(&hints);
    assert!(count == 4 && total <= layout::HINT_CONTENT.width(), "{count} {total}");
    let frame = clean_draw(|band| kit::hint_bar(band, &hints));
    let bar = theme::HINT_FILL;
    let outside = lit_in(&frame, layout::HINT_BAR)
        .filter(|&(x, y)| pixel(&frame, x, y) != kit::rgb(bar).into_storage())
        .find(|&(x, _)| !(32..448).contains(&x));
    assert_eq!(outside, None, "hint content outside x in [32, 448)");
    assert!(lit_in(&frame, layout::HINT_CONTENT).count() > 400);
    for x in 0..480 {
        assert_eq!(pixel(&frame, x, theme::HINT_Y - 1), kit::rgb(theme::HAIRLINE).into_storage());
        assert_eq!(pixel(&frame, x, theme::HINT_Y - 2), 0);
    }
    // A single label wider than the bar is cut with an ellipsis, not clipped.
    let wide = [Hint { key: "", label: "tap or press \u{b7} dashed keys are touch-only \u{b7} hold to assign \u{b7} double SHIFT manages faces \u{b7} and more" }];
    let frame = clean_draw(|band| kit::hint_bar(band, &wide));
    assert_eq!(kit::hints_that_fit(&wide).0, 0);
    let lit: Vec<_> = lit_in(&frame, layout::HINT_CONTENT)
        .filter(|&(x, y)| pixel(&frame, x, y) != kit::rgb(bar).into_storage())
        .collect();
    assert!(lit.iter().all(|&(x, _)| (32..448).contains(&x)));
    assert!(!lit.is_empty(), "a dropped-but-only hint still draws what fits");
    assert_eq!(lit_outside_safe_area(&frame), None);
}

/// The glyphs that end a run at `x + width`, compared with "…" drawn alone.
fn ends_in_ellipsis(frame: &[u8], font: Font, x: i32, width: i32, baseline: i32) -> bool {
    let dots = font::measure(font, "\u{2026}");
    let alone = clean_draw(|band| {
        font::draw(band, font, x + width - dots, baseline, "\u{2026}", theme::TEXT_PRIMARY, Some(theme::CANVAS), 100, Align::Left);
    });
    let lane = Rect::new(x + width - dots, baseline - font::ascent(font), dots, font::line_height(font));
    lit_in(&alone, lane).count() > 0
        && lit_in(&alone, lane).all(|(px, py)| pixel(frame, px, py) == pixel(&alone, px, py))
}

#[test]
fn an_over_wide_run_ends_in_an_ellipsis_within_max_w() {
    for (font, max_w) in [(Font::Row, 200), (Font::Body, 121), (Font::Display, 300), (Font::Label, 90)] {
        let mut width = 0;
        let frame = clean_draw(|band| {
            width = font::draw(band, font, 40, 200, LONG, theme::TEXT_PRIMARY, Some(theme::CANVAS), max_w, Align::Left);
        });
        assert!(width > 0 && width <= max_w, "{font:?}: {width} of {max_w}");
        assert!(width > max_w - font::measure(font, "MM"), "{font:?} wastes room: {width} of {max_w}");
        let beyond = Rect::new(40 + max_w + 2, 100, 400 - max_w, 200);
        assert_eq!(lit_in(&frame, beyond).next(), None, "{font:?} lit past max_w");
        assert!(ends_in_ellipsis(&frame, font, 40, width, 200), "{font:?} must end in …");
    }
    // A run that fits is drawn whole, right-aligned or centred alike.
    let short = "Idle";
    let w = font::measure(Font::Body, short);
    for align in [Align::Left, Align::Center, Align::Right] {
        let frame = clean_draw(|band| {
            assert_eq!(font::draw(band, Font::Body, 240, 100, short, theme::TEXT_PRIMARY, None, w, align), w);
        });
        assert!(!ends_in_ellipsis(&frame, Font::Body, 240 - w, w, 100));
        let (left, right) = match align {
            Align::Left => (240, 240 + w),
            Align::Center => (240 - w / 2, 240 - w / 2 + w),
            Align::Right => (240 - w, 240),
        };
        let lit: Vec<_> = lit_in(&frame, Rect::new(0, 80, 480, 30)).collect();
        assert!(lit.iter().all(|&(x, _)| x >= left - 1 && x < right + 1), "{align:?}");
    }
}

#[test]
fn wrapped_lines_fit_and_a_truncated_last_line_ends_in_an_ellipsis() {
    let max_w = 220;
    let mut lines = Vec::new();
    font::wrap(Font::Text, LONG, max_w, 3, &mut |index, line, cut| lines.push((index, line.to_string(), cut)));
    assert_eq!(lines.len(), 3);
    assert!(lines[..2].iter().all(|(_, line, cut)| !cut && font::measure(Font::Text, line) <= max_w));
    assert!(lines[2].2 && font::measure(Font::Text, &lines[2].1) > max_w);
    let lh = font::line_height(Font::Text);
    let mut widths = Vec::new();
    let frame = clean_draw(|band| {
        widths.clear();
        for (index, line, _) in &lines {
            let y = 100 + *index as i32 * lh;
            widths.push(font::draw(band, Font::Text, 40, y, line, theme::TEXT_PRIMARY, Some(theme::CANVAS), max_w, Align::Left));
        }
    });
    assert!(widths.iter().all(|&w| w > 0 && w <= max_w));
    assert!(ends_in_ellipsis(&frame, Font::Text, 40, widths[2], 100 + 2 * lh));
    assert!(!ends_in_ellipsis(&frame, Font::Text, 40, widths[0], 100));
    assert_eq!(lit_in(&frame, Rect::new(40 + max_w + 1, 60, 200, 120)).next(), None);
}

#[test]
fn unknown_codepoints_draw_the_replacement_box() {
    let draw = |text: &'static str| {
        clean_draw(move |band| {
            font::draw(band, Font::Row, 40, 100, text, theme::TEXT_PRIMARY, Some(theme::CANVAS), 400, Align::Left);
        })
    };
    let boxes = draw("\u{fffd}\u{fffd}\u{fffd}");
    assert!(lit_in(&boxes, Rect::SCREEN).count() > 30);
    assert_frame(&draw("\u{20000}\u{1f600}\u{3a9}"), &boxes, "unsupported extension B, emoji and Greek are the box");
    assert_frame(&draw("\n\t\u{7f}"), &boxes, "controls are the box");
    assert_eq!(font::measure(Font::Row, "\u{20000}\u{1f600}\u{3a9}"), font::measure(Font::Row, "\u{fffd}\u{fffd}\u{fffd}"));
    // Covered glyphs are not the box.
    let covered = draw("\u{2192}\u{2713}\u{153}");
    assert!(lit_in(&covered, Rect::SCREEN).zip(lit_in(&boxes, Rect::SCREEN)).any(|(a, b)| a != b));
}

#[test]
fn chinese_glyphs_are_distinct_and_scale_through_every_band_and_clip() {
    let scene = |band: &mut display::Band| {
        band.fill(kit::rgb(theme::CANVAS));
        for (i, spec) in theme::RAMP.iter().enumerate() {
            let y = 37 + i as i32 * 29;
            font::draw(band, spec.font, 30, y, "中文簡體，。", theme::TEXT_PRIMARY, Some(theme::CANVAS), 200, Align::Left);
            font::draw(band, spec.font, 450, y, "繁體測試！？", theme::AMBER, None, 200, Align::Right);
        }
    };
    let reference = full_draw(scene);
    for (h, w) in [(1, 480), (7, 13)] {
        assert_frame(&partitioned(h, w, &mut |band| scene(band)), &reference, "Chinese clip seams");
    }
    for spec in theme::RAMP {
        let draw = |text: &str| clean_draw(|band| {
            font::draw(band, spec.font, 40, 100, text, theme::TEXT_PRIMARY, Some(theme::CANVAS), 400, Align::Left);
        });
        let chinese = draw("中");
        assert_ne!(chinese, draw("文"));
        assert_ne!(chinese, draw("\u{fffd}"));
        assert!(lit_in(&chinese, Rect::new(40, 40, 60, 80)).count() > 15);
    }
}

#[test]
fn the_ramp_path_matches_read_blending_on_black_and_nearly_on_a_card() {
    let run = |bg: Option<u32>, under: u32| {
        clean_draw(move |band| {
            kit::fill(band, Rect::new(0, 60, 480, 120), under);
            font::draw(band, Font::Name, 40, 120, "Codex \u{b7} cohdl-api \u{e9}", theme::TEXT_PRIMARY, bg, 400, Align::Left);
            font::draw(band, Font::Small, 440, 160, "blocked 2m 中文繁體", theme::AMBER, bg, 400, Align::Right);
        })
    };
    assert_frame(&run(Some(theme::CANVAS), theme::CANVAS), &run(None, theme::CANVAS), "black: both paths identical");
    let ramp = run(Some(theme::CARD_FILL), theme::CARD_FILL);
    let blend = run(None, theme::CARD_FILL);
    let mut worst = 0;
    for (a, b) in ramp.chunks_exact(2).zip(blend.chunks_exact(2)) {
        let (a, b) = (rgb888(u16::from_le_bytes([a[0], a[1]])), rgb888(u16::from_le_bytes([b[0], b[1]])));
        worst = worst.max((0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap_or(0));
    }
    // The blend path sees the card fill after RGB565 rounding, the ramp the true colour.
    assert!(worst <= 12, "paths differ by {worst}/255");
    assert!(lit_in(&ramp, Rect::new(0, 0, 480, 60)).next().is_none());
}

#[test]
fn rounded_shapes_stay_inside_their_rects_and_are_symmetric() {
    let rect = Rect::new(100, 100, 200, 90);
    let filled = clean_draw(|band| kit::round(band, rect, 14, theme::GREEN));
    let outlined = clean_draw(|band| kit::outline(band, rect, 14, theme::GREEN, 2));
    let dashed = clean_draw(|band| kit::dashed(band, rect, 14, theme::GREEN, theme::CANVAS));
    let dot = clean_draw(|band| kit::dot(band, 200, 145, 5, theme::RED));
    let dot_rect = Rect::new(195, 140, 11, 11);
    for (name, frame, rect) in [("round", &filled, rect), ("outline", &outlined, rect), ("dashed", &dashed, rect), ("dot", &dot, dot_rect)] {
        assert!(lit_in(frame, Rect::SCREEN).all(|(x, y)| rect.contains(x, y)), "{name} leaks");
        if name == "dashed" {
            continue; // the dash phase runs from one corner
        }
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let mirror_x = rect.right() - 1 - (x - rect.left());
                let mirror_y = rect.bottom() - 1 - (y - rect.top());
                assert_eq!(pixel(frame, x, y), pixel(frame, mirror_x, y), "{name} x-symmetry");
                assert_eq!(pixel(frame, x, y), pixel(frame, x, mirror_y), "{name} y-symmetry");
            }
        }
    }
    let green = kit::rgb(theme::GREEN).into_storage();
    // Corners are anti-aliased: a partial pixel exists and the very corner is dark.
    assert_eq!(pixel(&filled, rect.left(), rect.top()), 0);
    assert_eq!(pixel(&filled, rect.left() + 14, rect.top()), green);
    assert_eq!(pixel(&filled, rect.left(), rect.top() + 14), green);
    assert!(lit_in(&filled, Rect::new(100, 100, 14, 14)).any(|(x, y)| pixel(&filled, x, y) != green));
    // The outline is hollow and two pixels wide on the straight edges.
    let (cx, cy) = rect.center();
    assert_eq!(pixel(&outlined, cx, cy), 0);
    assert_eq!(pixel(&outlined, rect.left(), cy), green);
    assert_eq!(pixel(&outlined, rect.left() + 1, cy), green);
    assert_eq!(pixel(&outlined, rect.left() + 2, cy), 0);
    assert_eq!(pixel(&outlined, cx, rect.top() + 1), green);
    assert_eq!(pixel(&outlined, cx, rect.top() + 2), 0);
    // Dashes: gaps in the straight runs, solid arcs.
    let top: Vec<bool> = (rect.left()..rect.right()).map(|x| pixel(&dashed, x, rect.top()) != 0).collect();
    assert!(top[14..186].iter().any(|&lit| !lit) && top[14..186].iter().any(|&lit| lit));
    assert!(lit_in(&dashed, Rect::SCREEN).count() < lit_in(&outlined, Rect::SCREEN).count());
    // The dot is 11 px across with a soft edge.
    assert_eq!(pixel(&dot, 200, 145), kit::rgb(theme::RED).into_storage());
    assert_eq!(pixel(&dot, 195, 145), kit::rgb(theme::RED).into_storage());
    assert_eq!(pixel(&dot, 194, 145), 0);
    assert_ne!(pixel(&dot, 196, 141), 0);
    assert_ne!(pixel(&dot, 196, 141), kit::rgb(theme::RED).into_storage());
}

#[test]
fn a_roster_frame_renders_in_bounded_host_time() {
    let ui = ui();
    let _ = full(&ui);
    let frames = 50;
    let per_frame = (0..5)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..frames {
                let _ = full(&ui);
            }
            start.elapsed() / frames
        })
        .min()
        .unwrap_or_default();
    eprintln!("roster frame (10 bands, host, opt-level 2, best of 5): {per_frame:?}");
    assert!(per_frame.as_millis() < 200, "{per_frame:?}");
}
