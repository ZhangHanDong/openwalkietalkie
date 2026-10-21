//! Arcade is entirely local. PTT never becomes a voice event on this face.
use super::{global, Overlay, Owner, Source, Target, TouchAct, UiState, Update};
use crate::control::{Button, Control, Edge, Wheel};
use crate::deck::msg::{Event, SettingKey, Template};
use crate::ui::layout::Rect;
pub mod games;
pub mod sound;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Game {
    Flappy,
    Snake,
    Raiden,
}
impl Game {
    pub const ALL: [Self; 3] = [Self::Flappy, Self::Snake, Self::Raiden];
    pub const fn title(self) -> &'static str {
        match self {
            Self::Flappy => "FLAPPY BIRD",
            Self::Snake => "SNAKE",
            Self::Raiden => "RAIDEN",
        }
    }
    pub const fn controls(self) -> &'static str {
        match self {
            Self::Flappy => "PTT to fly",
            Self::Snake => "Wheel / Left / Right to turn",
            Self::Raiden => "Left / Right to move · auto-fire",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Menu,
    Ready,
    Playing,
    Paused,
    Over,
}
#[derive(Clone, Copy)]
pub struct View {
    pub selection: u8,
    pub game: Game,
    pub phase: Phase,
    pub best: [u16; 3],
    pub flappy: games::Flappy,
    pub snake: games::Snake,
    pub raiden: games::Raiden,
    pub held: u8,
    pub visual: u32,
    pub effect: sound::Effect,
    pub effect_seq: u16,
    last_ms: u32,
    remainder: u32,
    seed: u32,
}
impl View {
    pub const fn new() -> Self {
        Self {
            selection: 0,
            game: Game::Flappy,
            phase: Phase::Menu,
            best: [0; 3],
            flappy: games::Flappy::new(),
            snake: games::Snake::new(),
            raiden: games::Raiden::new(),
            held: 0,
            visual: 0,
            effect: sound::Effect::None,
            effect_seq: 0,
            last_ms: 0,
            remainder: 0,
            seed: 1,
        }
    }
    pub fn score(&self) -> u16 {
        match self.game {
            Game::Flappy => self.flappy.score,
            Game::Snake => self.snake.score(),
            Game::Raiden => self.raiden.score,
        }
    }
    fn fx(&mut self, effect: sound::Effect) {
        self.effect = effect;
        self.effect_seq = self.effect_seq.wrapping_add(1);
    }
    fn prepare(&mut self, now: u32) {
        self.game = Game::ALL[self.selection.min(2) as usize];
        self.phase = Phase::Ready;
        self.flappy = games::Flappy::new();
        self.snake = games::Snake::new();
        self.raiden = games::Raiden::new();
        self.last_ms = now;
        self.remainder = 0;
        self.held = 0;
        self.seed = now ^ 0x504c4159;
        self.effect = sound::Effect::None;
    }
    fn start(&mut self, now: u32) {
        self.phase = Phase::Playing;
        self.last_ms = now;
        self.remainder = 0;
        self.fx(sound::Effect::Start);
    }
}
pub const FIELD: Rect = Rect::SCREEN;
pub const SOUND: Rect = Rect::new(64, 370, 352, 42);
pub const fn menu_row(i: u8) -> Rect {
    Rect::new(48, 90 + i as i32 * 90, 384, 78)
}
pub fn active(ui: &UiState) -> bool {
    ui.current_face().template == Template::Arcade && ui.global.overlay == Overlay::None
}
/// Keep system status chrome on the menu, outside immersive gameplay.
pub fn fullscreen(ui: &UiState) -> bool {
    active(ui) && ui.arcade.phase != Phase::Menu && ui.strip_at_rest()
}
pub fn playing(ui: &UiState) -> bool {
    active(ui) && ui.arcade.phase == Phase::Playing && ui.strip_at_rest()
}
pub fn sound_label(mode: u8) -> &'static str {
    match mode {
        0 => "Off",
        1 => "SFX only",
        _ => "Music + SFX",
    }
}
pub fn sound_command(ui: &UiState) -> u32 {
    if !playing(ui) && !(active(ui) && ui.arcade.phase == Phase::Over) {
        return 0;
    }
    sound::command(
        if playing(ui) {
            ui.arcade.game as u8 + 1
        } else {
            0
        },
        ui.deck.settings.arcade_sound,
        ui.arcade.effect,
        ui.arcade.effect_seq,
        ui.deck.settings.speaker,
    )
}
pub(super) fn suspend(ui: &mut UiState, out: &mut Update) {
    if ui.arcade.phase == Phase::Playing {
        ui.arcade.phase = Phase::Paused;
        out.dirty = true;
    }
    ui.arcade.held = 0;
}
pub(super) fn released(ui: &mut UiState, button: Button) {
    ui.arcade.held &= !button.bit();
}
fn activate(ui: &mut UiState, now: u32, out: &mut Update) {
    if ui.arcade.phase == Phase::Menu {
        if ui.arcade.selection == 3 {
            let value = (ui.deck.settings.arcade_sound + 1) % 3;
            ui.deck.settings.arcade_sound = value;
            out.push_deck(Event::Set {
                key: SettingKey::ArcadeSound,
                value: u16::from(value),
            });
        } else {
            ui.arcade.prepare(now);
        }
    } else {
        match ui.arcade.phase {
            Phase::Ready | Phase::Paused => ui.arcade.start(now),
            Phase::Playing => suspend(ui, out),
            Phase::Over => {
                ui.arcade.prepare(now);
                ui.arcade.start(now);
            }
            Phase::Menu => {}
        }
    }
    out.dirty = true;
}
pub(super) fn on_control(ui: &mut UiState, control: Control, now: u32, out: &mut Update) -> bool {
    if ui.global.shift {
        return true;
    }
    if let Control::Button(button, Edge::Press) = control {
        if global::held(ui, Source::Button(button)).is_some() {
            return true;
        }
        global::hold(ui, Source::Button(button), Owner::Arcade(button));
        ui.arcade.held |= button.bit();
    }
    match control {
        Control::Button(Button::Cancel, Edge::Press) => {
            if ui.arcade.phase == Phase::Menu {
                global::go_home(ui, now, out);
            } else {
                ui.arcade.phase = Phase::Menu;
                ui.arcade.held = 0;
            }
            out.dirty = true;
        }
        Control::Button(Button::Enter, Edge::Press) => activate(ui, now, out),
        Control::Button(Button::PushToTalk, Edge::Press) => {
            if ui.arcade.game == Game::Flappy
                && matches!(ui.arcade.phase, Phase::Ready | Phase::Playing)
            {
                if ui.arcade.phase == Phase::Ready {
                    ui.arcade.start(now);
                }
                ui.arcade.flappy.flap();
                ui.arcade.fx(sound::Effect::Flap);
                out.dirty = true;
            }
        }
        Control::Wheel(wheel) => {
            let right = if ui.arcade.phase == Phase::Menu {
                super::selection_step(wheel) > 0
            } else {
                wheel == Wheel::Cw
            };
            turn(ui, right, out);
        }
        Control::Button(Button::Left, Edge::Press) => turn(ui, false, out),
        Control::Button(Button::Right, Edge::Press) => turn(ui, true, out),
        _ => {}
    }
    true
}
fn turn(ui: &mut UiState, right: bool, out: &mut Update) {
    if ui.arcade.phase == Phase::Menu {
        ui.arcade.selection = (ui.arcade.selection + if right { 1 } else { 3 }) % 4;
        out.dirty = true;
    } else if ui.arcade.phase == Phase::Playing && ui.arcade.game == Game::Snake {
        ui.arcade.snake.turn(right);
    }
}
pub(super) fn hit(ui: &UiState, x: i32, y: i32) -> Option<Target> {
    if ui.arcade.phase != Phase::Menu {
        return None;
    }
    for i in 0..4 {
        let r = if i == 3 { SOUND } else { menu_row(i) };
        if r.contains(x, y) {
            return Some(Target::tap(r, i as u16));
        }
    }
    None
}
pub(super) fn on_touch(
    ui: &mut UiState,
    target: Target,
    act: TouchAct,
    now: u32,
    out: &mut Update,
) {
    if act == TouchAct::Activate {
        ui.arcade.selection = target.id as u8;
        activate(ui, now, out);
    }
}
pub(super) fn on_tick(ui: &mut UiState, now: u32, screen_on: bool, out: &mut Update) {
    if !playing(ui) || !screen_on {
        suspend(ui, out);
        return;
    }
    let a = &mut ui.arcade;
    let dt = now.wrapping_sub(a.last_ms);
    a.last_ms = now;
    // Wake, debugger pauses and missed frames must never teleport into a loss.
    if dt > 250 {
        suspend(ui, out);
        return;
    }
    a.remainder += dt;
    let mut effect = sound::Effect::None;
    while a.remainder >= 20 && a.phase == Phase::Playing {
        a.remainder -= 20;
        let old_head = a.snake.body[0];
        let (alive, score, hit, shot) = match a.game {
            Game::Flappy => {
                let (ok, score) = a.flappy.step(&mut a.seed);
                (ok, score, false, false)
            }
            Game::Snake => {
                let (ok, score) = a.snake.step(&mut a.seed);
                (ok, score, false, false)
            }
            Game::Raiden => a.raiden.step(
                &mut a.seed,
                a.held & Button::Left.bit() != 0,
                a.held & Button::Right.bit() != 0,
            ),
        };
        if shot && effect == sound::Effect::None {
            effect = sound::Effect::Shot;
        }
        if score && effect != sound::Effect::Hit {
            effect = sound::Effect::Score;
        }
        if hit {
            effect = sound::Effect::Hit;
        }
        a.best[a.game as usize] = a.best[a.game as usize].max(a.score());
        if !alive {
            a.phase = Phase::Over;
            a.held = 0;
            effect = if a.game == Game::Snake && a.snake.won {
                sound::Effect::Win
            } else {
                sound::Effect::Over
            };
        }
        if a.game != Game::Snake || old_head != a.snake.body[0] || !alive {
            a.visual = a.visual.wrapping_add(1);
            out.dirty = true;
        }
    }
    if effect != sound::Effect::None {
        a.fx(effect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ui(game: Game) -> std::boxed::Box<UiState> {
        let mut ui = std::boxed::Box::new(UiState::new());
        ui.reset();
        ui.on_face(Template::Arcade);
        ui.arcade.selection = game as u8;
        ui
    }
    fn press(ui: &mut UiState, b: Button, now: u32) -> Update {
        let mut out = Update::new();
        ui.apply(Control::Button(b, Edge::Press), now, &mut out);
        ui.apply(Control::Button(b, Edge::Release), now, &mut out);
        out
    }
    fn begin(ui: &mut UiState, now: u32) {
        press(ui, Button::Enter, now);
        press(ui, Button::Enter, now);
    }
    #[test]
    fn arcade_is_default_before_settings_with_stable_ids() {
        let ui = ui(Game::Flappy);
        assert_eq!(ui.face_at(4).unwrap().template, Template::Arcade);
        assert_eq!(ui.face_at(4).unwrap().id, 6);
        assert_eq!(ui.face_at(5).unwrap().id, 5);
        assert!(ui.still_default());
        assert!(super::super::manager::GALLERY.contains(&Template::Arcade));
    }
    #[test]
    fn ptt_never_leaks_to_voice_even_with_a_live_host() {
        for game in Game::ALL {
            let mut ui = ui(game);
            ui.deck.host_present = true;
            for phase in [
                Phase::Menu,
                Phase::Ready,
                Phase::Playing,
                Phase::Paused,
                Phase::Over,
            ] {
                ui.arcade.phase = phase;
                ui.arcade.game = game;
                let out = press(&mut ui, Button::PushToTalk, 100);
                assert_eq!(out.deck_events().count(), 0);
                assert!(ui.global.ptt.is_none());
            }
        }
    }
    #[test]
    fn ready_pause_resume_and_return_do_not_advance_or_hold_keys() {
        let mut ui = ui(Game::Raiden);
        begin(&mut ui, 1000);
        let mut out = Update::new();
        ui.apply(Control::Button(Button::Left, Edge::Press), 1000, &mut out);
        ui.tick(1040, 0, true, &mut out);
        assert!(ui.arcade.raiden.x < games::W / 2);
        press(&mut ui, Button::Enter, 1040);
        let x = ui.arcade.raiden.x;
        assert_eq!(sound_command(&ui), 0);
        ui.tick(2000, 0, true, &mut out);
        assert_eq!(ui.arcade.raiden.x, x);
        press(&mut ui, Button::Enter, 2000);
        ui.tick(2020, 0, true, &mut out);
        assert_eq!(ui.arcade.raiden.x, x);
        global::set_overlay(&mut ui, Overlay::Manager, 2020, &mut out);
        assert_eq!(ui.arcade.phase, Phase::Paused);
        assert_eq!(ui.arcade.held, 0);
    }
    #[test]
    fn screen_off_stalls_and_navigation_pause_instead_of_catching_up() {
        for dark in [false, true] {
            let mut ui = ui(Game::Snake);
            begin(&mut ui, 10);
            ui.tick(if dark { 30 } else { 1000 }, 0, !dark, &mut Update::new());
            assert_eq!(ui.arcade.phase, Phase::Paused);
        }
        let mut ui = ui(Game::Raiden);
        begin(&mut ui, 100);
        ui.apply(
            Control::Button(Button::Shift, Edge::Press),
            100,
            &mut Update::new(),
        );
        ui.apply(Control::Wheel(Wheel::Cw), 100, &mut Update::new());
        assert_eq!(ui.arcade.phase, Phase::Paused);
        assert_eq!(sound_command(&ui), 0);
    }
    #[test]
    fn wrapped_clock_and_idle_home_leave_active_game_running() {
        let mut ui = ui(Game::Snake);
        begin(&mut ui, u32::MAX - 30);
        ui.tick(9, 240000, true, &mut Update::new());
        assert_eq!(ui.arcade.phase, Phase::Playing);
        assert_eq!(ui.current_face().template, Template::Arcade);
    }
    #[test]
    fn sound_cycles_all_modes_and_emits_persistable_settings() {
        let mut ui = ui(Game::Flappy);
        ui.arcade.selection = 3;
        for mode in [1, 2, 0] {
            let out = press(&mut ui, Button::Enter, 10);
            assert_eq!(ui.deck.settings.arcade_sound, mode);
            assert!(out.deck_events().any(|e| e
                == Event::Set {
                    key: SettingKey::ArcadeSound,
                    value: mode.into()
                }));
        }
    }
    #[test]
    fn menu_selection_runs_counterclockwise_forward_and_clockwise_back() {
        let mut ui = ui(Game::Flappy);
        let first = ui.arcade.selection;
        ui.apply(Control::Wheel(Wheel::Ccw), 10, &mut Update::new());
        assert_eq!(ui.arcade.selection, (first + 1) % 4);
        ui.apply(Control::Wheel(Wheel::Cw), 20, &mut Update::new());
        assert_eq!(ui.arcade.selection, first);
        press(&mut ui, Button::Right, 30);
        assert_eq!(ui.arcade.selection, (first + 1) % 4);
    }
    #[test]
    fn snake_wheel_and_backup_buttons_match() {
        let mut a = ui(Game::Snake);
        let mut b = ui(Game::Snake);
        begin(&mut a, 0);
        begin(&mut b, 0);
        a.apply(Control::Wheel(Wheel::Cw), 1, &mut Update::new());
        press(&mut b, Button::Right, 1);
        for t in (20..=160).step_by(20) {
            a.tick(t, 0, true, &mut Update::new());
            b.tick(t, 0, true, &mut Update::new());
        }
        assert_eq!(a.arcade.snake.body[0], b.arcade.snake.body[0]);
        assert_eq!(a.arcade.snake.direction, 2);
    }
}
