//! XL9555 16-bit I2C GPIO expander.
//!
//! On the T-SF32-Keyboard this chip is the gatekeeper: the TCA8418's reset line
//! hangs off P0_7, so until the expander is configured and that pin driven
//! high, the keypad does not acknowledge its own address and a bus scan shows
//! nothing but the expander itself.
//!
//! Registers auto-increment across the two ports, so a single 3-byte write
//! (`reg`, port0, port1) covers all 16 pins at once -- which is how LilyGo's
//! driver does it and why `write16` takes a `u16`.

// The full pin map is recorded even though this firmware only drives three of
// the lines: the next thing to need the GPS or haptic rails should not have to
// go back to LilyGo's SDK to find out where they live.
#![allow(dead_code)]

use embedded_hal::i2c::I2c;

/// A0..A2 are strapped to 0b010 on this board.
pub const ADDR: u8 = 0x22;

const OUTPUT_PORT_0: u8 = 0x02;
/// Direction: 0 = output, 1 = input. Power-on default is all-input.
const CONFIG_0: u8 = 0x06;

// Pin assignments from LilyGo's SDK:
// `sifli-sdk/t_sf32_display_hw_device/io_expand/xl9555/inc/xl9555.h`.
pub const GPS_EN: u8 = 0;
pub const KEY_LED_EN: u8 = 1;
pub const WIFI_EN: u8 = 2;
pub const AW86224_RST: u8 = 3;
pub const GPS_RST: u8 = 4;
pub const WIFI_RST: u8 = 5;
pub const GPS_ESP32C6_SEL: u8 = 6;
/// TCA8418 reset, active low.
pub const KEY_RST: u8 = 7;

pub struct Xl9555<I2C> {
    i2c: I2C,
    addr: u8,
    /// Shadow of the output register: the chip is write-only in practice here,
    /// and read-modify-write over I2C for every pin change would be wasteful.
    outputs: u16,
}

impl<I2C: I2c> Xl9555<I2C> {
    pub fn new(i2c: I2C, addr: u8) -> Self {
        Self {
            i2c,
            addr,
            outputs: 0,
        }
    }

    /// Drive every pin low, then switch all 16 from input to output.
    ///
    /// Order matters: setting the output register first means the pins go to a
    /// defined low rather than briefly floating at whatever the last state was.
    pub fn init(&mut self) -> Result<(), I2C::Error> {
        self.outputs = 0;
        self.write16(OUTPUT_PORT_0, 0)?;
        self.write16(CONFIG_0, 0)
    }

    pub fn set(&mut self, pin: u8, high: bool) -> Result<(), I2C::Error> {
        debug_assert!(pin < 16, "XL9555 has 16 pins");
        let mask = 1u16 << pin;
        if high {
            self.outputs |= mask;
        } else {
            self.outputs &= !mask;
        }
        let outputs = self.outputs;
        self.write16(OUTPUT_PORT_0, outputs)
    }

    fn write16(&mut self, reg: u8, value: u16) -> Result<(), I2C::Error> {
        self.i2c
            .write(self.addr, &[reg, value as u8, (value >> 8) as u8])
    }
}
