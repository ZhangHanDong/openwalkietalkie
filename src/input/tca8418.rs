//! TI TCA8418 I2C keypad scanner.
//!
//! The chip debounces and scans the matrix itself and reports presses and
//! releases through a 10-deep event FIFO, so the host never scans rows. Board
//! code uses the PA11 interrupt plus fallback polling to drain this FIFO, then
//! acknowledges its interrupt. The protocol driver stays pin-independent.

// `pending` and the GPI decoding are part of the chip's contract and are kept
// available even though the current poll loop does not need them.
#![allow(dead_code)]

use embedded_hal::i2c::I2c;

pub const ADDR: u8 = 0x34;

const CFG: u8 = 0x01;
const INT_STAT: u8 = 0x02;
const KEY_LCK_EC: u8 = 0x03;
const KEY_EVENT_A: u8 = 0x04;
const GPIO_INT_EN1: u8 = 0x1A;
const GPIO_INT_EN2: u8 = 0x1B;
const GPIO_INT_EN3: u8 = 0x1C;
const KP_GPIO1: u8 = 0x1D;
const KP_GPIO2: u8 = 0x1E;
const KP_GPIO3: u8 = 0x1F;

/// CFG[0], key-event interrupt enable. Also what makes the FIFO fill.
const CFG_KE_IEN: u8 = 0x01;
const EVENT_COUNT_MASK: u8 = 0x0F;
/// INT_STAT bits are write-1-to-clear.
const INT_STAT_ALL: u8 = 0x1F;

#[derive(Debug, defmt::Format)]
pub enum Error<E> {
    I2c(E),
    /// The chip acknowledged but did not read back the configuration we wrote,
    /// which in practice means it is still held in reset.
    Unresponsive,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, defmt::Format)]
pub struct KeyEvent {
    /// Key number as the chip reports it, 1..=80 for matrix keys.
    pub key: u8,
    pub pressed: bool,
}

impl KeyEvent {
    /// Matrix keys are numbered in *decimal* as `row * 10 + col + 1`, so the
    /// tens digit is the row and the ones digit the column (datasheet Table 1).
    /// The chip supports 10 columns even where, as here, only 4 are wired.
    ///
    /// Returns `None` for GPI events (97..=114), which are not matrix presses.
    pub fn position(&self) -> Option<(u8, u8)> {
        match self.key {
            1..=80 => Some(((self.key - 1) / 10, (self.key - 1) % 10)),
            _ => None,
        }
    }
}

pub struct Tca8418<I2C> {
    i2c: I2C,
    addr: u8,
}

impl<I2C: I2c> Tca8418<I2C> {
    pub fn new(i2c: I2C, addr: u8) -> Self {
        Self { i2c, addr }
    }

    /// Claim the first `rows` rows and `cols` columns for the key matrix and
    /// start filling the event FIFO. The T-SF32-Keyboard is 5x4.
    pub fn configure(&mut self, rows: u8, cols: u8) -> Result<(), Error<I2C::Error>> {
        debug_assert!(rows <= 8 && cols <= 10);
        let row_mask = low_bits(rows) as u8;
        let col_mask = low_bits(cols);

        // Rows and columns not listed here stay GPIO and are simply ignored.
        self.write(KP_GPIO1, row_mask).map_err(Error::I2c)?;
        self.write(KP_GPIO2, col_mask as u8).map_err(Error::I2c)?;
        self.write(KP_GPIO3, (col_mask >> 8) as u8)
            .map_err(Error::I2c)?;

        // Same masks again: only the lines that are part of the matrix should
        // be allowed to raise an event.
        self.write(GPIO_INT_EN1, row_mask).map_err(Error::I2c)?;
        self.write(GPIO_INT_EN2, col_mask as u8)
            .map_err(Error::I2c)?;
        self.write(GPIO_INT_EN3, (col_mask >> 8) as u8)
            .map_err(Error::I2c)?;

        self.write(CFG, CFG_KE_IEN).map_err(Error::I2c)?;
        if self.read(CFG).map_err(Error::I2c)? != CFG_KE_IEN {
            return Err(Error::Unresponsive);
        }
        self.flush().map_err(Error::I2c)?;
        Ok(())
    }

    /// Number of events waiting in the FIFO.
    pub fn pending(&mut self) -> Result<u8, I2C::Error> {
        Ok(self.read(KEY_LCK_EC)? & EVENT_COUNT_MASK)
    }

    /// Pop one event. `None` means the FIFO is empty.
    pub fn next_event(&mut self) -> Result<Option<KeyEvent>, I2C::Error> {
        let raw = self.read(KEY_EVENT_A)?;
        if raw == 0 {
            return Ok(None);
        }
        Ok(Some(KeyEvent {
            key: raw & 0x7F,
            pressed: raw & 0x80 != 0,
        }))
    }

    /// Drop anything queued -- keys mashed during power-up, mostly.
    pub fn flush(&mut self) -> Result<(), I2C::Error> {
        while self.next_event()?.is_some() {}
        self.acknowledge()
    }

    /// Clear the latched interrupt after draining the FIFO. A periodic
    /// fallback covers an event arriving during this write-one-to-clear.
    pub fn acknowledge(&mut self) -> Result<(), I2C::Error> {
        self.write(INT_STAT, INT_STAT_ALL)
    }

    fn read(&mut self, reg: u8) -> Result<u8, I2C::Error> {
        let mut buf = [0u8; 1];
        self.i2c.write_read(self.addr, &[reg], &mut buf)?;
        Ok(buf[0])
    }

    fn write(&mut self, reg: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.addr, &[reg, value])
    }
}

/// `low_bits(3) == 0b111`. Guards against the `1 << 16` overflow at n == 16.
fn low_bits(n: u8) -> u16 {
    if n >= 16 {
        u16::MAX
    } else {
        (1u16 << n) - 1
    }
}
