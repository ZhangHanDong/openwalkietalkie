//! Everything drawn over the strip, in precedence order: the notice indicator
//! and toast, the drawer, the manager family, the factory-reset dialog, the
//! global PTT overlay, and above everything the power-off hold dialog. The
//! screen lock covers everything. Otherwise the tutorial stands alone:
//! nothing else is drawn while it is up (no toast,
//! no PTT session exists), only the power-off dialog over it.
//!
//!   pub fn draw(band: &mut Band, ui: &UiState)
//!   pub fn signature(ui: &UiState, sig: &mut SigBuilder)
//!
//! Overlays draw at origin 0 with the damage clip; they never slide.
use crate::display::Band;
use crate::ui::signature::SigBuilder;
use crate::ui::state::{Overlay, UiState};

pub mod device;
pub mod macroedit;
pub mod manager;
pub mod notify;
pub mod power;
pub mod ptt;
pub mod reset;
pub mod screen_lock;
pub mod tutorial;

/// The LISTENING overlay is up: a PTT session exists.
pub fn ptt_visible(ui: &UiState) -> bool {
    ui.global.ptt.is_some() || ui.global.ptt_pending.is_some()
}

/// Nothing of the strip shows through: the drawer, the manager, the gallery,
/// naming, the tutorial, screen lock and LISTENING overlay each START with
/// `band.fill(CANVAS)` and own every pixel, so `scene` neither draws nor
/// signs the faces beneath them (a hidden face costs no render time and
/// causes no damage). The two confirm dialogs and the toast/indicator are
/// not opaque.
pub fn opaque(ui: &UiState) -> bool {
    ui.screen_lock.is_some() || ptt_visible(ui)
        || matches!(
            ui.global.overlay,
            Overlay::Drawer
                | Overlay::Manager
                | Overlay::Gallery
                | Overlay::Naming
                | Overlay::Tutorial
                | Overlay::MacroEdit
        )
}

pub fn draw(band: &mut Band, ui: &UiState) {
    if ui.screen_lock.is_some() {
        screen_lock::draw(band, ui);
        return;
    }
    if ui.global.overlay == Overlay::Tutorial {
        tutorial::draw(band, ui);
        power::draw(band, ui);
        return;
    }
    notify::draw(band, ui);
    match ui.global.overlay {
        Overlay::None | Overlay::Tutorial => {}
        Overlay::Drawer => notify::draw_drawer(band, ui),
        Overlay::Manager | Overlay::Gallery | Overlay::Naming | Overlay::ConfirmRemove => {
            manager::draw(band, ui)
        }
        Overlay::ConfirmReset => reset::draw(band, ui),
        Overlay::MacroEdit => macroedit::draw(band, ui),
    }
    if ptt_visible(ui) {
        ptt::draw(band, ui);
    }
    power::draw(band, ui);
}

pub fn signature(ui: &UiState, sig: &mut SigBuilder) {
    sig.mode(ui.screen_lock.is_some() as u32);
    if ui.screen_lock.is_some() {
        screen_lock::signature(ui, sig);
        return;
    }
    sig.mode(
        ui.global.overlay as u32
            | (ptt_visible(ui) as u32) << 8
            | (ui.power_hold().is_some() as u32) << 10,
    );
    if ui.global.overlay == Overlay::Tutorial {
        tutorial::signature(ui, sig);
        power::signature(ui, sig);
        return;
    }
    notify::signature(ui, sig);
    match ui.global.overlay {
        Overlay::None | Overlay::Tutorial => {}
        Overlay::Drawer => notify::signature_drawer(ui, sig),
        Overlay::Manager | Overlay::Gallery | Overlay::Naming | Overlay::ConfirmRemove => {
            manager::signature(ui, sig)
        }
        Overlay::ConfirmReset => reset::signature(ui, sig),
        Overlay::MacroEdit => macroedit::signature(ui, sig),
    }
    if ptt_visible(ui) {
        ptt::signature(ui, sig);
    }
    power::signature(ui, sig);
}
