//! Replay partial damage for real simulated game frames and all menus.
use super::*;
use crate::{
    control::{Button, Control, Edge},
    ui::state::{
        arcade::{games, Game, Phase},
        Update,
    },
};
fn press(ui: &mut UiState, b: Button, now: u32) {
    ui.apply(Control::Button(b, Edge::Press), now, &mut Update::new());
    ui.apply(Control::Button(b, Edge::Release), now, &mut Update::new());
}

#[test]
fn gameplay_reaches_all_edges_and_hides_passive_status_chrome() {
    use crate::ui::{
        bluetooth::{Level, Status},
        state::Overlay,
    };
    for game in Game::ALL {
        let mut ui = on(Template::Arcade);
        ui.deck.settings.demo = false;
        ui.arcade.selection = game as u8;
        press(&mut ui, Button::Enter, 0);
        press(&mut ui, Button::Enter, 0);
        let frame = full(&ui);
        let canvas = crate::ui::kit::rgb(crate::ui::theme::CANVAS).into_storage();
        for (x, y) in [(0, 240), (240, 0), (479, 240), (240, 479)] {
            let at = (y * 480 + x) * 2;
            assert_ne!(u16::from_le_bytes([frame[at], frame[at + 1]]), canvas);
        }
        assert_eq!(lit_outside_safe_area(&frame), None);
        let mut screens = Screens::new(&ui);
        let old = *ui;
        ui.update_bluetooth(Status::Signal(Level::Strong));
        assert_eq!(update(&mut screens, &old, &ui), 0);
        let old = *ui;
        ui.global.overlay = Overlay::Drawer;
        update(&mut screens, &old, &ui);
        let old = *ui;
        ui.global.overlay = Overlay::None;
        update(&mut screens, &old, &ui);
        if game == Game::Raiden {
            for x in [games::PLAYER_MIN_X, games::PLAYER_MAX_X] {
                let old = *ui;
                ui.arcade.raiden.x = x;
                ui.arcade.visual += 1;
                update(&mut screens, &old, &ui);
                assert_eq!(lit_outside_safe_area(&full(&ui)), None);
            }
        }
        let old = *ui;
        press(&mut ui, Button::Cancel, 10);
        update(&mut screens, &old, &ui);
        let old = *ui;
        ui.update_bluetooth(Status::Signal(Level::Weak));
        assert!(
            update(&mut screens, &old, &ui) > 0,
            "status returns in the menu"
        );
    }
}
#[test]
fn arcade_menu_and_games_fit_and_partial_updates_match_full_frames() {
    let mut ui = on(Template::Arcade);
    ui.deck.settings.demo = false;
    assert_eq!(lit_outside_safe_area(&full(&ui)), None);
    let mut screens = Screens::new(&ui);
    for game in Game::ALL {
        ui.arcade.selection = game as u8;
        let old = *ui;
        press(&mut ui, Button::Enter, 10);
        update(&mut screens, &old, &ui);
        assert_eq!(ui.arcade.phase, Phase::Ready);
        assert_eq!(lit_outside_safe_area(&full(&ui)), None);
        let old = *ui;
        press(&mut ui, Button::Enter, 20);
        update(&mut screens, &old, &ui);
        for now in (40..=440).step_by(40) {
            let old = *ui;
            ui.tick(now, 0, true, &mut Update::new());
            update(&mut screens, &old, &ui);
        }
        assert_eq!(lit_outside_safe_area(&full(&ui)), None);
        let old = *ui;
        press(&mut ui, Button::Enter, 460);
        update(&mut screens, &old, &ui);
        let old = *ui;
        press(&mut ui, Button::Cancel, 480);
        update(&mut screens, &old, &ui);
        // Reset preview clock for the next game; production uses monotonic time.
        ui.now_ms = 0;
    }
}
