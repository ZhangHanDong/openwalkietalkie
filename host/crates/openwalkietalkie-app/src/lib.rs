//! `OpenWalkieTalkie.app` — the GUI (design §8).
//!
//! A pure client. It opens no HID link, no audio device and no hotkey, and it
//! writes no third-party config file: `deckd` owns all of that and this app
//! talks to it over one UDS session. What is here is the window.
//!
//! The split (§8.1, contract §4): everything testable is in
//! [`deck_gui_core`] — the event bus, the tokio bridge, the `ApiClient`, the
//! `HostState` mirror with its reducer, the admin and voice reply shapes and
//! the tray's aggregate — and this crate is the `gpui` layer on top:
//! [`present`] turns those into strings and tones, [`shell`] is the window
//! that owns the one mirror and pumps the bus, [`deck`], [`integrations`],
//! [`voice`], [`service`] and [`welcome`] are the panes §8.2 ships in v1,
//! [`tray`] is the status item, [`login_item`] is `SMAppService.mainApp`,
//! and [`app`] boots it.
//!
//! The one threading rule, which the shape keeps rather than the discipline:
//! **nothing calls `block_on` on the gpui thread.** `deck-gui-core` hands out
//! a spawn and never a runtime `Handle`, so I/O goes to
//! [`deck_gui_core::TokioBridge::spawn`] and anything awaiting a reply goes
//! to `cx.spawn`, which runs a plain future on gpui's own foreground
//! executor.
pub mod app;
pub mod assets;
pub mod confirm;
pub mod deck;
pub mod integrations;
pub mod login_item;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod present;
pub mod service;
pub mod shell;
pub mod shots;
#[cfg(target_os = "macos")]
pub mod snapshot;
pub mod style;
pub mod tray;
pub mod updater;
pub mod ui;
pub mod voice;
pub mod welcome;
pub mod workflow;

pub use app::run;

pub mod updates;
