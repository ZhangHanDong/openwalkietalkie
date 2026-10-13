//! The device link on the hid thread (design §6): desired state, outbox,
//! session state machine, framing.
pub mod framer;
pub mod outbox;
pub mod session;
pub mod view;
pub mod writer;
