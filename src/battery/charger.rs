//! SGM41562S charger (device ID 0x09), as fitted to this T-Display SF32.
//!
//! Register definitions: SGM41562S datasheet, July 2026, pp. 20–25.
//! https://www.sg-micro.com/rect/assets/213b1edb-e9a1-4f49-9b20-6f1330d7c3ce/SGM41562S.PDF
//! Match LilyGo's menu firmware: 350mA target (344mA in 8mA steps), 4.2V.
//! Its older SGM41562B register encodings must NOT be used for this S variant:
//! REG04 has 10mV steps, and WD_RST is REG0C bit 0 (not REG02 bit 6).
//! https://github.com/Xinyuan-LilyGO/SlFli-SDK-Lilygo/blob/80561b83b03c6293f9ad4cb5fc056ae75b01d5b3/sifli-sdk/t_sf32_display_hw_device/charge/sgm41562b/sgm41562b.c#L852

use embedded_hal::i2c::I2c;

pub const ADDRESS: u8 = 0x03;
pub const REG_VOLTAGE: u8 = 0x04;
pub const REG_STATUS: u8 = 0x08;
pub const REG_FAULT: u8 = 0x09;
const REG_POWER: u8 = 0x01;
const REG_ID: u8 = 0x0b;
const REG_INPUT_WATCHDOG: u8 = 0x0c;
const CEB: u8 = 0x08;

#[derive(Debug, PartialEq)]
pub enum Error<E> {
    Bus(E),
    UnsupportedDevice(u8),
    Verify {
        register: u8,
        expected: u8,
        actual: u8,
    },
}

pub fn read_reg<I: I2c>(bus: &mut I, register: u8) -> Result<u8, I::Error> {
    let mut value = [0];
    bus.write_read(ADDRESS, &[register], &mut value)?;
    Ok(value[0])
}

/// Configure with charging disabled, verify each setting, then enable last.
/// Unknown device IDs are never written. A failed setup attempts to leave
/// charging disabled; the application can continue using USB or battery power.
pub fn initialize<I: I2c>(bus: &mut I) -> Result<(), Error<I::Error>> {
    let id = read_reg(bus, REG_ID).map_err(Error::Bus)?;
    if id != 0x09 {
        return Err(Error::UnsupportedDevice(id));
    }
    let result = configure(bus);
    if result.is_err() {
        let _ = update_verified(bus, REG_POWER, CEB, CEB);
    }
    result
}

fn configure<I: I2c>(bus: &mut I) -> Result<(), Error<I::Error>> {
    update_verified(bus, REG_POWER, CEB, CEB)?;
    // (register, mask, value). Preserve unrelated power/reset settings.
    for (reg, mask, value) in [
        (0x00, 0xf0, 0x90), // Input voltage regulation: 4.6V.
        (0x02, 0xff, 0x2a), // 350mA target: 8 + 8*42 = 344mA; REG_RST stays clear.
        (0x03, 0x0f, 0x0f), // Pre-charge / termination current: 31mA.
        (0x04, 0xfe, 0x8c), // 3500 + 10*70 = 4200mV.
        (0x05, 0xff, 0x3c), // 64s watchdog; termination and 8h safety timer.
        (0x06, 0xe0, 0xc0), // NTC and extended safety timing on, BATFET on.
        (0x07, 0x80, 0x00), // Input voltage regulation enabled.
        (0x08, 0x60, 0x00), // PCB over-temperature protection, 6V input OVP.
        (0x0a, 0x1b, 0x00), // Normal power path/current scale, input OVP on.
        (0x0c, 0xfa, 0x7a), // Input limit 500mA; pre-charge threshold 3.0V.
    ] {
        update_verified(bus, reg, mask, value)?;
    }
    feed_watchdog(bus).map_err(Error::Bus)?;
    // Both the charge path and USB input path can now be enabled.
    update_verified(bus, REG_POWER, CEB | 0x10, 0)
}

fn update_verified<I: I2c>(
    bus: &mut I,
    register: u8,
    mask: u8,
    value: u8,
) -> Result<(), Error<I::Error>> {
    let previous = read_reg(bus, register).map_err(Error::Bus)?;
    let expected = (previous & !mask) | (value & mask);
    bus.write(ADDRESS, &[register, expected])
        .map_err(Error::Bus)?;
    let actual = read_reg(bus, register).map_err(Error::Bus)?;
    if actual & mask != expected & mask {
        return Err(Error::Verify {
            register,
            expected,
            actual,
        });
    }
    Ok(())
}

/// Only reset the host watchdog. Never restart the charge safety timer or
/// change the charge current while servicing the watchdog.
pub fn feed_watchdog<I: I2c>(bus: &mut I) -> Result<(), I::Error> {
    let value = read_reg(bus, REG_INPUT_WATCHDOG)?;
    bus.write(ADDRESS, &[REG_INPUT_WATCHDOG, value | 0x01])
}

pub fn full_voltage_mv(register: u8) -> u16 {
    3500 + (register >> 1) as u16 * 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::i2c::{ErrorKind, ErrorType, Operation};

    struct Mock {
        registers: [u8; 13],
        writes: Vec<(u8, u8)>,
        fail_write: Option<usize>,
        ignore_voltage_write: bool,
    }

    impl Mock {
        fn board() -> Self {
            Self {
                registers: [
                    0x90, 0xac, 0x0f, 0x91, 0x8d, 0x7a, 0xc0, 0x73, 0x02, 0x00, 0x60, 0x09, 0x7a,
                ],
                writes: Vec::new(),
                fail_write: None,
                ignore_voltage_write: false,
            }
        }
    }

    impl ErrorType for Mock {
        type Error = ErrorKind;
    }
    impl I2c for Mock {
        fn transaction(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), ErrorKind> {
            assert_eq!(address, ADDRESS);
            match operations {
                [Operation::Write(reg), Operation::Read(data)] => {
                    assert_eq!(reg.len(), 1);
                    assert_eq!(data.len(), 1);
                    data[0] = self.registers[reg[0] as usize];
                }
                [Operation::Write(data)] => {
                    assert_eq!(data.len(), 2);
                    let (reg, value) = (data[0], data[1]);
                    self.writes.push((reg, value));
                    if self.fail_write == Some(self.writes.len()) {
                        return Err(ErrorKind::Other);
                    }
                    if !(reg == REG_VOLTAGE && self.ignore_voltage_write) {
                        self.registers[reg as usize] = if reg == REG_INPUT_WATCHDOG {
                            value & !1 // WD_RST self-clears.
                        } else {
                            value
                        };
                    }
                }
                _ => panic!("unexpected I2C transaction"),
            }
            Ok(())
        }
    }

    #[test]
    fn configures_the_s_variant_before_enabling_charge() {
        let mut bus = Mock::board();
        initialize(&mut bus).unwrap();
        assert_eq!(bus.writes.first(), Some(&(REG_POWER, 0xac)));
        assert_eq!(bus.writes.last(), Some(&(REG_POWER, 0xa4)));
        assert!(!bus.writes[..bus.writes.len() - 1]
            .iter()
            .any(|&(reg, value)| reg == REG_POWER && value & CEB == 0));
        assert_eq!(8 + 8 * (bus.registers[2] & 0x7f) as u16, 344);
        assert_eq!(full_voltage_mv(bus.registers[4]), 4200);
        assert_eq!(50 + 30 * (bus.registers[12] >> 3) as u16, 500);
        assert_eq!(bus.registers[5], 0x3c); // Watchdog, termination, 8h timer.
        assert_eq!(bus.registers[6] & 0xe0, 0xc0); // NTC, BATFET.
        assert_eq!(bus.registers[8] & 0x60, 0); // PCB OTP and 6V OVP.
        assert_eq!(bus.registers[7], 0x73); // System rail unchanged.
    }

    #[test]
    fn unknown_and_legacy_chargers_are_not_written() {
        for id in [0x00, 0x02, 0xff] {
            let mut bus = Mock::board();
            bus.registers[11] = id;
            assert_eq!(initialize(&mut bus), Err(Error::UnsupportedDevice(id)));
            assert!(bus.writes.is_empty());
        }
    }

    #[test]
    fn failed_writes_leave_charging_disabled() {
        let mut successful = Mock::board();
        initialize(&mut successful).unwrap();
        for write in 1..=successful.writes.len() {
            let mut bus = Mock::board();
            bus.fail_write = Some(write);
            assert!(initialize(&mut bus).is_err());
            assert_ne!(bus.registers[1] & CEB, 0);
        }
    }

    #[test]
    fn verifies_voltage_before_enabling() {
        let mut bus = Mock::board();
        bus.registers[4] = 0x8b;
        bus.ignore_voltage_write = true;
        assert!(matches!(
            initialize(&mut bus),
            Err(Error::Verify {
                register: REG_VOLTAGE,
                ..
            })
        ));
        assert_ne!(bus.registers[1] & CEB, 0);
        assert!(!bus
            .writes
            .iter()
            .any(|&(reg, value)| reg == REG_POWER && value & CEB == 0));
    }

    #[test]
    fn watchdog_feed_does_not_raise_current_or_restart_safety_timer() {
        let mut bus = Mock::board();
        initialize(&mut bus).unwrap();
        bus.writes.clear();
        let before = bus.registers;
        for _ in 0..20 {
            feed_watchdog(&mut bus).unwrap();
        }
        assert_eq!(bus.registers, before);
        assert!(bus
            .writes
            .iter()
            .all(|&(reg, value)| reg == 0x0c && value == 0x7b));
    }

    #[test]
    fn decodes_s_variant_voltage_in_ten_mv_steps() {
        assert_eq!(full_voltage_mv(0x8d), 4200);
        assert_eq!(full_voltage_mv(0x8c), 4200);
        assert_eq!(full_voltage_mv(0xab), 4350);
    }
}
