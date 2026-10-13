//! Local-input inactivity policy, shared by both firmware runtimes.
//!
//! Dark-screen rule: PTT wakes AND acts (the radio must work blind). Every
//! other button or wheel detent that arrives while the screen is Off, or
//! after the panel slept and before its wake frame is on the glass
//! (`awaiting_frame`, judged by the control's stamp), only wakes: the press
//! is consumed together with its matching release, as a wake touch is. A
//! notice may light the screen without counting as input (`notify`), so
//! idle-home keeps the single idle clock (`idle_ms`).
use crate::control::{Button, Control, Edge, TouchPhase};

pub const DIM_AFTER_MS: u32 = 30_000;
pub const OFF_AFTER_MS: u32 = 120_000;
/// A notice keeps the screen lit this long unless real input arrives.
pub const NOTIFY_MS: u32 = 15_000;
pub const BATTERY_INTERVAL_MS: u32 = 15_000;
pub const IDLE_POLL_MS: u32 = 50;
/// Native IRQ latches wake immediately; this is dark-screen recovery only.
pub const DARK_POLL_MS: u32 = 500;
pub const ACTIVE_POLL_MS: u32 = 5;
/// A control stamped up to this long before the wake frame reached the glass
/// was made in the dark, even if the runtime dequeues it afterwards.
const QUEUED_MS: u32 = 1000;
pub const FULL_BRIGHTNESS: u8 = 255;
pub const DIM_BRIGHTNESS: u8 = 32;
/// Panel level for each user brightness setting 1..=5.
const LEVELS: [u8; 5] = [40, 80, 130, 190, FULL_BRIGHTNESS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Screen {
    Active,
    Dim,
    Off,
}
impl Screen {
    pub fn animation_ms(self) -> u32 {
        match self {
            Self::Active => 80,
            Self::Dim => 200,
            Self::Off => u32::MAX,
        }
    }
}

/// Panel brightness for `screen` at the user's level (1..=5). Dim never
/// exceeds the user's lit level.
pub fn brightness(screen: Screen, level: u8) -> u8 {
    let lit = LEVELS[(level.clamp(1, 5) - 1) as usize];
    match screen {
        Screen::Active => lit,
        Screen::Dim => DIM_BRIGHTNESS.min(lit),
        Screen::Off => 0,
    }
}

pub struct Policy {
    screen: Screen,
    last_input: u32,
    buttons: u8,
    contact: bool,
    wake_contact: bool,
    /// Buttons whose press only woke the screen: their release is swallowed too.
    wake_buttons: u8,
    awaiting_frame: bool,
    /// When the last wake frame was shown, while controls stamped before it
    /// may still be queued.
    shown_at: Option<u32>,
    notified: Option<u32>,
}
impl Policy {
    pub const fn new(now: u32) -> Self {
        Self {
            screen: Screen::Active,
            last_input: now,
            buttons: 0,
            contact: false,
            wake_contact: false,
            wake_buttons: 0,
            awaiting_frame: false,
            shown_at: None,
            notified: None,
        }
    }
    pub fn screen(&self) -> Screen {
        self.screen
    }
    pub fn held(&self) -> bool {
        self.buttons != 0 || self.contact
    }
    /// Whether `button` is down, as far as this policy has seen.
    #[cfg_attr(not(feature = "hw-a"), allow(dead_code))]
    pub fn is_down(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }
    /// The UI's single idle clock: 0 while anything is held.
    pub fn idle_ms(&self, now: u32) -> u32 {
        if self.held() {
            0
        } else {
            now.wrapping_sub(self.last_input)
        }
    }
    /// The panel really went to sleep (not merely the clock reaching Off:
    /// a busy panel may never sleep): its wake needs a full frame first.
    pub fn slept(&mut self) {
        self.awaiting_frame = true;
    }
    /// A complete wake frame has been on the glass since `at`.
    pub fn frame_ready(&mut self, at: u32) {
        if core::mem::take(&mut self.awaiting_frame) {
            self.shown_at = Some(at);
        }
    }
    /// Local wake evidence without a complete key/touch command. Keep the
    /// hidden-frame gate intact so waking never clicks an invisible control.
    #[allow(dead_code)] // Raw wake evidence is supplied by the native runtime.
    pub fn wake(&mut self, now: u32) {
        self.activity(now);
    }
    /// A new notice: light the screen for `NOTIFY_MS` without counting as
    /// input, so idle-home and the dim/off clock are untouched.
    pub fn notify(&mut self, now: u32) {
        self.notified = Some(now);
        self.screen = Screen::Active;
    }
    pub fn poll_ms(&self) -> u32 {
        if self.held() {
            ACTIVE_POLL_MS
        } else if self.screen == Screen::Off {
            DARK_POLL_MS
        } else {
            IDLE_POLL_MS
        }
    }
    pub fn tick(&mut self, now: u32) -> Screen {
        if self.held() {
            self.activity(now);
        } else if self
            .notified
            .is_some_and(|at| now.wrapping_sub(at) < NOTIFY_MS)
        {
            self.screen = Screen::Active;
        } else {
            self.notified = None;
            self.screen = match now.wrapping_sub(self.last_input) {
                idle if idle >= OFF_AFTER_MS => Screen::Off,
                idle if idle >= DIM_AFTER_MS => Screen::Dim,
                _ => Screen::Active,
            };
        }
        if self
            .shown_at
            .is_some_and(|at| now.wrapping_sub(at) >= QUEUED_MS)
        {
            self.shown_at = None;
        }
        self.screen
    }
    /// True when the UI should see this control. A dark-screen touch, button
    /// (PTT excepted) or detent is consumed through its release/cancel, even
    /// if the native runtime already woke the screen for the same key.
    /// Host configuration/status traffic never calls this function.
    pub fn input(&mut self, input: Control, now: u32) -> bool {
        let unseen = self
            .shown_at
            .is_some_and(|at| at.wrapping_sub(now).wrapping_sub(1) < QUEUED_MS);
        let dark = self.screen == Screen::Off || self.awaiting_frame || unseen;
        match input {
            Control::Touch(touch) => {
                match touch.phase {
                    TouchPhase::Down => {
                        self.wake_contact = dark;
                        self.contact = true;
                        self.activity(now);
                    }
                    TouchPhase::Move => {
                        if self.contact {
                            self.activity(now);
                        }
                    }
                    TouchPhase::Up | TouchPhase::Cancel => {
                        if self.contact {
                            self.activity(now);
                        }
                        self.contact = false;
                        return !core::mem::replace(&mut self.wake_contact, false);
                    }
                }
                !self.wake_contact
            }
            Control::Button(button, edge) => {
                let mask = button.bit();
                if edge == Edge::Press {
                    self.buttons |= mask;
                    self.activity(now);
                    if dark && button != Button::PushToTalk {
                        self.wake_buttons |= mask;
                        return false;
                    }
                    self.wake_buttons &= !mask;
                    true
                } else {
                    // Repeated synthetic releases on a failed bus do not wake.
                    if self.buttons & mask != 0 {
                        self.activity(now);
                    }
                    self.buttons &= !mask;
                    let swallowed = self.wake_buttons & mask != 0;
                    self.wake_buttons &= !mask;
                    !swallowed
                }
            }
            Control::Wheel(_) => {
                self.activity(now);
                !dark
            }
        }
    }
    fn activity(&mut self, now: u32) {
        self.last_input = now;
        self.screen = Screen::Active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{Button, Touch, Wheel};
    fn touch(phase: TouchPhase) -> Control {
        Control::Touch(Touch {
            phase,
            x: 200,
            y: 300,
        })
    }
    fn press(button: Button) -> Control {
        Control::Button(button, Edge::Press)
    }
    fn release(button: Button) -> Control {
        Control::Button(button, Edge::Release)
    }
    /// Asleep at `OFF_AFTER_MS`, and so is the panel.
    fn dark() -> Policy {
        let mut p = Policy::new(0);
        assert_eq!(p.tick(OFF_AFTER_MS), Screen::Off);
        p.slept();
        p
    }
    #[test]
    fn idle_boundaries_and_clock_wrap() {
        for start in [0u32, u32::MAX - 1000] {
            let mut p = Policy::new(start);
            for (elapsed, expected) in [
                (DIM_AFTER_MS - 1, Screen::Active),
                (DIM_AFTER_MS, Screen::Dim),
                (OFF_AFTER_MS - 1, Screen::Dim),
                (OFF_AFTER_MS, Screen::Off),
            ] {
                assert_eq!(p.tick(start.wrapping_add(elapsed)), expected);
            }
        }
    }
    #[test]
    fn entire_wake_gesture_is_consumed_then_next_tap_works() {
        for end in [TouchPhase::Up, TouchPhase::Cancel] {
            let mut p = dark();
            assert!(!p.input(touch(TouchPhase::Down), OFF_AFTER_MS + 1));
            assert_eq!(p.screen(), Screen::Active);
            assert!(!p.input(touch(TouchPhase::Move), OFF_AFTER_MS + 2));
            assert!(!p.input(touch(end), OFF_AFTER_MS + 3));
            assert!(!p.held());
            p.frame_ready(OFF_AFTER_MS + 4);
            assert!(p.input(touch(TouchPhase::Down), OFF_AFTER_MS + 4));
            assert!(p.input(touch(TouchPhase::Up), OFF_AFTER_MS + 5));
        }
    }
    #[test]
    fn dim_touch_acts_and_restores_brightness() {
        let mut p = Policy::new(0);
        p.tick(DIM_AFTER_MS);
        assert!(p.input(touch(TouchPhase::Down), DIM_AFTER_MS + 1));
        assert_eq!(p.screen(), Screen::Active);
    }
    #[test]
    fn second_contact_during_panel_warmup_is_also_consumed() {
        let mut p = dark();
        p.input(touch(TouchPhase::Down), OFF_AFTER_MS + 1);
        p.input(touch(TouchPhase::Up), OFF_AFTER_MS + 2);
        assert!(!p.input(touch(TouchPhase::Down), OFF_AFTER_MS + 3));
        p.frame_ready(OFF_AFTER_MS + 4);
        assert!(!p.input(touch(TouchPhase::Up), OFF_AFTER_MS + 4));
        assert!(p.input(touch(TouchPhase::Down), OFF_AFTER_MS + 5));
    }
    #[test]
    fn dark_ptt_acts() {
        let mut p = dark();
        assert!(p.input(press(Button::PushToTalk), OFF_AFTER_MS));
        assert_eq!(p.screen(), Screen::Active);
        assert!(p.held());
        assert_eq!(p.tick(OFF_AFTER_MS * 3), Screen::Active);
        assert_eq!(p.poll_ms(), ACTIVE_POLL_MS);
        // Still awaiting the wake frame: PTT is exempt, the release included.
        assert!(p.input(release(Button::PushToTalk), OFF_AFTER_MS * 3));
        assert_eq!(p.tick(OFF_AFTER_MS * 3 + DIM_AFTER_MS), Screen::Dim);
    }
    #[test]
    fn dark_enter_consumed_through_release_then_next_press_acts() {
        for button in Button::ALL {
            if button == Button::PushToTalk {
                continue;
            }
            let mut p = dark();
            assert!(!p.input(press(button), OFF_AFTER_MS + 1));
            assert_eq!(p.screen(), Screen::Active);
            assert!(p.held());
            // The wake frame is on the glass before the finger lifts: the
            // release still belongs to the wake press.
            p.frame_ready(OFF_AFTER_MS + 100);
            assert!(!p.input(release(button), OFF_AFTER_MS + 200));
            assert!(!p.held());
            assert!(p.input(press(button), OFF_AFTER_MS + 300));
            assert!(p.input(release(button), OFF_AFTER_MS + 400));
        }
    }
    #[test]
    fn dark_press_before_the_wake_frame_is_consumed_too() {
        let mut p = dark();
        assert!(!p.input(press(Button::Enter), OFF_AFTER_MS + 1));
        assert!(!p.input(release(Button::Enter), OFF_AFTER_MS + 2));
        // Panel still resetting: a second press is swallowed like the first.
        assert!(!p.input(press(Button::Cancel), OFF_AFTER_MS + 3));
        p.frame_ready(OFF_AFTER_MS + 4);
        assert!(!p.input(release(Button::Cancel), OFF_AFTER_MS + 4));
        assert!(p.input(press(Button::Cancel), OFF_AFTER_MS + 5));
    }
    #[test]
    fn a_press_stamped_before_the_wake_frame_is_dark_though_dequeued_after() {
        for start in [0u32, u32::MAX - OFF_AFTER_MS - 100] {
            let mut p = Policy::new(start);
            p.tick(start.wrapping_add(OFF_AFTER_MS));
            p.slept();
            let at = |ms: u32| start.wrapping_add(OFF_AFTER_MS + ms);
            assert!(!p.input(press(Button::Enter), at(10)));
            assert!(!p.input(release(Button::Enter), at(20)));
            // Embassy: pressed during the wake render, taken off the queue
            // only once the frame is on the glass.
            p.frame_ready(at(260));
            assert!(!p.input(press(Button::Enter), at(180)));
            assert!(!p.input(release(Button::Enter), at(200)));
            assert!(p.input(press(Button::Enter), at(260)));
            assert!(p.input(release(Button::Enter), at(300)));
            // The window closes: an old stamp no longer means dark.
            p.tick(at(260 + QUEUED_MS));
            assert!(p.input(press(Button::Enter), at(100)));
        }
        // A frame shown without a sleep before it changes nothing.
        let mut p = Policy::new(0);
        p.frame_ready(500);
        assert!(p.input(press(Button::Enter), 400));
    }
    #[test]
    fn the_clock_reaching_off_without_a_panel_sleep_leaves_input_live() {
        // A panel busy with a frame at the Off tick never slept, so no wake
        // frame will come: the waking press is the only one swallowed.
        let mut p = Policy::new(0);
        assert_eq!(p.tick(OFF_AFTER_MS), Screen::Off);
        assert!(!p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 5));
        assert_eq!(p.tick(OFF_AFTER_MS + 10), Screen::Active);
        assert!(p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 10_000));
        assert!(p.input(press(Button::Enter), OFF_AFTER_MS + 20_000));
        assert!(p.input(release(Button::Enter), OFF_AFTER_MS + 20_030));
    }
    #[test]
    fn dark_shift_tap_does_not_count_toward_double_shift() {
        // Neither edge reaches the UI, so the chord detector never sees it.
        let mut p = dark();
        assert!(!p.input(press(Button::Shift), OFF_AFTER_MS + 1));
        assert!(!p.input(release(Button::Shift), OFF_AFTER_MS + 100));
        p.frame_ready(OFF_AFTER_MS + 150);
        assert!(p.input(press(Button::Shift), OFF_AFTER_MS + 200));
        assert!(p.input(release(Button::Shift), OFF_AFTER_MS + 300));
    }
    #[test]
    fn dark_wheel_consumed_until_frame_ready() {
        let mut p = dark();
        assert!(!p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 1));
        assert_eq!(p.screen(), Screen::Active);
        assert!(!p.input(Control::Wheel(Wheel::Ccw), OFF_AFTER_MS + 50));
        p.frame_ready(OFF_AFTER_MS + 100);
        assert!(p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 100));
        // Dim acts normally.
        p.tick(OFF_AFTER_MS + 100 + DIM_AFTER_MS);
        assert_eq!(p.screen(), Screen::Dim);
        assert!(p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 100 + DIM_AFTER_MS));
        assert_eq!(p.screen(), Screen::Active);
    }
    #[test]
    fn a_wake_press_whose_release_was_lost_is_cleared_by_the_next_press() {
        let mut p = dark();
        assert!(!p.input(press(Button::Enter), OFF_AFTER_MS + 1));
        p.frame_ready(OFF_AFTER_MS + 2);
        // SHIFT self-heal style: a press while already down is a fresh press.
        assert!(p.input(press(Button::Enter), OFF_AFTER_MS + 500));
        assert!(p.input(release(Button::Enter), OFF_AFTER_MS + 600));
    }
    #[test]
    fn notify_lights_the_screen_for_fifteen_seconds_without_counting_as_input() {
        let mut p = dark();
        let at = OFF_AFTER_MS + 5000;
        p.notify(at);
        assert_eq!(p.screen(), Screen::Active);
        assert_eq!(p.idle_ms(at), at);
        assert_eq!(p.tick(at + NOTIFY_MS - 1), Screen::Active);
        assert_eq!(p.tick(at + NOTIFY_MS), Screen::Off);
        assert_eq!(p.poll_ms(), DARK_POLL_MS);
        // The wake frame gate holds: a blind ENTER cannot answer the card.
        let mut p = dark();
        p.notify(at);
        assert!(!p.input(press(Button::Enter), at + 100));
        assert!(!p.input(release(Button::Enter), at + 150));
        // From Dim it goes straight to Active and the dim clock is untouched.
        let mut p = Policy::new(0);
        assert_eq!(p.tick(DIM_AFTER_MS), Screen::Dim);
        p.notify(DIM_AFTER_MS);
        assert_eq!(p.tick(DIM_AFTER_MS + NOTIFY_MS - 1), Screen::Active);
        assert_eq!(p.tick(DIM_AFTER_MS + NOTIFY_MS), Screen::Dim);
        assert_eq!(p.idle_ms(DIM_AFTER_MS + NOTIFY_MS), DIM_AFTER_MS + NOTIFY_MS);
        assert_eq!(p.tick(OFF_AFTER_MS), Screen::Off);
    }
    #[test]
    fn real_input_during_a_notify_window_restarts_the_idle_clocks() {
        let mut p = dark();
        p.notify(OFF_AFTER_MS);
        p.frame_ready(OFF_AFTER_MS + 500);
        assert!(p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 1000));
        assert_eq!(p.idle_ms(OFF_AFTER_MS + 1500), 500);
        assert_eq!(p.tick(OFF_AFTER_MS + 1000 + DIM_AFTER_MS - 1), Screen::Active);
        assert_eq!(p.tick(OFF_AFTER_MS + 1000 + DIM_AFTER_MS), Screen::Dim);
    }
    #[test]
    fn idle_ms_reads_zero_while_held_and_counts_from_the_last_input() {
        let mut p = Policy::new(100);
        assert_eq!(p.idle_ms(100), 0);
        assert_eq!(p.idle_ms(5100), 5000);
        p.input(press(Button::Left), 6000);
        assert_eq!(p.idle_ms(90_000), 0);
        p.tick(90_000);
        p.input(release(Button::Left), 90_000);
        assert_eq!(p.idle_ms(90_010), 10);
        p.input(touch(TouchPhase::Down), 91_000);
        assert_eq!(p.idle_ms(200_000), 0);
        p.input(touch(TouchPhase::Cancel), 200_000);
        assert_eq!(p.idle_ms(200_000 + OFF_AFTER_MS), OFF_AFTER_MS);
        // Clock wrap.
        let p = Policy::new(u32::MAX - 5);
        assert_eq!(p.idle_ms(4), 10);
    }
    #[test]
    fn brightness_scales_with_the_user_level_and_dim_stays_below_it() {
        assert_eq!(brightness(Screen::Active, 5), FULL_BRIGHTNESS);
        assert_eq!(brightness(Screen::Active, 0), brightness(Screen::Active, 1));
        assert_eq!(brightness(Screen::Active, 9), FULL_BRIGHTNESS);
        for level in 1..5 {
            assert!(brightness(Screen::Active, level) < brightness(Screen::Active, level + 1));
            assert!(brightness(Screen::Dim, level) < brightness(Screen::Active, level));
            assert_eq!(brightness(Screen::Off, level), 0);
        }
        assert_eq!(brightness(Screen::Dim, 5), DIM_BRIGHTNESS);
    }
    #[test]
    fn each_of_the_six_buttons_wakes_holds_and_releases_independently() {
        for button in Button::ALL {
            let mut p = dark();
            let delivered = button == Button::PushToTalk;
            assert_eq!(p.input(press(button), OFF_AFTER_MS), delivered);
            assert_eq!(p.screen(), Screen::Active);
            assert!(p.held());
            // Another button's release must not drop this hold.
            for other in Button::ALL {
                if other != button {
                    assert!(p.input(release(other), OFF_AFTER_MS));
                }
            }
            assert!(p.held());
            assert_eq!(p.tick(OFF_AFTER_MS * 3), Screen::Active);
            assert_eq!(p.input(release(button), OFF_AFTER_MS * 3), delivered);
            assert!(!p.held());
        }
        let mut p = Policy::new(0);
        for button in Button::ALL {
            assert!(p.input(press(button), 1));
        }
        for button in Button::ALL {
            assert!(p.held());
            assert!(p.input(release(button), 2));
        }
        assert!(!p.held());
    }
    #[test]
    fn synthetic_releases_do_not_keep_device_awake() {
        let mut p = Policy::new(0);
        for now in [1000, 10_000, 30_000, 100_000] {
            // A failed keypad bus releases every button on each poll.
            for button in Button::ALL {
                assert!(p.input(release(button), now));
            }
            p.input(touch(TouchPhase::Cancel), now);
        }
        assert_eq!(p.tick(OFF_AFTER_MS), Screen::Off);
        assert_eq!(p.poll_ms(), DARK_POLL_MS);
    }
    #[test]
    fn physical_wake_does_not_unmask_touch_release() {
        let mut p = dark();
        p.input(touch(TouchPhase::Down), OFF_AFTER_MS);
        assert!(!p.input(Control::Wheel(Wheel::Cw), OFF_AFTER_MS + 1));
        assert!(!p.input(touch(TouchPhase::Up), OFF_AFTER_MS + 2));
    }
    #[test]
    fn held_touch_and_cancel_restart_idle_timer() {
        let mut p = Policy::new(0);
        p.input(touch(TouchPhase::Down), 10);
        assert_eq!(p.tick(OFF_AFTER_MS), Screen::Active);
        p.input(touch(TouchPhase::Cancel), OFF_AFTER_MS);
        assert_eq!(p.tick(OFF_AFTER_MS + DIM_AFTER_MS - 1), Screen::Active);
        assert_eq!(p.tick(OFF_AFTER_MS + DIM_AFTER_MS), Screen::Dim);
    }
}
