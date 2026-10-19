//! NOR parts supported by the application's SRAM-only flash writers.
//!
//! These 3 V, 16 MiB parts share 3-byte addressing, 256-byte pages,
//! WREN (06h), RDSR (05h, BUSY bit 0 / WEL bit 1), PP (02h), sector erase
//! (20h) and block erase (D8h). The SDK bootloader already selects their
//! respective quad-read/status-register configuration by JEDEC ID.

/// Accept only two consecutive, identical JEDEC replies for a supported part.
/// MPI receives manufacturer first in the least-significant byte of DR.
///
/// This must inline into the SRAM writers: no lookup table or flash call is
/// allowed while NOR is busy. Keep the explicit IDs (including density/type),
/// rather than accepting every device from a supported manufacturer.
#[inline(always)]
pub const fn supported_stable_id(previous: u32, current: u32) -> bool {
    previous == current
        && matches!(
            current,
            0x1840ef // EF 40 18: Winbond W25Q128JVSIQ (C97521)
                | 0x182085 // 85 20 18: Puya PY25Q128HA-SUH-IR (C5354566)
                | 0x1840c8 // C8 40 18: GigaDevice GD25Q128ESIGR (C2928891)
        )
}

#[cfg(test)]
mod tests {
    use super::supported_stable_id;

    const IDS: [u32; 3] = [0x1840ef, 0x182085, 0x1840c8];

    #[test]
    fn accepts_supported_wire_ids_in_mpi_byte_order() {
        for wire in [[0xef, 0x40, 0x18], [0x85, 0x20, 0x18], [0xc8, 0x40, 0x18]] {
            let id = u32::from_le_bytes([wire[0], wire[1], wire[2], 0]);
            assert!(supported_stable_id(id, id));
            let reversed = u32::from_be_bytes([0, wire[0], wire[1], wire[2]]);
            assert!(!supported_stable_id(reversed, reversed));
        }
    }

    #[test]
    fn rejects_unstable_or_absent_replies() {
        for current in IDS {
            for previous in IDS.into_iter().chain([0, 0xffffff]) {
                assert_eq!(supported_stable_id(previous, current), previous == current);
            }
        }
        for absent in [0, 0xffffff, u32::MAX] {
            assert!(!supported_stable_id(absent, absent));
        }
    }

    #[test]
    fn rejects_other_manufacturers_types_and_densities() {
        // Include every single-byte substitution, not just nearby densities:
        // a matching manufacturer or capacity alone must never authorize WREN.
        for original in IDS {
            for shift in [0, 8, 16, 24] {
                for byte in 0..=255 {
                    let id = (original & !(0xff << shift)) | (byte << shift);
                    assert_eq!(supported_stable_id(id, id), IDS.contains(&id), "{id:08x}");
                }
            }
        }
    }
}
