//! Agent Command Deck model and channel-5 host protocol. Like `ui`, this one
//! root is mounted as `deck` by every consumer, and may only name the shared
//! crate paths `battery`, `control`, `display`, `orientation`, `deck` and `ui`.
//!
//! `text` and `msg` are the shared vocabulary; `model` gives host messages
//! meaning, `json`/`proto` carry them over HID channel 5, and `demo` stands in
//! for a host until one says hello.

pub mod demo;
pub mod json;
pub mod model;
pub mod msg;
pub mod proto;
pub mod text;

// Not every consumer names every re-export.
#[allow(unused_imports)]
pub use model::{Agent, Card, Deck, Effects, MacroSet, Notice, Settings, Usage, Workspaces};
#[allow(unused_imports)]
pub use msg::*;
#[allow(unused_imports)]
pub use text::Text;
