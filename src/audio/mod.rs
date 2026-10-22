//! The deck's voice: its microphone and speaker, carried over the BLE audio
//! service as IMA ADPCM frames (`codec`, shared with deckd). The deck is a
//! walkie-talkie, not a Bluetooth audio device: a PTT press that a deck host
//! hears starts a microphone stream, and the host's replies arrive as
//! speaker streams. `voice` is the board-independent glue between the radio
//! and the audio tasks; the drivers exist on Rev A only.
// Shared with deckd and the host tests, which use the parts this image does not.
#[allow(dead_code)]
pub mod codec;
pub mod arcade;
#[cfg_attr(not(feature = "hw-a"), allow(dead_code))]
pub mod policy;
#[cfg_attr(not(feature = "hw-a"), allow(dead_code))]
pub mod upsample;
#[cfg_attr(not(feature = "hw-a"), allow(dead_code))]
pub mod voice;
#[cfg(feature = "audio-usb-rate-test")]
pub mod usb_test;

#[cfg(feature = "hw-a")]
pub mod mic;
#[cfg(feature = "hw-a")]
pub mod pdm;
#[cfg(feature = "hw-a")]
pub mod speaker;
