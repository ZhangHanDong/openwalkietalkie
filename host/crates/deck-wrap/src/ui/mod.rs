//! The firmware's text stack, mounted read-only: the glyph atlas, the font
//! specs, the measuring/wrapping code and the lane geometry. `font.rs` names
//! `super::font_data`, `super::theme` and `crate::display::Band`; `theme.rs`
//! names `crate::deck::msg`; `layout.rs` names `super::theme`. Mounting the
//! four together under these names is what lets them build unmodified.

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/ui/font_data.rs"]
pub mod font_data;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/ui/theme.rs"]
pub mod theme;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/ui/font.rs"]
pub mod font;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/ui/layout.rs"]
pub mod layout;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/ui/rich.rs"]
pub mod rich;
