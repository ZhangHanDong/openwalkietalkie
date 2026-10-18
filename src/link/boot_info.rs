//! Read-only identification of the installed recovery image.

pub const RECOVERY_MARKER: &[u8] = if cfg!(feature = "hw-b") {
    b"OpenWalkieTalkie Recovery 2 Rev B"
} else {
    b"OpenWalkieTalkie Recovery 2 Rev A"
};

#[cfg(feature = "bootloader-update")]
pub const BOOTSTRAP_MARKER: &[u8] = if cfg!(feature = "hw-b") {
    b"OpenWalkieTalkie Bootstrap 1 Rev B"
} else {
    b"OpenWalkieTalkie Bootstrap 1 Rev A"
};

pub fn recovery_protocol() -> Option<u8> {
    // Main-board bootloader slot only; no Bluetooth keys or flash-table keys.
    let bytes = unsafe { core::slice::from_raw_parts(0x1201_0000 as *const u8, 65536) };
    bytes
        .windows(RECOVERY_MARKER.len())
        .any(|w| w == RECOVERY_MARKER)
        .then_some(2)
}
