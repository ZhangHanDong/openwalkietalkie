//! The firmware's message vocabulary, mounted because `ui/theme.rs` names
//! `crate::deck::msg::{AgentState, Tone}` absolutely. `msg.rs` in turn
//! names `super::text`, so its fixed-capacity string comes along.

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/text.rs"]
pub mod text;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/msg.rs"]
pub mod msg;
