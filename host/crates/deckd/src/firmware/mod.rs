//! Release discovery and verified, board-specific USB application updates.
//! All network/USB work runs off the core. Only an explicit install call
//! acquires HID maintenance and writes flash; polling never installs anything.
mod transport;

use crate::hid::Maintenance;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::time::{Duration, Instant};

pub const REPOSITORY: &str = "conol-ai/openwalkietalkie";
pub const MANIFEST_URL: &str =
    "https://github.com/conol-ai/openwalkietalkie/releases/latest/download/release-manifest.json";
pub const APP_BASE: u32 = 0x12020000;
pub const APP_SIZE: usize = 7 * 1024 * 1024 - 64 * 1024;
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Firmware {
    pub version: String,
    pub protocol: u8,
    pub app_base: u32,
    pub app_size: usize,
    pub variants: BTreeMap<String, Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u8,
    pub product: String,
    pub release_url: String,
    pub firmware: Firmware,
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.product != "openwalkietalkie"
            || self.firmware.protocol != 1
            || self.firmware.app_base != APP_BASE
            || self.firmware.app_size != APP_SIZE
        {
            return Err("This release targets another product or flash layout.".into());
        }
        let version =
            Version::parse(&self.firmware.version).map_err(|_| "Invalid firmware version.")?;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err("The stable updater requires a stable firmware version.".into());
        }
        let prefix = format!("https://github.com/{REPOSITORY}/releases/tag/v");
        let tag = self
            .release_url
            .strip_prefix(&prefix)
            .ok_or("Unexpected release URL.")?;
        let tag_version = Version::parse(tag).map_err(|_| "Invalid release tag.")?;
        if !tag_version.pre.is_empty() || !tag_version.build.is_empty() {
            return Err("Invalid stable release tag.".into());
        }
        if self.firmware.variants.is_empty() {
            return Err("No firmware in this release.".into());
        }
        for (board, asset) in &self.firmware.variants {
            if !matches!(board.as_str(), "hw-a" | "hw-b") {
                return Err("Unknown board in release.".into());
            }
            let name = format!("OpenWalkieTalkie-{}-{board}.bin", self.firmware.version);
            let url = format!("https://github.com/{REPOSITORY}/releases/download/v{tag}/{name}");
            if asset.name != name
                || asset.url != url
                || !(8..=APP_SIZE).contains(&asset.size)
                || asset.sha256.len() != 64
                || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid firmware asset or download location.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Device {
    pub serial: String,
    pub board: Option<String>,
    pub version: Option<String>,
    pub recovery: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub phase: String,
    pub progress: u8,
    pub message: String,
    pub device: Option<Device>,
    pub release: Option<Manifest>,
    pub error: Option<String>,
    #[serde(default)]
    pub available: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            progress: 0,
            message: "Connect your deck with USB to update its firmware.".into(),
            device: None,
            release: None,
            error: None,
            available: false,
        }
    }
}

impl Status {
    pub fn busy(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "checking" | "downloading" | "installing" | "reconnecting"
        )
    }
}

#[derive(Clone, Default)]
pub struct Updater {
    state: Arc<Mutex<Status>>,
    checked: Arc<Mutex<Option<Instant>>>,
}

impl Updater {
    pub fn status(&self) -> Status {
        let mut state = self.state.lock().unwrap().clone();
        state.available = state
            .device
            .as_ref()
            .zip(state.release.as_ref())
            .is_some_and(|(d, r)| {
                d.recovery
                    || d.version
                        .as_deref()
                        .and_then(firmware_version)
                        .zip(firmware_version(&r.firmware.version))
                        .is_some_and(|(current, new)| new > current)
            });
        state
    }

    pub fn check(&self, cache: PathBuf, force: bool) -> Result<Status, String> {
        let mut state = self.state.lock().unwrap();
        if state.busy() {
            return Ok(state.clone());
        }
        if !force
            && self
                .checked
                .lock()
                .unwrap()
                .is_some_and(|t| t.elapsed() < CHECK_INTERVAL)
        {
            return Ok(state.clone());
        }
        state.phase = "checking".into();
        state.message = "Checking for firmware updates…".into();
        state.error = None;
        let initial = state.clone();
        drop(state);
        let this = self.clone();
        std::thread::Builder::new()
            .name("firmware-check".into())
            .spawn(move || {
                let result = (|| {
                    let device =
                        crate::hid::probe_mut(transport::inspect).map_err(|e| e.to_string())??;
                    this.state.lock().unwrap().device = device;
                    let path = cache.join("release-manifest.json");
                    fetch(MANIFEST_URL, &path, 128 * 1024)?;
                    let release: Manifest =
                        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                            .map_err(|e| format!("Invalid release manifest: {e}"))?;
                    release.validate()?;
                    let mut state = this.state.lock().unwrap();
                    state.release = Some(release);
                    state.phase = "ready".into();
                    state.message =
                        "Firmware release checked. Installation requires your confirmation.".into();
                    *this.checked.lock().unwrap() = Some(Instant::now());
                    Ok(())
                })();
                if let Err(e) = result {
                    this.fail(e);
                }
            })
            .map_err(|e| {
                self.fail(e.to_string());
                e.to_string()
            })?;
        Ok(initial)
    }

    pub fn install(
        &self,
        board: &str,
        serial: &str,
        version: &str,
        cache: PathBuf,
        maintenance: Maintenance,
    ) -> Result<Status, String> {
        let mut state = self.state.lock().unwrap();
        if state.busy() {
            return Err("An update operation is already running.".into());
        }
        let device = state
            .device
            .clone()
            .ok_or("Check for updates with your deck connected by USB first.")?;
        let release = state
            .release
            .clone()
            .ok_or("Check for a firmware release first.")?;
        release.validate()?;
        if device.serial != serial || serial.is_empty() || release.firmware.version != version {
            return Err("The device or release changed. Check for updates again.".into());
        }
        if let Some(actual) = &device.board {
            if actual != board {
                return Err("This firmware is for a different board.".into());
            }
        }
        if !device.recovery {
            if let Some(current) = device.version.as_deref().and_then(firmware_version) {
                let new = Version::parse(version).map_err(|_| "Invalid version.")?;
                if new <= current {
                    return Err("The deck already has this version or a newer one.".into());
                }
            }
        }
        let asset = release
            .firmware
            .variants
            .get(board)
            .cloned()
            .ok_or("No firmware for the selected board.")?;
        state.phase = "downloading".into();
        state.progress = 0;
        state.error = None;
        state.message = "Downloading and verifying firmware…".into();
        let initial = state.clone();
        drop(state);
        let this = self.clone();
        let board = board.to_string();
        let version = version.to_string();
        std::thread::Builder::new()
            .name("firmware-install".into())
            .spawn(move || {
                let result = (|| {
                    let _pause = Pause::acquire(maintenance)?;
                    let path = cache.join(&asset.name);
                    // A bundle may carry the identical verified image for offline installation.
                    let bundled = std::env::current_exe().ok().and_then(|p| {
                        p.parent()?
                            .parent()
                            .map(|c| c.join("Resources/firmware").join(&asset.name))
                    });
                    let bytes = if let Some(path) = bundled.filter(|p| p.is_file()) {
                        std::fs::read(path).map_err(|e| e.to_string())?
                    } else {
                        fetch(&asset.url, &path, asset.size)?;
                        std::fs::read(path).map_err(|e| e.to_string())?
                    };
                    validate_image(&bytes, &asset, &board)?;
                    this.progress(
                        "installing",
                        0,
                        "Installing firmware. Keep the USB cable connected.",
                    );
                    let progress = this.clone();
                    let updated = crate::hid::probe_mut(move |api| {
                        transport::install(
                            api,
                            &device,
                            &board,
                            &version,
                            &bytes,
                            |phase, percent| {
                                progress.progress(
                                    phase,
                                    percent,
                                    if phase == "reconnecting" {
                                        "Waiting for the deck to restart…"
                                    } else {
                                        "Installing firmware. Keep the USB cable connected."
                                    },
                                );
                            },
                        )
                    })
                    .map_err(|e| e.to_string())??;
                    let mut state = this.state.lock().unwrap();
                    state.device = Some(updated);
                    state.phase = "complete".into();
                    state.progress = 100;
                    state.message = "Firmware updated. Your deck is ready.".into();
                    Ok(())
                })();
                if let Err(e) = result {
                    this.fail(e);
                }
            })
            .map_err(|e| {
                self.fail(e.to_string());
                e.to_string()
            })?;
        Ok(initial)
    }

    fn progress(&self, phase: &str, progress: u8, message: &str) {
        let mut state = self.state.lock().unwrap();
        state.phase = phase.into();
        state.progress = progress;
        state.message = message.into();
    }
    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap();
        state.phase = "error".into();
        state.error = Some(error);
        state.message = "Update did not complete. You can check again and retry.".into();
    }
}

/// Versions reported by the running firmware append the product name.
pub fn firmware_version(s: &str) -> Option<Version> {
    Version::parse(s.strip_suffix("-openwalkietalkie").unwrap_or(s)).ok()
}

pub fn validate_image(bytes: &[u8], asset: &Asset, board: &str) -> Result<(), String> {
    if bytes.len() != asset.size || !(8..=APP_SIZE).contains(&bytes.len()) {
        return Err("Firmware length does not match the release.".into());
    }
    if format!("{:x}", Sha256::digest(bytes)) != asset.sha256.to_ascii_lowercase() {
        return Err("Firmware checksum does not match the release.".into());
    }
    let sp = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let pc = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if !(0x20000000..=0x20080000).contains(&sp)
        || pc & 1 != 1
        || !(APP_BASE..APP_BASE + bytes.len() as u32).contains(&(pc & !1))
    {
        return Err("Invalid application vector table.".into());
    }
    let marker = match board {
        "hw-a" => b"OpenWalkieTalkie Rev A (".as_slice(),
        "hw-b" => b"OpenWalkieTalkie Rev B (".as_slice(),
        _ => return Err("Unknown board.".into()),
    };
    if !bytes.windows(marker.len()).any(|w| w == marker) {
        return Err("Firmware board identity does not match.".into());
    }
    Ok(())
}

fn fetch(url: &str, path: &Path, max: usize) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid cache path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let part = path.with_extension("part");
    let result = std::process::Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "120",
            "--max-filesize",
            &max.to_string(),
            "--output",
        ])
        .arg(&part)
        .arg(url)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&part);
        return Err("Could not download the release. Check your connection; the first release may not be published yet.".into());
    }
    if std::fs::metadata(&part).map_err(|e| e.to_string())?.len() > max as u64 {
        let _ = std::fs::remove_file(&part);
        return Err("Release download exceeded its size limit.".into());
    }
    std::fs::rename(part, path).map_err(|e| e.to_string())
}

struct Pause(Maintenance);
impl Pause {
    fn acquire(m: Maintenance) -> Result<Self, String> {
        m.ready.store(false, Ordering::Release);
        if m.requested.swap(true, Ordering::AcqRel) {
            return Err("USB maintenance is already active.".into());
        }
        let guard = Self(m);
        let until = Instant::now() + Duration::from_secs(10);
        while !guard.0.ready.load(Ordering::Acquire) {
            if Instant::now() >= until {
                return Err(
                    "The deck connection could not be paused. No firmware was written.".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(guard)
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        self.0.requested.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Manifest, Vec<u8>) {
        let mut image = vec![0; 128];
        image[..4].copy_from_slice(&0x20040000u32.to_le_bytes());
        image[4..8].copy_from_slice(&(APP_BASE + 9).to_le_bytes());
        let marker = b"OpenWalkieTalkie Rev A (test)";
        image[16..16 + marker.len()].copy_from_slice(marker);
        let asset = Asset { name: "OpenWalkieTalkie-0.2.0-hw-a.bin".into(), url: format!("https://github.com/{REPOSITORY}/releases/download/v0.3.0/OpenWalkieTalkie-0.2.0-hw-a.bin"), sha256: format!("{:x}", Sha256::digest(&image)), size: image.len() };
        let release = Manifest {
            schema: 1,
            product: "openwalkietalkie".into(),
            release_url: format!("https://github.com/{REPOSITORY}/releases/tag/v0.3.0"),
            firmware: Firmware {
                version: "0.2.0".into(),
                protocol: 1,
                app_base: APP_BASE,
                app_size: APP_SIZE,
                variants: BTreeMap::from([("hw-a".into(), asset)]),
            },
        };
        (release, image)
    }

    #[test]
    fn manifest_rejects_other_products_layouts_and_untrusted_locations() {
        let (release, _) = fixture();
        release.validate().unwrap();
        let mut bad = release.clone();
        bad.product = "openmicrokbd".into();
        assert!(bad.validate().is_err());
        let mut bad = release.clone();
        bad.firmware.app_base -= 65536;
        assert!(bad.validate().is_err());
        let mut bad = release.clone();
        bad.firmware.protocol = 2;
        assert!(bad.validate().is_err());
        let mut bad = release.clone();
        bad.firmware.variants.get_mut("hw-a").unwrap().url =
            "https://evil.example/image.bin".into();
        assert!(bad.validate().is_err());
        let mut bad = release.clone();
        bad.firmware.variants.get_mut("hw-a").unwrap().name = "../firmware.bin".into();
        assert!(bad.validate().is_err());
        let mut bad = release;
        bad.firmware.variants.get_mut("hw-a").unwrap().size = APP_SIZE + 1;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn corruption_wrong_board_and_invalid_vector_are_rejected_before_flash() {
        let (release, image) = fixture();
        let mut asset = release.firmware.variants["hw-a"].clone();
        validate_image(&image, &asset, "hw-a").unwrap();
        assert!(validate_image(&image, &asset, "hw-b").is_err());
        assert!(validate_image(&image[..127], &asset, "hw-a").is_err());
        let mut bad = image;
        bad[5] ^= 255;
        assert!(validate_image(&bad, &asset, "hw-a").is_err());
        asset.sha256 = format!("{:x}", Sha256::digest(&bad));
        assert!(validate_image(&bad, &asset, "hw-a").is_err());
    }

    #[test]
    fn install_refuses_stale_device_wrong_board_and_downgrades_without_spawning() {
        let (release, _) = fixture();
        let updater = Updater::default();
        let cache = PathBuf::from("/unused");
        let mut state = updater.state.lock().unwrap();
        state.release = Some(release);
        state.device = Some(Device {
            serial: "USB-A".into(),
            board: Some("hw-a".into()),
            version: Some("0.1.0-openwalkietalkie".into()),
            recovery: false,
        });
        drop(state);
        assert!(updater.status().available);
        for (board, serial, version) in [
            ("hw-b", "USB-A", "0.2.0"),
            ("hw-a", "USB-B", "0.2.0"),
            ("hw-a", "USB-A", "0.1.0"),
        ] {
            assert!(updater
                .install(
                    board,
                    serial,
                    version,
                    cache.clone(),
                    Maintenance::default()
                )
                .is_err());
        }
        updater
            .state
            .lock()
            .unwrap()
            .device
            .as_mut()
            .unwrap()
            .version = Some("0.3.0-openwalkietalkie".into());
        assert!(!updater.status().available);
        assert!(updater
            .install("hw-a", "USB-A", "0.2.0", cache, Maintenance::default())
            .is_err());
        assert_eq!(updater.status().phase, "idle");
    }

    #[test]
    fn maintenance_is_released_on_failure() {
        let maintenance = Maintenance::default();
        maintenance.requested.store(true, Ordering::Release);
        {
            let _guard = Pause(maintenance.clone());
        }
        assert!(!maintenance.requested.load(Ordering::Acquire));
    }

    #[test]
    fn firmware_version_compares_numerically() {
        assert!(
            firmware_version("0.10.0-openwalkietalkie")
                > firmware_version("0.9.0-openwalkietalkie")
        );
        assert_eq!(firmware_version("garbage"), None);
    }
}
