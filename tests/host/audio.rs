//! The voice link's pure parts, under the firmware's own `audio::` names
//! (`policy` and `upsample` reach `codec` as their sibling).
#[path = "../../src/audio/codec.rs"]
pub mod codec;
#[path = "../../src/audio/policy.rs"]
pub mod policy;
#[path = "../../src/audio/upsample.rs"]
pub mod upsample;
