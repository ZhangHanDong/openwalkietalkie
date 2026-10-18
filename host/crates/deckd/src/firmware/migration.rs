//! Update sequence, independent of HID so interruption paths can be exercised.
use super::{firmware_version, transport::crc32, Device};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub(super) struct Images {
    pub application: Vec<u8>,
    pub migration: Option<MigrationImages>,
}

pub(super) struct MigrationImages {
    pub bootstrap: Vec<u8>,
    pub bootloader: Vec<u8>,
}

pub(super) struct Request<'a> {
    pub expected: &'a Device,
    pub board: &'a str,
    pub version: &'a str,
    pub protocol: u8,
    pub allow_bootloader_update: bool,
}

pub(super) trait Io {
    fn describe(&mut self) -> Result<Device, String>;
    fn enter_recovery(&mut self) -> Result<u8, String>;
    fn transfer(
        &mut self,
        image: &[u8],
        protocol: u8,
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), String>;
    fn wait_application(&mut self) -> Result<Device, String>;
    fn boot_existing(&mut self) -> Result<(), String>;
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String>;
    fn backup(&mut self, bytes: &[u8]) -> Result<(), String>;
}

fn board_name(board: &str) -> &'static str {
    if board == "hw-b" {
        "Rev B"
    } else {
        "Rev A"
    }
}

fn refuse(io: &mut impl Io, why: &str) -> String {
    match io.boot_existing() {
        Ok(()) => format!("{why} No application flash was erased; the existing application was restarted."),
        Err(_) => format!("{why} No application flash was erased. The deck could not be restarted automatically; restart it to use its existing firmware."),
    }
}

pub(super) fn run(
    io: &mut impl Io,
    request: Request<'_>,
    images: &Images,
    mut progress: impl FnMut(&str, u8, &str),
) -> Result<Device, String> {
    let current = io.describe()?;
    let expected = request.expected;
    if current.serial != expected.serial
        || current.board.as_deref().is_some_and(|b| b != request.board)
        || current.version != expected.version
        || current.recovery != expected.recovery
        || current.maintenance != expected.maintenance
    {
        return Err(
            "The connected deck changed since the update check. Check again before installing."
                .into(),
        );
    }
    // A previous attempt may have lost the commit reply. Never reboot a
    // maintenance app until the entire resident bootloader has been verified.
    if current.maintenance && request.protocol != 2 {
        return Err("This device has an unfinished bootloader migration. Resume with a release containing its recovery images; keep power connected.".into());
    }
    let mut protocol = if current.maintenance {
        1
    } else {
        io.enter_recovery()?
    };
    if request.protocol == 2 && (protocol != 2 || current.maintenance) {
        if !request.allow_bootloader_update || images.migration.is_none() || protocol != 1 {
            let why = "A bootloader upgrade is required. Review and accept its power-loss warning, then retry with a complete release.";
            return Err(if current.maintenance {
                why.into()
            } else {
                refuse(io, why)
            });
        }
        let migration = images.migration.as_ref().unwrap();
        if !current.maintenance {
            progress(
                "installing",
                0,
                "Installing the temporary updater. Keep USB connected.",
            );
            io.transfer(&migration.bootstrap, protocol, &mut |p| {
                progress(
                    "installing",
                    p,
                    "Installing the temporary updater. Keep USB connected.",
                )
            })?;
            let temporary = io.wait_application()?;
            if temporary.serial != expected.serial
                || temporary.board.as_deref() != Some(request.board)
                || !temporary.maintenance
            {
                return Err("The temporary updater did not identify the expected board. No bootloader was written.".into());
            }
        }
        progress(
            "bootloader",
            0,
            "Checking and backing up the bootloader. Keep USB connected.",
        );
        replace_bootloader(io, request.board, &migration.bootloader, &mut progress)?;
        protocol = io.enter_recovery()?;
        if protocol != 2 {
            return Err("The replacement bootloader did not report recovery protocol 2. The final firmware was not written; keep USB connected and retry.".into());
        }
    }
    if let Err(error) = super::transport::preflight(&images.application, protocol) {
        return Err(refuse(io, &error));
    }
    progress("installing", 0, "Installing firmware. Keep USB connected.");
    io.transfer(&images.application, protocol, &mut |p| {
        progress("installing", p, "Installing firmware. Keep USB connected.")
    })?;
    progress("reconnecting", 100, "Verifying the updated device…");
    let mut updated = io.wait_application()?;
    if updated.serial != expected.serial
        || updated.board.as_deref() != Some(request.board)
        || updated.version.as_deref().and_then(firmware_version)
            != firmware_version(request.version)
        || updated.maintenance
        || updated.recovery
        || (request.protocol == 2 && updated.recovery_protocol.is_some_and(|p| p != 2))
    {
        return Err("The device restarted but did not confirm the expected firmware and bootloader. Check again and retry.".into());
    }
    // HELLO on this serial verified the resident recovery before transfer.
    // Early protocol-2 builds predate the read-only board marker in sys.version.
    updated.recovery_protocol = Some(protocol);
    Ok(updated)
}

fn read_slot(io: &mut impl Io, length: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(length);
    for offset in (0..length).step_by(64) {
        let n = (length - offset).min(64);
        let result = io.rpc("boot.read", json!({"o":offset,"n":n}))?;
        let hex = result["hex"]
            .as_str()
            .ok_or("Invalid bootloader read-back.")?;
        if hex.len() != n * 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid bootloader read-back.".into());
        }
        for at in (0..hex.len()).step_by(2) {
            bytes.push(
                u8::from_str_radix(&hex[at..at + 2], 16)
                    .map_err(|_| "Invalid bootloader read-back.")?,
            );
        }
    }
    Ok(bytes)
}

fn replace_bootloader(
    io: &mut impl Io,
    board: &str,
    image: &[u8],
    progress: &mut impl FnMut(&str, u8, &str),
) -> Result<(), String> {
    let info = io.rpc("boot.info", json!({}))?;
    let load = info["load_len"].as_u64().unwrap_or(0);
    if info["kind"] != format!("OpenWalkieTalkie Bootstrap 1 {}", board_name(board))
        || info["magic"] != 0x53454346u64
        || info["slot"] != 0x12010000u64
        || info["exec"] != 0x20020000u64
        || info["flags"] != 2u64
        || load < image.len() as u64
        || load > 65536
    {
        return Err("This device's flash table is incompatible with a USB bootloader upgrade. No bootloader was written. The temporary updater remains installed; contact support for hardware recovery.".into());
    }
    let original = read_slot(io, 65536)?;
    io.backup(&original)?;
    if original[..image.len()] == *image {
        return Ok(()); // Resume after a lost reply: verified, no needless erase.
    }
    let crc = crc32(image);
    if io.rpc("boot.begin", json!({"n":image.len(),"crc":crc}))?["ok"] != 1 {
        return Err("The temporary updater refused the bootloader image.".into());
    }
    for offset in (0..image.len()).step_by(128) {
        let end = (offset + 128).min(image.len());
        let reply = io.rpc(
            "boot.data",
            json!({"o":offset,"d":STANDARD.encode(&image[offset..end])}),
        )?;
        if reply["o"] != end {
            return Err("Bootloader staging failed. Nothing was erased; retry the update.".into());
        }
    }
    progress("bootloader", 50, "Replacing bootloader. Do not unplug USB, power off the device, or let your computer sleep.");
    // No automatic reboot or fallback from this point until read-back passes.
    let verify = (|| {
        let reply = io.rpc("boot.commit", json!({}))?;
        if reply["crc"] != crc || !(reply["written"] == image.len() || reply["written"] == 0) {
            return Err("Bootloader write verification failed.".into());
        }
        if read_slot(io, image.len())? != image {
            return Err("Bootloader read-back differs from the verified release image.".into());
        }
        Ok(())
    })();
    verify.map_err(|error: String| format!("{error} Keep power connected and retry the update. Do not restart or power-cycle the device; the bootloader may be incomplete."))?;
    progress(
        "bootloader",
        100,
        "Bootloader verified. Preparing the final firmware…",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        device: Device,
        protocol: u8,
        slot: Vec<u8>,
        staged: Vec<u8>,
        wanted: Vec<u8>,
        calls: Vec<&'static str>,
        fault: Option<&'static str>,
        temporary: bool,
        committed: bool,
    }

    fn fixture() -> (Fake, Images) {
        let bootloader = vec![42; 256];
        (
            Fake {
                device: Device {
                    serial: "REVB".into(),
                    board: Some("hw-b".into()),
                    version: Some("0.1.0-openwalkietalkie".into()),
                    ..Device::default()
                },
                protocol: 1,
                slot: vec![255; 65536],
                staged: Vec::new(),
                wanted: bootloader.clone(),
                calls: Vec::new(),
                fault: None,
                temporary: false,
                committed: false,
            },
            Images {
                application: vec![2; 32],
                migration: Some(MigrationImages {
                    bootstrap: vec![1; 32],
                    bootloader,
                }),
            },
        )
    }

    impl Io for Fake {
        fn describe(&mut self) -> Result<Device, String> {
            Ok(self.device.clone())
        }
        fn enter_recovery(&mut self) -> Result<u8, String> {
            self.calls.push("recovery");
            if self.slot[..self.wanted.len()] == self.wanted && self.fault != Some("old_protocol") {
                self.protocol = 2;
            }
            Ok(self.protocol)
        }
        fn transfer(&mut self, image: &[u8], _: u8, _: &mut dyn FnMut(u8)) -> Result<(), String> {
            self.temporary = image[0] == 1;
            let kind = if self.temporary {
                "bootstrap"
            } else {
                "application"
            };
            self.calls.push(kind);
            if self.fault == Some(kind) {
                return Err("USB interrupted".into());
            }
            Ok(())
        }
        fn wait_application(&mut self) -> Result<Device, String> {
            self.calls.push("wait");
            self.device.maintenance = self.temporary;
            self.device.version = Some("0.2.0-openwalkietalkie".into());
            self.device.recovery = false;
            self.device.recovery_protocol = Some(self.protocol);
            if self.fault == Some("no_marker") {
                self.device.recovery_protocol = None;
            }
            if self.fault == Some("wrong_board") && self.temporary {
                self.device.board = Some("hw-a".into());
            }
            if self.fault == Some("still_temporary") && !self.temporary {
                self.device.maintenance = true;
            }
            Ok(self.device.clone())
        }
        fn boot_existing(&mut self) -> Result<(), String> {
            self.calls.push("boot_existing");
            Ok(())
        }
        fn backup(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.calls.push("backup");
            assert_eq!(bytes.len(), 65536);
            if self.fault == Some("backup") {
                return Err("disk full".into());
            }
            Ok(())
        }
        fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
            match method {
                "boot.info" => {
                    self.calls.push("info");
                    Ok(
                        json!({"kind":"OpenWalkieTalkie Bootstrap 1 Rev B","magic":0x53454346u64,"slot":0x12010000u64,"exec":0x20020000u64,"flags":if self.fault == Some("table") { 1 } else { 2 },"load_len":65536}),
                    )
                }
                "boot.read" => {
                    let o = params["o"].as_u64().unwrap() as usize;
                    let n = params["n"].as_u64().unwrap() as usize;
                    if self.fault == Some("readback") && self.committed {
                        return Ok(json!({"hex":"broken"}));
                    }
                    Ok(
                        json!({"hex":self.slot[o..o+n].iter().map(|b| format!("{b:02x}")).collect::<String>()}),
                    )
                }
                "boot.begin" => {
                    self.calls.push("begin");
                    self.staged.clear();
                    Ok(json!({"ok":1}))
                }
                "boot.data" => {
                    if self.fault == Some("staging") {
                        return Err("USB interrupted".into());
                    }
                    assert_eq!(params["o"], self.staged.len());
                    self.staged
                        .extend(STANDARD.decode(params["d"].as_str().unwrap()).unwrap());
                    Ok(json!({"o":self.staged.len()}))
                }
                "boot.commit" => {
                    self.calls.push("commit");
                    self.slot[..self.staged.len()].copy_from_slice(&self.staged);
                    self.committed = true;
                    if self.fault == Some("commit_reply") {
                        return Err("reply lost".into());
                    }
                    Ok(
                        json!({"written":self.staged.len(),"crc":if self.fault == Some("crc") { 0 } else { crc32(&self.staged) }}),
                    )
                }
                _ => panic!("Unexpected RPC {method}"),
            }
        }
    }

    fn install(fake: &mut Fake, images: &Images, consent: bool) -> Result<Device, String> {
        let expected = fake.device.clone();
        run(
            fake,
            Request {
                expected: &expected,
                board: "hw-b",
                version: "0.2.0",
                protocol: 2,
                allow_bootloader_update: consent,
            },
            images,
            |_, _, _| {},
        )
    }

    #[test]
    fn revb_migration_backs_up_verifies_bootloader_then_installs_production() {
        let (mut fake, images) = fixture();
        let device = install(&mut fake, &images, true).unwrap();
        assert!(!device.maintenance);
        assert_eq!(device.recovery_protocol, Some(2));
        assert_eq!(
            fake.calls,
            [
                "recovery",
                "bootstrap",
                "wait",
                "info",
                "backup",
                "begin",
                "commit",
                "recovery",
                "application",
                "wait"
            ]
        );
    }

    #[test]
    fn current_recovery_updates_without_rewriting_bootloader_or_consent() {
        let (mut fake, images) = fixture();
        fake.protocol = 2;
        install(&mut fake, &images, false).unwrap();
        assert_eq!(fake.calls, ["recovery", "application", "wait"]);
    }

    #[test]
    fn early_v2_without_marker_is_verified_by_recovery_hello() {
        let (mut fake, images) = fixture();
        fake.protocol = 2;
        fake.fault = Some("no_marker");
        assert_eq!(
            install(&mut fake, &images, false)
                .unwrap()
                .recovery_protocol,
            Some(2)
        );
        assert_eq!(fake.calls, ["recovery", "application", "wait"]);
    }

    #[test]
    fn refusal_restarts_existing_firmware_before_any_flash_write() {
        for reason in ["consent", "assets", "protocol"] {
            let (mut fake, mut images) = fixture();
            if reason == "assets" {
                images.migration = None;
            }
            if reason == "protocol" {
                fake.protocol = 3;
            }
            assert!(install(&mut fake, &images, reason != "consent").is_err());
            assert_eq!(fake.calls, ["recovery", "boot_existing"]);
        }
    }

    #[test]
    fn wrong_board_table_backup_and_staging_fail_before_bootloader_erase() {
        for fault in ["bootstrap", "wrong_board", "table", "backup", "staging"] {
            let (mut fake, images) = fixture();
            fake.fault = Some(fault);
            assert!(install(&mut fake, &images, true).is_err(), "{fault}");
            assert!(!fake.calls.contains(&"commit"), "{fault}");
            assert!(!fake.calls.contains(&"application"), "{fault}");
        }
    }

    #[test]
    fn uncertain_commit_or_readback_never_reboots_or_installs_final_image() {
        for fault in ["commit_reply", "readback", "crc"] {
            let (mut fake, images) = fixture();
            fake.fault = Some(fault);
            let error = install(&mut fake, &images, true).unwrap_err();
            assert!(error.contains("Do not restart"));
            assert_eq!(fake.calls.iter().filter(|&&c| c == "recovery").count(), 1);
            assert!(!fake.calls.contains(&"boot_existing"));
            assert!(!fake.calls.contains(&"application"));
        }
    }

    #[test]
    fn retry_after_lost_commit_reply_verifies_before_reboot_without_reerase() {
        let (mut fake, images) = fixture();
        fake.fault = Some("commit_reply");
        assert!(install(&mut fake, &images, true).is_err());
        assert!(fake.device.maintenance);
        fake.fault = None;
        fake.calls.clear();
        install(&mut fake, &images, true).unwrap();
        assert_eq!(
            fake.calls,
            ["info", "backup", "recovery", "application", "wait"]
        );
    }

    #[test]
    fn partial_bootloader_retry_repairs_in_place_before_reboot() {
        let (mut fake, images) = fixture();
        fake.device.maintenance = true;
        fake.device.recovery_protocol = Some(2); // A marker alone is not proof.
        fake.slot[..64].copy_from_slice(&fake.wanted[..64]);
        install(&mut fake, &images, true).unwrap();
        assert_eq!(
            fake.calls,
            [
                "info",
                "backup",
                "begin",
                "commit",
                "recovery",
                "application",
                "wait"
            ]
        );
    }

    #[test]
    fn old_protocol_after_replacement_and_unfinished_final_image_are_errors() {
        for fault in ["old_protocol", "application", "still_temporary"] {
            let (mut fake, images) = fixture();
            fake.fault = Some(fault);
            assert!(install(&mut fake, &images, true).is_err());
            if fault == "old_protocol" {
                assert!(!fake.calls.contains(&"application"));
            }
        }
    }

    #[test]
    fn swapped_device_is_rejected_without_commands() {
        let (mut fake, images) = fixture();
        let expected = fake.device.clone();
        fake.device.serial = "OTHER".into();
        assert!(run(
            &mut fake,
            Request {
                expected: &expected,
                board: "hw-b",
                version: "0.2.0",
                protocol: 2,
                allow_bootloader_update: true
            },
            &images,
            |_, _, _| {}
        )
        .is_err());
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn legacy_release_cannot_bypass_an_unfinished_bootloader_migration() {
        let (mut fake, images) = fixture();
        fake.device.maintenance = true;
        let expected = fake.device.clone();
        assert!(run(
            &mut fake,
            Request {
                expected: &expected,
                board: "hw-b",
                version: "0.2.0",
                protocol: 1,
                allow_bootloader_update: false
            },
            &images,
            |_, _, _| {}
        )
        .is_err());
        assert!(fake.calls.is_empty());
    }
}
