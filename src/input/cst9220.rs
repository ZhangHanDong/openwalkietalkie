//! CST9220 single-contact protocol. Identity/reset follows LilyGo; report
//! framing follows SensorLib's CST92xx driver. No controller firmware update.
//! https://github.com/Xinyuan-LilyGO/SlFli-SDK-Lilygo/tree/master/sifli-sdk/customer/peripherals/cst922
//! https://github.com/lewisxhe/SensorLib/blob/master/src/touch/TouchDrvCST92xx.cpp

use embedded_hal::i2c::I2c;

pub const ADDRESS: u8 = 0x5a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Contact { x: u16, y: u16 },
    Released,
    NotReady,
    Invalid,
}

impl Report {
    pub fn decode(bytes: [u8; 15]) -> Self {
        // ACK remains at D000 until another report is available. It is not
        // evidence that the finger lifted, especially when polling.
        if bytes[0] == 0xab {
            return Self::NotReady;
        }
        if bytes[6] != 0xab {
            return Self::Invalid;
        }
        let count = bytes[5] & 0x7f;
        // The coordinate slot can retain a pressed event in an empty frame.
        // Always inspect the report header before trusting that slot.
        if count == 0 || bytes[0] == 0 {
            return Self::Released;
        }
        // This UI has one pointer. Cancel ambiguous multi-finger/cover reports
        // instead of moving a held button to a different finger.
        if count != 1 || bytes[4] & 0x80 != 0 || bytes[0] >> 4 >= 2 {
            return Self::Invalid;
        }
        if bytes[0] & 0x0f != 0x06 {
            return Self::Released;
        }
        Self::Contact {
            x: (bytes[1] as u16) << 4 | (bytes[3] >> 4) as u16,
            y: (bytes[2] as u16) << 4 | (bytes[3] & 0x0f) as u16,
        }
    }
}

#[derive(Debug)]
pub struct Identity {
    pub checkcode: u32,
    pub chip: u16,
    pub project: u16,
}

pub struct Cst9220<I> {
    bus: I,
}

impl<I: I2c> Cst9220<I> {
    pub fn new(bus: I) -> Self {
        Self { bus }
    }

    /// Two-byte command, not an address-only I2C probe.
    pub fn enter_info_mode(&mut self) -> Result<(), I::Error> {
        self.bus.write(ADDRESS, &[0xd1, 0x01])
    }

    /// Call after the information-mode settling delay (40ms).
    pub fn identity(&mut self) -> Result<Identity, I::Error> {
        let mut check = [0; 4];
        let mut chip = [0; 4];
        self.bus.write_read(ADDRESS, &[0xd1, 0xfc], &mut check)?;
        self.bus.write_read(ADDRESS, &[0xd2, 0x04], &mut chip)?;
        Ok(Identity {
            checkcode: u32::from_le_bytes(check),
            chip: u16::from_le_bytes([chip[2], chip[3]]),
            project: u16::from_le_bytes([chip[0], chip[1]]),
        })
    }

    pub fn enter_normal_mode(&mut self) -> Result<(), I::Error> {
        self.bus.write(ADDRESS, &[0xd1, 0x09])
    }

    /// Acknowledge even a failed read to release the report/interrupt latch.
    /// Never turn a failed read into a touch at (0, 0).
    pub fn read_report(&mut self) -> Result<Report, I::Error> {
        let mut bytes = [0; 15];
        let read = self.bus.write_read(ADDRESS, &[0xd0, 0x00], &mut bytes);
        let ack = self.bus.write(ADDRESS, &[0xd0, 0x00, 0xab]);
        read?;
        ack?;
        Ok(Report::decode(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::i2c::{ErrorKind, ErrorType, Operation};

    struct Mock {
        fail: bool,
        calls: u8,
        bytes: [u8; 15],
    }
    impl ErrorType for Mock {
        type Error = ErrorKind;
    }
    impl I2c for Mock {
        fn transaction(&mut self, address: u8, ops: &mut [Operation<'_>]) -> Result<(), ErrorKind> {
            assert_eq!(address, ADDRESS);
            self.calls += 1;
            match ops {
                [Operation::Write(reg), Operation::Read(data)] => {
                    assert_eq!(*reg, &[0xd0, 0x00]);
                    assert_eq!(data.len(), 15);
                    if self.fail {
                        return Err(ErrorKind::Other);
                    }
                    data.copy_from_slice(&self.bytes);
                }
                [Operation::Write(data)] => assert_eq!(*data, &[0xd0, 0x00, 0xab]),
                _ => panic!("unexpected transaction"),
            }
            Ok(())
        }
    }

    fn pressed_frame() -> [u8; 15] {
        [
            0x16, 0x1d, 0x0f, 0xf0, 0x20, 1, 0xab, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
    }

    #[test]
    fn decodes_twelve_bit_coordinates_and_release() {
        assert_eq!(
            Report::decode(pressed_frame()),
            Report::Contact { x: 479, y: 240 }
        );
        let mut bytes = pressed_frame();
        bytes[0] = 0x10;
        assert_eq!(Report::decode(bytes), Report::Released);
    }

    #[test]
    fn empty_frame_releases_even_when_coordinate_slot_is_still_pressed() {
        let mut bytes = pressed_frame();
        bytes[5] = 0;
        assert_eq!(Report::decode(bytes), Report::Released);
    }

    #[test]
    fn acknowledged_buffer_is_not_a_new_release() {
        let mut bytes = pressed_frame();
        bytes[0] = 0xab;
        assert_eq!(Report::decode(bytes), Report::NotReady);
    }

    #[test]
    fn malformed_multi_finger_and_cover_frames_are_rejected() {
        for (index, value) in [(6, 0), (5, 2), (5, 3), (4, 0x80), (0, 0x26)] {
            let mut bytes = pressed_frame();
            bytes[index] = value;
            assert_eq!(Report::decode(bytes), Report::Invalid);
        }
    }

    #[test]
    fn reads_and_acknowledges_the_documented_register() {
        let mut driver = Cst9220::new(Mock {
            fail: false,
            calls: 0,
            bytes: pressed_frame(),
        });
        assert_eq!(
            driver.read_report().unwrap(),
            Report::Contact { x: 479, y: 240 }
        );
        assert_eq!(driver.bus.calls, 2);
    }

    #[test]
    fn empty_and_malformed_frames_are_also_acknowledged() {
        let mut empty = pressed_frame();
        empty[5] = 0;
        for bytes in [empty, [0; 15]] {
            let mut driver = Cst9220::new(Mock {
                fail: false,
                calls: 0,
                bytes,
            });
            assert!(driver.read_report().is_ok());
            assert_eq!(driver.bus.calls, 2);
        }
    }

    #[test]
    fn failed_read_is_acknowledged_and_not_reported_as_a_touch() {
        let mut driver = Cst9220::new(Mock {
            fail: true,
            calls: 0,
            bytes: pressed_frame(),
        });
        assert!(driver.read_report().is_err());
        assert_eq!(driver.bus.calls, 2);
    }
}
