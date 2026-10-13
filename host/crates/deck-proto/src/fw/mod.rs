//! The firmware's deck module, mounted read-only from `src/deck` for the
//! contract tests (design §14 "Wire contract"): its `LineAssembler`, `parse`
//! and `Event` encoder are the other end of the wire, so every line this crate
//! emits is run through them and every line the device emits comes from them.
//! `proto.rs` names `super::model::MAX_AGENTS` and `crate::ui::UiState` for
//! its encoder; `model.rs` cannot be mounted (it loads the demo script, which
//! needs the whole UI), so `model` and `ui` are shims reduced to what the
//! encoder and its tests read (`lib.rs` re-exports `ui` as `crate::ui`).
//!
//! The files keep the firmware tree's formatting and lint profile, hence the
//! skips; `msg.rs`'s version test holds because the workspace version equals
//! `FW_VERSION`.

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/text.rs"]
pub mod text;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/msg.rs"]
pub mod msg;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/json.rs"]
pub mod json;

#[allow(warnings, clippy::all)]
#[rustfmt::skip]
#[path = "../../../../../src/deck/proto.rs"]
pub mod proto;

pub mod model;
pub mod ui;
