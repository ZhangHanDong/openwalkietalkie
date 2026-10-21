/// Consecutive counterclockwise detents needed to unlock a sleeping deck.
pub const UNLOCK_TICKS: u8 = 4;

/// One detent of the scroll wheel.
#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Wheel {
    /// Physical clockwise rotation.
    Cw,
    /// Physical counterclockwise rotation.
    Ccw,
}

/// The six physical buttons.
#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Button {
    Enter,
    Cancel,
    /// Pure modifier: never acts alone.
    Shift,
    /// Held, not toggled: audio is live for exactly as long as it is down.
    PushToTalk,
    Left,
    Right,
}
impl Button {
    pub const ALL: [Button; 6] = [
        Button::Enter,
        Button::Cancel,
        Button::Shift,
        Button::PushToTalk,
        Button::Left,
        Button::Right,
    ];
    /// One bit per button, `1 << n` in declaration order, for held masks.
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Edge {
    Press,
    Release,
}

#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Control {
    Wheel(Wheel),
    Button(Button, Edge),
    Touch(Touch),
}

/// Screen contact phases; Cancel releases a gesture without activating it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Touch {
    pub phase: TouchPhase,
    pub x: u16,
    pub y: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_follow_declaration_order_and_cover_all_six() {
        let expected = [
            (Button::Enter, 1),
            (Button::Cancel, 2),
            (Button::Shift, 4),
            (Button::PushToTalk, 8),
            (Button::Left, 16),
            (Button::Right, 32),
        ];
        assert_eq!(Button::ALL.len(), expected.len());
        for (index, (button, bit)) in expected.into_iter().enumerate() {
            assert_eq!(Button::ALL[index], button);
            assert_eq!(button.bit(), bit);
        }
        assert_eq!(
            Button::ALL.iter().fold(0, |mask, b| mask | b.bit()),
            0b11_1111
        );
    }
}
