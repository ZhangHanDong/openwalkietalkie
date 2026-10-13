//! Manager, gallery, naming, confirm and notification overlays
//! (`ui::overlay::{manager, notify}`): the toast slides inside its lane and
//! every frame of it refreshes partially, the indicator follows the drawer,
//! drawer / manager / gallery / confirm rows sit where `layout` hit-tests
//! them, the manager family refreshes partially through a script, hint bars
//! fit, nothing lights the arcs, and no origin panics.
use super::*;
use crate::{
    deck::model::Notice,
    deck::msg::{CardKind, FaceEntry, HostMsg},
    deck::text::Text,
    ui::{
        kit,
        overlay::{self, notify::{slide, toast_rect, SLIDE_MS}},
        state::{Overlay, Toast, ToastKind},
        theme, Rect, Update,
    },
};

fn pixel(frame: &[u8], x: i32, y: i32) -> u16 {
    let at = (y as usize * 480 + x as usize) * 2;
    u16::from_le_bytes([frame[at], frame[at + 1]])
}

fn rgb(color: u32) -> u16 {
    kit::rgb(color).into_storage()
}

fn diff_box(a: &[u8], b: &[u8]) -> Rect {
    let mut bounds = Rect::EMPTY;
    for y in 0..480 {
        for x in 0..480 {
            if pixel(a, x, y) != pixel(b, x, y) {
                bounds = bounds.union(&Rect::new(x, y, 1, 1));
            }
        }
    }
    bounds
}

/// The first pixel that differs between two frames outside `allowed`.
fn diff_outside(a: &[u8], b: &[u8], allowed: &[Rect]) -> Option<(i32, i32)> {
    (0..480)
        .flat_map(|y| (0..480).map(move |x| (x, y)))
        .find(|&(x, y)| pixel(a, x, y) != pixel(b, x, y) && !allowed.iter().any(|r| r.contains(x, y)))
}

fn lit_in(frame: &[u8], rect: Rect) -> usize {
    (rect.top()..rect.bottom())
        .flat_map(|y| (rect.left()..rect.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(frame, x, y) != 0)
        .count()
}

/// Hint content stays between x 32 and 448.
fn assert_hint_bar_fits(frame: &[u8], what: &str) {
    let bar = rgb(theme::HINT_FILL);
    for y in layout::HINT_CONTENT.top()..layout::HINT_CONTENT.bottom() {
        for x in 0..480 {
            assert!(pixel(frame, x, y) == bar || (32..448).contains(&x), "{what}: hint content at x {x}");
        }
    }
    assert!(lit_in(frame, layout::HINT_CONTENT) > 200, "{what}: a hint bar");
}

fn hosted(template: Template) -> Box<UiState> {
    let mut ui = on(template);
    ui.deck.host_present = true;
    ui.invalidate();
    ui
}

fn with_overlay(overlay: Overlay) -> Box<UiState> {
    let mut ui = hosted(Template::Omarchy);
    ui.global.overlay = overlay;
    ui.invalidate();
    ui
}

fn first_lit_row(frame: &[u8], rect: Rect) -> Option<i32> {
    (rect.top()..rect.bottom()).find(|&y| (rect.left()..rect.right()).any(|x| pixel(frame, x, y) != 0))
}

#[test]
fn a_toast_slides_down_its_lane_and_every_frame_refreshes_partially() {
    // The Settings face beneath signs fine regions, so the lane alone repaints.
    let mut ui = hosted(Template::Settings);
    ui.deck.notices = [Notice::EMPTY; 8];
    let mut out = Update::new();
    ui.tick(1000, 0, true, &mut out);
    let bare = full(&ui);
    ui.notify.toast = Some(Toast::card(1, 1, CardKind::Question, 1000));
    ui.invalidate();
    let mut screens = Screens::new(&ui);
    let mut last_top = -1;
    let mut now = 1000;
    let lane = layout::TOAST_LANE;
    while now <= 1000 + SLIDE_MS + 40 {
        let before = ui.clone();
        now += 16;
        ui.tick(now, 0, true, &mut out);
        let transferred = update(&mut screens, &before, &ui);
        let frame = full(&ui);
        let toast = ui.notify.toast.unwrap();
        let rect = toast_rect(&toast, now);
        assert!(rect.top() <= layout::TOAST.top() && rect.bottom() >= 0);
        // The lit rows of the lane are the toast's rows (the outline is 1 px).
        let top = first_lit_row(&frame, lane).unwrap_or(lane.bottom());
        assert!(top >= last_top, "the toast never moves back up: {top} < {last_top}");
        let expected = rect.top().max(0);
        assert!(expected <= top && top <= expected + 3, "{now}: lit from {top}, toast at {rect:?}");
        last_top = top;
        // Only the lane (and the indicator inside it) repaints.
        assert_eq!(lane.union(&diff_box(&bare, &frame)), lane);
        assert!(transferred <= (lane.width() + 2) as usize * (lane.height() + 2) as usize, "{transferred}");
    }
    assert_eq!(toast_rect(&ui.notify.toast.unwrap(), now), layout::TOAST);
    assert_eq!(slide(&ui.notify.toast.unwrap(), now), layout::TOAST.bottom());
    assert!(ui.animating());
    // The toast expires on its own timer into the drawer: the lane repaints
    // to the face, with the indicator pill now showing for the filed notice.
    let before = ui.clone();
    ui.tick(1000 + 5000, 0, true, &mut out);
    assert!(ui.notify.toast.is_none());
    assert_eq!(ui.deck.notice_count(), 1);
    update(&mut screens, &before, &ui);
    let indicator = layout::INDICATOR;
    assert_eq!(indicator.union(&diff_box(&bare, &full(&ui))), indicator);
}

#[test]
fn toast_kinds_show_their_dot_colour_and_their_lines() {
    let mut ui = hosted(Template::Usage);
    ui.deck.notices = [Notice::EMPTY; 8];
    let (dx, dy, _) = layout::TOAST_DOT;
    let dot = (layout::TOAST.left() + dx, layout::TOAST.top() + dy);
    let mut frames = Vec::new();
    for (toast, color) in [
        (Toast::card(1, 1, CardKind::Question, 0), theme::RED),
        (Toast::card(2, 2, CardKind::Ready, 0), theme::GREEN),
        (Toast::message(ToastKind::NoChannel, 0, 0, 0), theme::AMBER),
        (Toast::message(ToastKind::NotDelivered, 1, 1, 0), theme::AMBER),
        (Toast::message(ToastKind::NotDelivered, 2, 2, 0), theme::AMBER),
    ] {
        ui.now_ms = 5000;
        ui.notify.toast = Some(toast);
        ui.invalidate();
        let frame = full(&ui);
        assert_eq!(pixel(&frame, dot.0, dot.1), rgb(color), "{:?}", toast.kind);
        assert_eq!(pixel(&frame, 240, layout::TOAST.top() + 3), rgb(theme::CARD_FILL));
        assert_eq!(lit_outside_safe_area(&frame), None);
        frames.push(frame);
    }
    for (i, a) in frames.iter().enumerate() {
        for b in &frames[i + 1..] {
            assert_ne!(diff_box(a, b), Rect::EMPTY, "every toast reads differently");
            assert_eq!(layout::TOAST.union(&diff_box(a, b)), layout::TOAST);
        }
    }
    // A finger on the toast brightens it, inside the toast only.
    ui.notify.toast_pressed = true;
    ui.invalidate();
    let pressed = full(&ui);
    assert_eq!(pixel(&pressed, 240, layout::TOAST.top() + 3), rgb(theme::CHIP_FILL));
    assert_eq!(layout::TOAST.union(&diff_box(&frames[4], &pressed)), layout::TOAST);
    // A toast for an agent the roster no longer has still draws its card kind.
    ui.notify.toast_pressed = false;
    ui.notify.toast = Some(Toast::card(7, 9, CardKind::Ready, 0));
    ui.invalidate();
    let orphan = full(&ui);
    assert_eq!(pixel(&orphan, dot.0, dot.1), rgb(theme::GREEN));
    assert!(lit_in(&orphan, Rect::new(88, 30, 200, 30)) > 20, "the kind word shows");
}

#[test]
fn the_indicator_follows_the_drawer_and_repaints_alone() {
    let mut ui = hosted(Template::Settings);
    ui.deck.notices = [Notice::EMPTY; 8];
    ui.invalidate();
    let (cx, cy) = layout::INDICATOR.center();
    let none = full(&ui);
    assert_eq!(pixel(&none, cx, cy), 0);
    ui.deck.notices[2] = Notice::new(2, 2, CardKind::Ready, 0).filed();
    ui.invalidate();
    let green = full(&ui);
    assert_eq!(pixel(&green, cx, cy), rgb(theme::GREEN));
    assert_eq!(layout::INDICATOR.union(&diff_box(&none, &green)), layout::INDICATOR);
    ui.deck.notices[5] = Notice::new(1, 1, CardKind::Question, 0).filed();
    ui.invalidate();
    let red = full(&ui);
    assert_eq!(pixel(&red, cx, cy), rgb(theme::RED));
    // A notice that is still toasting (not filed) does not count.
    ui.deck.notices[5].in_drawer = false;
    ui.invalidate();
    assert_eq!(pixel(&full(&ui), cx, cy), rgb(theme::GREEN));
    let mut screens = Screens::new(&ui);
    let mut previous = ui.clone();
    for (slot, in_drawer) in [(5, true), (2, false), (5, false), (2, true)] {
        ui.deck.notices[slot].in_drawer = in_drawer;
        ui.invalidate();
        let moved = update(&mut screens, &previous, &ui);
        assert!(moved < 4000, "{moved} pixels for the pill");
        previous = ui.clone();
    }
}

fn drawer_with(notices: &[(u8, u16, CardKind)]) -> Box<UiState> {
    let mut ui = with_overlay(Overlay::Drawer);
    ui.deck.notices = [Notice::EMPTY; 8];
    for (slot, &(agent, card, kind)) in notices.iter().enumerate() {
        ui.deck.notices[slot] = Notice::new(agent, card, kind, 0).filed();
    }
    ui.notify.drawer_sel = notices.first().map_or((0, 0), |n| (n.0, n.1));
    ui.invalidate();
    ui
}

#[test]
fn drawer_rows_follow_the_viewport_and_the_selection_where_layout_says() {
    let kinds = [CardKind::Question, CardKind::Ready, CardKind::Ready, CardKind::Question, CardKind::Ready, CardKind::Question, CardKind::Ready];
    let notices: Vec<(u8, u16, CardKind)> = kinds.iter().enumerate().map(|(i, &k)| (1 + (i % 4) as u8, i as u16 + 1, k)).collect();
    let mut ui = drawer_with(&notices);
    let dot_of = |frame: &[u8], slot: u8| {
        let rect = layout::drawer_row(slot);
        pixel(frame, rect.left() + 21, rect.top() + 27)
    };
    let kind_color = |kind: CardKind| if kind == CardKind::Question { theme::RED } else { theme::GREEN };
    for top in 0..3u8 {
        ui.notify.drawer_top = top;
        ui.invalidate();
        let frame = full(&ui);
        assert_eq!(lit_outside_safe_area(&frame), None);
        assert_hint_bar_fits(&frame, "drawer");
        for slot in 0..layout::DRAWER_VISIBLE {
            let rect = layout::drawer_row(slot);
            let (cx, cy) = rect.center();
            assert_eq!(layout::drawer_row_at(cx, cy), Some(slot));
            let notice = notices[(top + slot) as usize];
            assert_eq!(dot_of(&frame, slot), rgb(kind_color(notice.2)), "top {top} slot {slot}");
            let selected = (notice.0, notice.1) == ui.notify.drawer_sel;
            let expected = if selected { theme::OUTLINE_SELECTED } else { theme::OUTLINE };
            assert_eq!(pixel(&frame, rect.left(), cy), rgb(expected), "top {top} slot {slot}");
        }
    }
    // Moving the selection repaints the two rows, not the frame.
    ui.notify.drawer_top = 0;
    ui.invalidate();
    let mut screens = Screens::new(&ui);
    let before = ui.clone();
    ui.notify.drawer_sel = (notices[2].0, notices[2].1);
    ui.invalidate();
    let moved = update(&mut screens, &before, &ui);
    assert!(moved <= 2 * 418 * 56, "{moved}");
    // Scrolling, removing a notice, and the card text changing all refresh partially.
    let mut out = Update::new();
    for change in [0u8, 1, 2, 3] {
        let before = ui.clone();
        match change {
            0 => ui.notify.drawer_top = 2,
            1 => ui.deck.notices[0].used = false,
            2 => {
                let mut choices = [Text::new(); 4];
                choices[0].set("Yes");
                ui.host(
                    &HostMsg::Card { agent: 2, id: 2, clear: false, kind: CardKind::Ready, text: Text::from_str("Deployed the header refactor"), choices, choice_count: 1, recommended: None },
                    100,
                    &mut out,
                );
            }
            _ => ui.deck.notices[2].kind = CardKind::Question,
        }
        ui.invalidate();
        update(&mut screens, &before, &ui);
    }
    // An empty drawer says so, in the body only.
    let empty = drawer_with(&[]);
    let frame = full(&empty);
    assert_eq!(lit_in(&frame, layout::drawer_row(0)), 0);
    assert!(lit_in(&frame, Rect::new(60, 200, 360, 80)) > 100, "empty state");
    let before = ui.clone();
    ui.deck.notices = [Notice::EMPTY; 8];
    ui.invalidate();
    update(&mut screens, &before, &ui);
}

fn manager_with(selected: u8, first_visible: u8, grabbed: bool) -> Box<UiState> {
    let mut ui = with_overlay(Overlay::Manager);
    ui.manager.selected = selected;
    ui.manager.first_visible = first_visible;
    ui.manager.grabbed = grabbed;
    ui.invalidate();
    ui
}

#[test]
fn manager_cards_sit_in_their_slots_and_the_selected_one_is_lifted() {
    let ui = manager_with(0, 0, false);
    assert_eq!(ui.global.strip_len, 5);
    for selected in 0..6u8 {
        for first_visible in 0..3u8 {
            let ui = manager_with(selected, first_visible, false);
            let frame = full(&ui);
            assert_eq!(lit_outside_safe_area(&frame), None);
            for slot in 0..layout::MANAGER_VISIBLE {
                let (cx, cy) = layout::manager_slot(slot).center();
                assert_eq!(layout::manager_card_at(cx, cy), Some(slot));
                let index = first_visible + slot;
                let card = layout::manager_card(slot);
                let lifted = card.offset(0, -layout::MANAGER_LIFT);
                let (x, _) = card.center();
                if index == 5 {
                    // The "+" card: a plus in the middle, lifted with the selection.
                    let (px, py) = if index == selected { lifted.center() } else { card.center() };
                    assert_eq!(pixel(&frame, px, py), rgb(theme::PLUS), "{selected}/{first_visible}/{slot}");
                    assert_ne!(pixel(&frame, px, py + 10 * (1 - 2 * i32::from(index == selected))), rgb(theme::PLUS));
                } else if index == selected {
                    assert_eq!(pixel(&frame, x, lifted.top()), rgb(theme::TEXT_PRIMARY), "{selected}/{first_visible}/{slot}");
                    assert_eq!(pixel(&frame, x, lifted.bottom() - 1), rgb(theme::TEXT_PRIMARY));
                } else {
                    assert_eq!(pixel(&frame, x, card.top()), rgb(theme::OUTLINE_THUMB), "{selected}/{first_visible}/{slot}");
                    assert_eq!(pixel(&frame, x, lifted.top()), 0);
                }
            }
        }
    }
    // Grabbing brightens the selected card without moving anything else.
    let plain = full(&manager_with(2, 0, false));
    let grabbed = full(&manager_with(2, 0, true));
    let slot = layout::manager_slot(2);
    assert!(diff_box(&plain, &grabbed).intersects(&slot));
    assert_eq!(diff_outside(&plain, &grabbed, &[slot, layout::HINT_BAR]), None);
    let card = layout::manager_card(2).offset(0, -layout::MANAGER_LIFT);
    assert_eq!(pixel(&grabbed, card.center().0, card.top() + 5), rgb(theme::CHIP_FILL));
    assert_eq!(pixel(&plain, card.center().0, card.top() + 5), rgb(theme::THUMB_FILL));
    // Eight faces: no "+" card anywhere.
    let mut ui = manager_with(7, 4, false);
    for index in 5..8 {
        ui.global.strip[index] = FaceEntry::new(index as u8 + 1, Template::Usage, "Usage");
    }
    ui.global.strip_len = 8;
    ui.invalidate();
    let frame = full(&ui);
    let card = layout::manager_card(3).offset(0, -layout::MANAGER_LIFT);
    assert_eq!(pixel(&frame, card.center().0, card.top()), rgb(theme::TEXT_PRIMARY));
    assert_ne!(pixel(&frame, card.center().0, card.center().1), rgb(theme::PLUS));
}

#[test]
fn gallery_confirm_and_naming_draw_where_layout_hit_tests() {
    let mut ui = with_overlay(Overlay::Gallery);
    for sel in 0..layout::GALLERY_ROWS {
        ui.manager.gallery_sel = sel;
        ui.manager.pressed = None;
        ui.invalidate();
        let frame = full(&ui);
        assert_eq!(lit_outside_safe_area(&frame), None);
        assert_hint_bar_fits(&frame, "gallery");
        for row in 0..layout::GALLERY_ROWS {
            let rect = layout::gallery_row(row);
            let (cx, cy) = rect.center();
            assert_eq!(layout::gallery_row_at(cx, cy), Some(row));
            let expected = if row == sel { theme::OUTLINE_SELECTED } else { theme::OUTLINE };
            assert_eq!(pixel(&frame, rect.left(), cy), rgb(expected), "gallery {sel}/{row}");
        }
        ui.manager.pressed = Some(sel);
        ui.invalidate();
        let pressed = full(&ui);
        let rect = layout::gallery_row(sel);
        assert_eq!(rect.union(&diff_box(&frame, &pressed)), rect);
        assert_eq!(pixel(&pressed, rect.center().0, rect.top() + 6), rgb(theme::CHIP_FILL));
    }
    // A third Macropad greys the Macropad row.
    let plain = full(&ui);
    for index in [1, 3] {
        ui.global.strip[index].template = Template::Macropad;
    }
    ui.invalidate();
    let refused = full(&ui);
    assert_eq!(layout::gallery_row(1).union(&diff_box(&plain, &refused)), layout::gallery_row(1));

    let mut ui = with_overlay(Overlay::ConfirmRemove);
    ui.manager.confirm_face = 2;
    ui.invalidate();
    let frame = full(&ui);
    assert_eq!(lit_outside_safe_area(&frame), None);
    assert_hint_bar_fits(&frame, "confirm");
    assert!(lit_in(&frame, layout::CONFIRM_TEXT) > 200, "the question and the name");
    for index in 0..2u8 {
        let rect = layout::confirm_button(index);
        let (cx, cy) = rect.center();
        assert_eq!(layout::confirm_button_at(cx, cy), Some(index));
        let expected = if index == 1 { theme::RED } else { theme::OUTLINE_THUMB };
        assert_eq!(pixel(&frame, rect.left(), cy), rgb(expected));
        ui.manager.pressed = Some(index);
        ui.invalidate();
        let pressed = full(&ui);
        assert_eq!(rect.union(&diff_box(&frame, &pressed)), rect, "confirm button {index}");
        assert_eq!(pixel(&pressed, cx, rect.top() + 5), rgb(theme::CHIP_FILL));
    }
    // A confirm about a face that is gone still draws a whole dialog.
    ui.manager.pressed = None;
    ui.manager.confirm_face = 200;
    ui.invalidate();
    assert!(lit_in(&full(&ui), layout::CONFIRM_TEXT) > 100);

    let mut ui = with_overlay(Overlay::Naming);
    ui.manager.naming_template = Template::Macropad;
    ui.manager.naming_text.clear();
    ui.invalidate();
    let placeholder = full(&ui);
    assert_eq!(lit_outside_safe_area(&placeholder), None);
    assert_hint_bar_fits(&placeholder, "naming");
    assert!(lit_in(&placeholder, layout::NAMING_FIELD) > 300, "the template name stands in");
    ui.manager.naming_text.set("Deploy");
    ui.invalidate();
    let typed = full(&ui);
    let diff = diff_box(&placeholder, &typed);
    assert!(diff.intersects(&layout::NAMING_FIELD) && diff.intersects(&layout::NAMING_HELP));
    assert!(!diff.intersects(&layout::NAMING_TITLE) && !diff.intersects(&layout::HEADER));
    ui.manager.naming_template = Template::Usage;
    ui.invalidate();
    let retitled = diff_box(&typed, &full(&ui));
    assert!(retitled.intersects(&layout::NAMING_TITLE) && retitled.intersects(&layout::HEADER));
}

#[test]
fn the_manager_family_refreshes_partially_through_a_script() {
    let mut ui = manager_with(0, 0, false);
    let mut screens = Screens::new(&ui);
    let mut out = Update::new();
    let step = |ui: &mut Box<UiState>, screens: &mut Screens, change: &mut dyn FnMut(&mut UiState)| {
        let before = ui.clone();
        change(ui);
        ui.invalidate();
        update(screens, &before, ui)
    };
    for selected in 1..7u8 {
        let moved = step(&mut ui, &mut screens, &mut |ui| {
            ui.manager.selected = selected;
            ui.manager.first_visible = selected.saturating_sub(3);
        });
        if selected <= 3 {
            assert!(moved < 480 * 480 / 2, "{moved} pixels for a selection move");
        }
    }
    step(&mut ui, &mut screens, &mut |ui| ui.manager.selected = 2);
    step(&mut ui, &mut screens, &mut |ui| ui.manager.grabbed = true);
    step(&mut ui, &mut screens, &mut |ui| {
        // The grabbed card moves one slot right: swap strip entries.
        ui.global.strip.swap(2, 3);
        ui.manager.selected = 3;
    });
    step(&mut ui, &mut screens, &mut |ui| ui.manager.grabbed = false);
    step(&mut ui, &mut screens, &mut |ui| {
        ui.global.strip[3].name.set("Deploy pad");
        ui.host(&HostMsg::Hello { v: 1, sid: 9, minute_of_day: Some(600) }, 5, &mut out);
    });
    step(&mut ui, &mut screens, &mut |ui| ui.global.overlay = Overlay::ConfirmRemove);
    step(&mut ui, &mut screens, &mut |ui| ui.manager.confirm_face = 4);
    step(&mut ui, &mut screens, &mut |ui| ui.manager.pressed = Some(1));
    step(&mut ui, &mut screens, &mut |ui| {
        ui.manager.pressed = None;
        ui.global.overlay = Overlay::Manager;
    });
    step(&mut ui, &mut screens, &mut |ui| ui.manager.selected = 6);
    step(&mut ui, &mut screens, &mut |ui| ui.global.overlay = Overlay::Gallery);
    step(&mut ui, &mut screens, &mut |ui| ui.manager.gallery_sel = 3);
    step(&mut ui, &mut screens, &mut |ui| ui.manager.pressed = Some(3));
    step(&mut ui, &mut screens, &mut |ui| {
        ui.manager.pressed = None;
        ui.global.overlay = Overlay::Naming;
        ui.manager.naming_template = Template::Usage;
    });
    step(&mut ui, &mut screens, &mut |ui| ui.manager.naming_text.set("Lim"));
    step(&mut ui, &mut screens, &mut |ui| {
        ui.host(&HostMsg::Name { text: Text::from_str("Limits") }, 6, &mut out);
        ui.manager.naming_text.set("Limits");
    });
    step(&mut ui, &mut screens, &mut |ui| ui.manager.naming_template = Template::Macropad);
    step(&mut ui, &mut screens, &mut |ui| {
        ui.update_battery(crate::battery::BatteryState {
            estimated_percent: Some(5),
            charge: crate::battery::ChargeState::Battery,
            ..crate::battery::BatteryState::UNKNOWN
        });
    });
    step(&mut ui, &mut screens, &mut |ui| ui.global.overlay = Overlay::None);
}

#[test]
fn every_manager_view_keeps_its_hint_bar_inside_and_fits_the_mockup_strings() {
    let mut states = vec![manager_with(0, 0, false), manager_with(1, 0, false), manager_with(6, 3, false), manager_with(1, 0, true)];
    let mut ui = with_overlay(Overlay::ConfirmRemove);
    ui.manager.confirm_face = 3;
    states.push(ui);
    for ui in &states {
        let frame = full(ui);
        assert_eq!(lit_outside_safe_area(&frame), None);
        assert_hint_bar_fits(&frame, "manager");
    }
    // The mockup's three hints all fit; the "done" hint is dropped only when it must.
    let mockup = [
        kit::Hint { key: "ENTER", label: "grab / drop" },
        kit::Hint { key: "SCROLL", label: "move" },
        kit::Hint { key: "CANCEL", label: "remove" },
    ];
    assert_eq!(kit::hints_that_fit(&mockup).0, 3);
    let frame = full(&manager_with(1, 0, false));
    let mockup_frame = full_draw(|band| {
        overlay::manager::draw(band, &manager_with(1, 0, false));
    });
    assert_frame(&frame, &mockup_frame, "the overlay alone is the frame (opaque)");
}

#[test]
fn overlays_and_toasts_draw_at_every_origin_and_clip_without_panicking() {
    let mut states = vec![manager_with(2, 1, true), manager_with(6, 3, false)];
    let mut confirm = with_overlay(Overlay::ConfirmRemove);
    confirm.manager.pressed = Some(0);
    states.push(confirm);
    let mut gallery = with_overlay(Overlay::Gallery);
    gallery.manager.pressed = Some(2);
    states.push(gallery);
    states.push(drawer_with(&[(1, 1, CardKind::Question), (2, 2, CardKind::Ready)]));
    states.push(drawer_with(&[]));
    for step in crate::ui::state::tutorial::Step::ALL {
        states.push(tutorial_at(step));
    }
    let mut reset = hosted(Template::Settings);
    reset.global.overlay = Overlay::ConfirmReset;
    reset.settings.pressed = Some(crate::ui::state::settings::CONFIRM_PRESSED);
    states.push(reset);
    states.push(macro_edit(true));
    let mut cursor = hosted(Template::Macropad);
    cursor.macropad.cursor = Some((cursor.current_face().id, 4));
    cursor.invalidate();
    states.push(cursor);
    for now in [0u32, 40, 130, 5000] {
        let mut toast = hosted(Template::Usage);
        toast.now_ms = now;
        toast.notify.toast = Some(Toast::card(1, 1, CardKind::Question, 0));
        toast.notify.toast_pressed = now == 130;
        states.push(toast);
    }
    let mut band = display::Band::new();
    for ui in &states {
        let face = ui.current_face();
        for origin in -480i32..=480 {
            let bands: &[u16] = if origin % 8 == 0 { &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9] } else { &[0, 3, 9] };
            for &index in bands {
                band.set_row(index);
                if origin % 7 == 0 {
                    band.clip_to(Rectangle::new(Point::new((origin + 480) / 3, index as i32 * 48 + 7), Size::new(83, 19)));
                }
                band.set_origin_x(origin);
                if !overlay::opaque(ui) {
                    scene::draw_face(&mut band, ui, face);
                }
                overlay::draw(&mut band, ui);
            }
        }
    }
}

fn tutorial_at(step: crate::ui::state::tutorial::Step) -> Box<UiState> {
    let mut ui = hosted(Template::Agents);
    ui.start_tutorial(false);
    ui.tutorial.step = step;
    ui.invalidate();
    ui
}

#[test]
fn the_tutorial_pages_stay_inside_the_arcs_and_a_first_run_refreshes_partially() {
    use crate::control::{Button, Control, Edge, Wheel};
    use crate::ui::state::tutorial::{Step, BUTTONS, PANEL_SWITCHES, WHEEL_DETENTS};
    let mut states: Vec<Box<UiState>> = Step::ALL.iter().map(|step| tutorial_at(*step)).collect();
    for (index, button) in BUTTONS.iter().enumerate() {
        let mut asking = tutorial_at(Step::Buttons);
        asking.tutorial.index = index as u8;
        asking.tutorial.named = Some(*button);
        asking.tutorial.wrong = index % 2 == 1;
        states.push(asking);
    }
    let mut listening = tutorial_at(Step::Ptt);
    listening.tutorial.ptt_down = true;
    states.push(listening);
    let mut needs_shift = tutorial_at(Step::Latest);
    needs_shift.tutorial.needs_shift = true;
    states.push(needs_shift);
    let mut replay = tutorial_at(Step::Done);
    replay.tutorial.replay = true;
    states.push(replay);
    let mut full_strip = tutorial_at(Step::Panels);
    for index in 5..8 {
        full_strip.global.strip[index] = FaceEntry::new(index as u8 + 1, Template::Macropad, "Deploy pad");
    }
    full_strip.global.strip_len = 8;
    full_strip.tutorial.panel = 7;
    states.push(full_strip);
    for ui in &states {
        let frame = full(ui);
        assert_eq!(lit_outside_safe_area(&frame), None, "{:?}", ui.tutorial.step);
        assert_hint_bar_fits(&frame, "tutorial");
        assert!(overlay::opaque(ui));
        let alone = full_draw(|band| overlay::draw(band, ui));
        assert_frame(&frame, &alone, "the tutorial alone is the frame");
    }
    // A scripted first run: every frame refreshes partially and equals a
    // full redraw.
    let mut ui = tutorial_at(Step::Buttons);
    let mut screens = Screens::new(&ui);
    let apply = |ui: &mut Box<UiState>, screens: &mut Screens, control: Control| {
        let before = ui.clone();
        ui.apply(control, 10, &mut Update::new());
        update(screens, &before, ui)
    };
    // A wrong key repaints the caption lane only; the right one the page
    // below the header (the next key lights, the title changes).
    let moved = apply(&mut ui, &mut screens, Control::Button(Button::Right, Edge::Press));
    assert!(moved <= 480 * 48, "{moved} pixels for a wrong key");
    apply(&mut ui, &mut screens, Control::Button(Button::Right, Edge::Release));
    for button in BUTTONS {
        let moved = apply(&mut ui, &mut screens, Control::Button(button, Edge::Press));
        // The fifth key ends the step: a new page, drawn in full.
        if button != *BUTTONS.last().unwrap() {
            assert!(moved <= 480 * (480 - 60 - 52), "{moved} pixels for one asked-for key");
        }
        apply(&mut ui, &mut screens, Control::Button(button, Edge::Release));
    }
    assert_eq!(ui.tutorial.step, Step::Wheel);
    for _ in 0..WHEEL_DETENTS {
        apply(&mut ui, &mut screens, Control::Wheel(Wheel::Cw));
    }
    assert_eq!(ui.tutorial.step, Step::Panels);
    apply(&mut ui, &mut screens, Control::Wheel(Wheel::Cw));
    apply(&mut ui, &mut screens, Control::Button(Button::Shift, Edge::Press));
    for _ in 0..PANEL_SWITCHES {
        apply(&mut ui, &mut screens, Control::Wheel(Wheel::Cw));
    }
    apply(&mut ui, &mut screens, Control::Button(Button::Shift, Edge::Release));
    assert_eq!(ui.tutorial.step, Step::Latest);
    apply(&mut ui, &mut screens, Control::Button(Button::Enter, Edge::Press));
    apply(&mut ui, &mut screens, Control::Button(Button::Enter, Edge::Release));
    assert_eq!(ui.tutorial.step, Step::Latest);
    apply(&mut ui, &mut screens, Control::Button(Button::Shift, Edge::Press));
    apply(&mut ui, &mut screens, Control::Button(Button::Enter, Edge::Press));
    apply(&mut ui, &mut screens, Control::Button(Button::Enter, Edge::Release));
    apply(&mut ui, &mut screens, Control::Button(Button::Shift, Edge::Release));
    assert_eq!(ui.tutorial.step, Step::Ptt);
    apply(&mut ui, &mut screens, Control::Button(Button::PushToTalk, Edge::Press));
    apply(&mut ui, &mut screens, Control::Button(Button::PushToTalk, Edge::Release));
    assert_eq!(ui.tutorial.step, Step::Done);
    apply(&mut ui, &mut screens, Control::Button(Button::Enter, Edge::Press));
    assert_eq!(ui.global.overlay, Overlay::None);
}

#[test]
fn the_factory_reset_dialog_sits_over_settings_and_only_its_card_and_bar_change() {
    use crate::ui::state::settings::{CONFIRM_PRESSED, FACTORY_RESET_ROW};
    let mut ui = hosted(Template::Settings);
    ui.settings.panel = FACTORY_RESET_ROW.0;
    ui.settings.row = FACTORY_RESET_ROW.1;
    ui.global.overlay = Overlay::ConfirmReset;
    ui.invalidate();
    let frame = full(&ui);
    assert_eq!(lit_outside_safe_area(&frame), None);
    assert_hint_bar_fits(&frame, "factory reset");
    assert!(!overlay::opaque(&ui));
    let mut plain = ui.clone();
    plain.global.overlay = Overlay::None;
    plain.invalidate();
    assert_eq!(
        diff_outside(&full(&plain), &frame, &[layout::CONFIRM_BOX, layout::HINT_BAR]),
        None,
        "the Settings face shows through around the card"
    );
    let mut screens = Screens::new(&ui);
    let before = ui.clone();
    ui.settings.pressed = Some(CONFIRM_PRESSED + 1);
    ui.invalidate();
    let moved = update(&mut screens, &before, &ui);
    assert!(moved <= (layout::CONFIRM_BOX.width() * layout::CONFIRM_BOX.height()) as usize, "{moved}");
    let before = ui.clone();
    ui.settings.pressed = None;
    ui.global.overlay = Overlay::None;
    ui.invalidate();
    update(&mut screens, &before, &ui);
}

fn macro_edit(present: bool) -> Box<UiState> {
    let mut ui = hosted(Template::Macropad);
    ui.global.overlay = Overlay::MacroEdit;
    ui.macropad.edit.face = ui.current_face().id;
    ui.macropad.edit.index = 4;
    ui.macropad.edit.present = present;
    if present {
        ui.macropad.edit.label.set("screenshot");
        ui.macropad.edit.keys = crate::deck::msg::Keystroke::parse("cmd+shift+4").unwrap();
    }
    ui.invalidate();
    ui
}

#[test]
fn the_key_editor_and_the_grid_cursor_stay_inside_the_arcs_and_refresh_partially() {
    use crate::ui::state::macropad::{ROW_KEY, ROW_MODS, ROW_REMOVE, ROW_SAVE};
    for present in [false, true] {
        let ui = macro_edit(present);
        let frame = full(&ui);
        assert_eq!(lit_outside_safe_area(&frame), None, "present {present}");
        assert_hint_bar_fits(&frame, "key editor");
        assert!(overlay::opaque(&ui));
    }
    let mut ui = macro_edit(true);
    let mut screens = Screens::new(&ui);
    let step = |ui: &mut Box<UiState>, screens: &mut Screens, change: &mut dyn FnMut(&mut UiState)| {
        let before = ui.clone();
        change(ui);
        ui.invalidate();
        update(screens, &before, ui)
    };
    let moved = step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.row = ROW_MODS);
    // Two rows, each padded to even band rows by the damage tracker.
    assert!(moved <= 2 * (layout::macro_edit_row(0).width() * (layout::macro_edit_row(0).height() + 4)) as usize, "{moved} for a row move");
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.adjusting = true);
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.keys.mods = crate::deck::msg::MOD_CMD | crate::deck::msg::MOD_CTRL);
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.row = ROW_KEY);
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.keys.key = 1);
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.adjusting = false);
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.pressed = Some(ROW_SAVE));
    step(&mut ui, &mut screens, &mut |ui| {
        ui.macropad.edit.pressed = None;
        ui.macropad.edit.row = ROW_REMOVE;
    });
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.label.clear());
    step(&mut ui, &mut screens, &mut |ui| ui.macropad.edit.needs_label = true);
    // The cursor on the grid repaints one key as it moves.
    let mut ui = hosted(Template::Macropad);
    let face = ui.current_face().id;
    let mut screens = Screens::new(&ui);
    // One key plus the hint bar, then two keys of one row (merged per band
    // by the damage tracker): well under a quarter of the screen each,
    // never a full redraw.
    let moved = step(&mut ui, &mut screens, &mut |ui| ui.macropad.cursor = Some((face, 4)));
    assert!(moved <= 480 * 480 / 4, "{moved}");
    let moved = step(&mut ui, &mut screens, &mut |ui| ui.macropad.cursor = Some((face, 5)));
    assert!(moved <= 480 * 480 / 4, "{moved}");
    let frame = full(&ui);
    assert_eq!(lit_outside_safe_area(&frame), None);
    assert_hint_bar_fits(&frame, "cursor");
}
