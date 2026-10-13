//! The resident OWTB recovery protocol (native/bootloader): application
//! window only, CRC per sector, vector sector committed last by the device.
use super::{firmware_version, Device, APP_BASE};
use crate::hid::{PID, USAGE_PAGE, VID};
use hidapi::{BusType, HidApi, HidDevice};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const SECTOR: usize = 4096;
const RECOVERY: u8 = 9;

struct Wire(HidDevice);
impl Wire {
    fn send(&self, channel: u8, payload: &[u8]) -> Result<(), String> {
        if payload.len() > 61 {
            return Err("USB payload too large.".into());
        }
        let mut report = [0; 64];
        report[0] = 6;
        report[1] = channel;
        report[2] = payload.len() as u8;
        report[3..3 + payload.len()].copy_from_slice(payload);
        match self.0.write(&report) {
            Ok(64) => Ok(()),
            _ => Err("USB write failed. Reconnect the deck and retry.".into()),
        }
    }
    fn recv(&self, channel: u8, deadline: Instant) -> Result<Option<Vec<u8>>, String> {
        while Instant::now() < deadline {
            let mut report = [0; 64];
            let n = self
                .0
                .read_timeout(&mut report, 100)
                .map_err(|_| "USB connection lost. Reconnect and retry.")?;
            if n == 0 {
                continue;
            }
            let body = if report[0] == 6 {
                &report[1..n]
            } else {
                &report[..n]
            };
            if body.len() < 2 || body[0] != channel {
                continue;
            }
            let len = body[1] as usize;
            if len > 61 || len > body.len() - 2 {
                return Err("Malformed USB report.".into());
            }
            return Ok(Some(body[2..2 + len].to_vec()));
        }
        Ok(None)
    }
    fn hello(&self) -> Result<bool, String> {
        self.send(RECOVERY, &[1])?;
        let until = Instant::now() + Duration::from_millis(500);
        while let Some(reply) = self.recv(RECOVERY, until)? {
            if reply.starts_with(b"\x01OWTB") {
                if reply.len() < 7 || reply[5] != 1 {
                    return Err("Unsupported recovery protocol.".into());
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let request = format!("{}\n", json!({"method":method,"params":params,"id":47201}));
        for chunk in request.as_bytes().chunks(61) {
            self.send(2, chunk)?;
        }
        let until = Instant::now() + Duration::from_secs(2);
        let mut line = Vec::new();
        while let Some(bytes) = self.recv(2, until)? {
            line.extend(bytes);
            if line.len() > 4096 {
                return Err("Oversized device reply.".into());
            }
            if let Some(end) = line.iter().position(|b| *b == b'\n') {
                let reply: Value =
                    serde_json::from_slice(&line[..end]).map_err(|_| "Invalid device reply.")?;
                if reply.get("id").and_then(Value::as_u64) != Some(47201) {
                    line.drain(..=end);
                    continue;
                }
                return reply
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "Device refused the update request.".into());
            }
        }
        Err("The deck did not answer. Connect its USB cable and try again.".into())
    }
    fn command(&self, op: u8, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut payload = vec![op];
        payload.extend_from_slice(data);
        self.send(RECOVERY, &payload)?;
        let until = Instant::now() + Duration::from_secs(10);
        while let Some(reply) = self.recv(RECOVERY, until)? {
            if reply.first() == Some(&op) {
                if reply.get(1) != Some(&0) {
                    return Err(format!(
                        "Recovery command {op} failed. Reconnect and retry."
                    ));
                }
                return Ok(reply[2..].to_vec());
            }
        }
        Err("Recovery timed out. Keep the deck on USB and retry the update.".into())
    }
}

fn candidates(api: &mut HidApi) -> Result<Vec<(String, std::ffi::CString)>, String> {
    api.set_open_exclusive(false);
    api.refresh_devices().map_err(|e| e.to_string())?;
    Ok(api
        .device_list()
        .filter(|d| {
            d.vendor_id() == VID
                && d.product_id() == PID
                && d.usage_page() == USAGE_PAGE
                && matches!(d.bus_type(), BusType::Usb)
        })
        .filter_map(|d| {
            d.serial_number()
                .filter(|s| !s.is_empty())
                .map(|s| (s.to_string(), d.path().to_owned()))
        })
        .collect())
}

fn describe(wire: &Wire, serial: String) -> Result<Device, String> {
    if wire.hello()? {
        return Ok(Device {
            serial,
            recovery: true,
            ..Default::default()
        });
    }
    let result = wire.rpc("sys.version", json!({}))?;
    let version = result
        .get("version")
        .and_then(Value::as_str)
        .ok_or("Firmware version missing.")?;
    if !version.ends_with("-openwalkietalkie") || firmware_version(version).is_none() {
        return Err("This updater supports the standard OpenWalkieTalkie firmware; this device runs a different image.".into());
    }
    let board = match result.get("board").and_then(Value::as_str) {
        Some("Rev A") => Some("hw-a".into()),
        Some("Rev B") => Some("hw-b".into()),
        None => None,
        _ => return Err("This device board is not supported by the USB updater.".into()),
    };
    Ok(Device {
        serial,
        board,
        version: Some(version.into()),
        recovery: false,
    })
}

pub fn inspect(api: &mut HidApi) -> Result<Option<Device>, String> {
    let found = candidates(api)?;
    if found.len() > 1 {
        return Err("Connect only the deck you want to update by USB.".into());
    }
    let Some((serial, path)) = found.into_iter().next() else {
        return Ok(None);
    };
    let wire = Wire(api.open_path(&path).map_err(|e| e.to_string())?);
    describe(&wire, serial).map(Some)
}

fn open_serial(api: &mut HidApi, serial: &str) -> Result<Wire, String> {
    let path = candidates(api)?
        .into_iter()
        .find(|(s, _)| s == serial)
        .map(|(_, p)| p)
        .ok_or("The selected deck is no longer connected by USB.")?;
    Ok(Wire(api.open_path(&path).map_err(|e| e.to_string())?))
}

fn wait<T>(mut f: impl FnMut() -> Option<T>, what: &str) -> Result<T, String> {
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some(value) = f() {
            return Ok(value);
        }
        if Instant::now() >= until {
            return Err(format!(
                "Timed out waiting for {what}. Reconnect USB and check again."
            ));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

pub fn install(
    api: &mut HidApi,
    expected: &Device,
    board: &str,
    version: &str,
    image: &[u8],
    mut progress: impl FnMut(&str, u8),
) -> Result<Device, String> {
    let mut wire = open_serial(api, &expected.serial)?;
    let current = describe(&wire, expected.serial.clone())?;
    // Check again after download and before the first mutating USB command.
    if current
        .board
        .as_deref()
        .is_some_and(|actual| actual != board)
        || current.version != expected.version
        || current.recovery != expected.recovery
    {
        return Err(
            "The connected deck changed since the update check. Check again before installing."
                .into(),
        );
    }
    if !current.recovery {
        wire.rpc("sys.reboot", json!({"mode":"recovery"}))?;
        drop(wire);
        wire = wait(
            || {
                let w = open_serial(api, &expected.serial).ok()?;
                w.hello().ok()?.then_some(w)
            },
            "this deck's recovery mode",
        )?;
    }
    transfer(
        image,
        |op, payload, reply| {
            if reply {
                wire.command(op, payload)
            } else {
                let mut bytes = vec![op];
                bytes.extend_from_slice(payload);
                wire.send(RECOVERY, &bytes)?;
                Ok(Vec::new())
            }
        },
        |percent| progress("installing", percent),
    )?;
    drop(wire);
    progress("reconnecting", 100);
    let updated = wait(
        || {
            let w = open_serial(api, &expected.serial).ok()?;
            let d = describe(&w, expected.serial.clone()).ok()?;
            (!d.recovery).then_some(d)
        },
        "the updated deck",
    )?;
    if updated.board.as_deref() != Some(board)
        || updated.version.as_deref().and_then(firmware_version) != firmware_version(version)
    {
        return Err(
            "The deck restarted, but did not report the expected firmware and board.".into(),
        );
    }
    Ok(updated)
}

fn transfer(
    image: &[u8],
    mut command: impl FnMut(u8, &[u8], bool) -> Result<Vec<u8>, String>,
    mut progress: impl FnMut(u8),
) -> Result<(), String> {
    command(2, &[], true)?;
    for (index, sector) in image.chunks(SECTOR).enumerate() {
        for (part, bytes) in sector.chunks(57).enumerate() {
            let offset = (part * 57) as u16;
            let mut payload = offset.to_le_bytes().to_vec();
            payload.push(bytes.len() as u8);
            payload.extend_from_slice(bytes);
            command(3, &payload, false)?;
        }
        let addr = APP_BASE + (index * SECTOR) as u32;
        let mut payload = addr.to_le_bytes().to_vec();
        payload.extend_from_slice(&(sector.len() as u16).to_le_bytes());
        payload.extend_from_slice(&crc32(sector).to_le_bytes());
        let reply = command(4, &payload, true)?;
        if reply.as_slice() != addr.to_le_bytes() {
            return Err("The bootloader acknowledged the wrong flash sector.".into());
        }
        progress(((index * SECTOR + sector.len()) * 100 / image.len()) as u8);
    }
    let reply = command(5, &[], true)?;
    if reply.first() != Some(&1) {
        return Err("The bootloader could not validate the new image. Retry from recovery.".into());
    }
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_matches_sector_protocol_including_short_final_sector() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        let image = vec![0xa5; 4200];
        let mut offset = 0;
        let mut staged = Vec::new();
        let mut completed = false;
        transfer(
            &image,
            |op, data, reply| {
                match op {
                    2 => assert!(reply),
                    3 => {
                        assert!(!reply);
                        assert_eq!(
                            u16::from_le_bytes(data[..2].try_into().unwrap()) as usize,
                            staged.len()
                        );
                        assert_eq!(data[2] as usize, data.len() - 3);
                        assert!(data.len() <= 60);
                        staged.extend_from_slice(&data[3..]);
                    }
                    4 => {
                        assert!(reply);
                        assert_eq!(
                            u32::from_le_bytes(data[..4].try_into().unwrap()),
                            APP_BASE + offset
                        );
                        assert_eq!(
                            u16::from_le_bytes(data[4..6].try_into().unwrap()) as usize,
                            staged.len()
                        );
                        assert_eq!(
                            u32::from_le_bytes(data[6..].try_into().unwrap()),
                            crc32(&staged)
                        );
                        assert_eq!(
                            &image[offset as usize..offset as usize + staged.len()],
                            staged
                        );
                        offset += staged.len() as u32;
                        staged.clear();
                        return Ok(data[..4].to_vec());
                    }
                    5 => {
                        assert_eq!(offset as usize, image.len());
                        completed = true;
                        return Ok(vec![1]);
                    }
                    _ => panic!(),
                }
                Ok(vec![])
            },
            |_| {},
        )
        .unwrap();
        assert!(completed);
    }
    #[test]
    fn failed_commit_never_finishes_or_boots() {
        let mut operations = Vec::new();
        let result = transfer(
            &[1; 12],
            |op, _, _| {
                operations.push(op);
                if op == 4 {
                    Err("CRC mismatch".into())
                } else {
                    Ok(vec![])
                }
            },
            |_| {},
        );
        assert!(result.is_err());
        assert!(!operations.contains(&5));
    }
}
