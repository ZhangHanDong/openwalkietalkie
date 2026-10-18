//! Temporary migration firmware (`bootloader-update`, Rev A/B): replace the
//! USB recovery bootloader from the running application. Never leave this
//! feature in the normal application: while the slot is erased
//! and rewritten the board has no bootloader, and a power loss then leaves
//! only the UART debug port to put one back.
//!
//! On the system RPC channel, driven by `tools/flash-usb.py --bootloader`:
//!
//! - `boot.begin {"n":LEN,"crc":CRC32}` starts a new image of LEN bytes
//!   (at most the 64 KiB slot) whose zlib CRC-32 is CRC;
//! - `boot.data {"o":OFFSET,"d":BASE64}` appends the next piece, in order,
//!   into RAM; nothing touches flash yet;
//! - `boot.commit {}` checks the whole image (length, CRC, a vector table
//!   that points into SRAM, where the bootloader runs), then erases and
//!   programs the slot at 0x12010000, reads it back and answers
//!   `{"written":N,"crc":CRC}` from what flash now holds. An image equal to
//!   what is already there is not rewritten (`"written":0`).
//! - `boot.info {}` returns the installed flash table's bootloader mapping.
//! - `boot.read {"o":OFFSET,"n":LEN}` returns up to 64 slot bytes as hex for
//!   a pre-upgrade backup; it cannot read application or Bluetooth storage.
//!
//! The flash table, the application and the bond journal are never written;
//! the new bootloader runs at the next reboot (`sys.reboot`).
#[cfg(not(feature = "hw-a"))]
compile_error!("bootloader-update is for Rev A/B: LilyGo keeps its factory bootloader");

use super::wire::{as_str, find_key, parse_u32, Out};
use sifli_hal::pac;

/// The bootloader slot: flash offset, CPU address and the most it may hold.
const SLOT_OFFSET: u32 = 0x1_0000;
const SLOT_ADDRESS: usize = 0x1201_0000;
const SLOT_MAX: usize = 64 * 1024;
const SECTOR: usize = 4096;
/// One FIFO's worth: the program unit, as in `bond_flash`.
const CHUNK: usize = 64;

/// SDK sec_configuration: ftab[DFU_FLASH_IMG_BL] and imgs[BL - 2].
fn boot_mapping() -> [u32; 5] {
    // Fixed, read-only metadata; never expose the flash table's key fields.
    unsafe {
        [
            (0x1200_0000 as *const u32).read_volatile(),
            (0x1200_0034 as *const u32).read_volatile(),
            (0x1200_003c as *const u32).read_volatile(),
            (0x1200_1200 as *const u32).read_volatile(),
            u32::from((0x1200_1206 as *const u16).read_volatile()),
        ]
    }
}

struct Image {
    bytes: [u8; SLOT_MAX],
    len: usize,
    crc: u32,
    /// Bytes received so far; `boot.data` must continue exactly here.
    next: usize,
}

/// Zeroed so it costs .bss, not 64 KiB of image; `boot.begin` fills it.
static mut IMAGE: Image = Image {
    bytes: [0; SLOT_MAX],
    len: 0,
    crc: 0,
    next: 0,
};

/// A `boot.*` request: the reply's length in `tx`; `None` for anything else.
pub fn handle(req: &[u8], tx: &mut [u8]) -> Option<usize> {
    let method = find_key(req, b"method").and_then(as_str)?;
    let params = find_key(req, b"params").unwrap_or(b"{}");
    let num = |key: &[u8]| find_key(params, key).and_then(parse_u32);
    // SAFETY: only the link task that owns channel 2 calls this, one request
    // at a time.
    let image = unsafe { &mut *core::ptr::addr_of_mut!(IMAGE) };
    let mut out = Out::new(tx);
    let result: Result<(), &[u8]> = match method {
        b"boot.info" => {
            let values = boot_mapping();
            out.push(b"{\"result\":{\"kind\":\"");
            out.push(super::boot_info::BOOTSTRAP_MARKER);
            out.push(b"\",\"magic\":");
            out.push_u32(values[0]);
            for (key, value) in [
                (b",\"slot\":".as_slice(), values[1]),
                (b",\"exec\":".as_slice(), values[2]),
                (b",\"load_len\":".as_slice(), values[3]),
                (b",\"flags\":".as_slice(), values[4]),
            ] {
                out.push(key);
                out.push_u32(value);
            }
            out.push(b"}");
            Ok(())
        }
        b"boot.read" => match (num(b"o"), num(b"n")) {
            (Some(offset), Some(n)) if n > 0 && n <= 64 && offset <= (SLOT_MAX as u32) - n => {
                out.push(b"{\"result\":{\"hex\":\"");
                const HEX: &[u8] = b"0123456789abcdef";
                for &byte in &slot(SLOT_MAX)[offset as usize..(offset + n) as usize] {
                    out.push(&[HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]]);
                }
                out.push(b"\"}");
                Ok(())
            }
            _ => Err(b"read must stay within bootloader slot; n must be 1..64"),
        },
        b"boot.begin" => match (num(b"n"), num(b"crc")) {
            (Some(n), Some(crc)) if (1..=SLOT_MAX as u32).contains(&n) => {
                image.bytes.fill(0xff);
                image.len = n as usize;
                image.crc = crc;
                image.next = 0;
                out.push(b"{\"result\":{\"ok\":1}");
                Ok(())
            }
            _ => Err(b"n must be 1..65536 and crc given"),
        },
        b"boot.data" => {
            let data = find_key(params, b"d").and_then(as_str);
            match (num(b"o"), data) {
                (Some(o), Some(data)) if o as usize == image.next && image.len > 0 => {
                    match decode_into(data, &mut image.bytes[image.next..image.len]) {
                        Some(n) => {
                            image.next += n;
                            out.push(b"{\"result\":{\"o\":");
                            out.push_u32(image.next as u32);
                            out.push(b"}");
                            Ok(())
                        }
                        None => Err(b"bad base64, or past the announced length"),
                    }
                }
                _ => Err(b"out of order or no boot.begin"),
            }
        }
        b"boot.commit" => commit(image).map(|written| {
            out.push(b"{\"result\":{\"written\":");
            out.push_u32(written as u32);
            out.push(b",\"crc\":");
            out.push_u32(crc32(slot(image.len)));
            out.push(b"}");
        }),
        _ => return None,
    };
    if let Err(message) = result {
        out.push(b"{\"error\":{\"code\":-32000,\"message\":\"");
        out.push(message);
        out.push(b"\"}");
    }
    out.push(b",\"id\":");
    out.push(
        find_key(req, b"id")
            .filter(|id| id.len() <= 40)
            .unwrap_or(b"null"),
    );
    out.push(b"}");
    Some(out.finish())
}

/// What the slot holds now, through the cache.
fn slot(len: usize) -> &'static [u8] {
    // SAFETY: the bootloader slot is mapped, read-only memory.
    unsafe { core::slice::from_raw_parts(SLOT_ADDRESS as *const u8, len) }
}

fn commit(image: &Image) -> Result<usize, &'static [u8]> {
    let bytes = &image.bytes[..image.len];
    if image.len == 0 || image.next != image.len {
        return Err(b"the image is incomplete");
    }
    if crc32(bytes) != image.crc {
        return Err(b"CRC mismatch");
    }
    let marker = super::boot_info::RECOVERY_MARKER;
    if !bytes.windows(marker.len()).any(|w| w == marker) {
        return Err(b"bootloader is not recovery 2 for this board");
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let sram = 0x2000_0000..=0x2008_0000;
    if image.len < 8 || !sram.contains(&word(0)) || word(4) & 1 == 0 || !sram.contains(&word(4)) {
        return Err(b"not a bootloader: its vector table must point into SRAM");
    }
    let [magic, base, exec, load_len, flags] = boot_mapping();
    let entry = word(4) & !1;
    if magic != 0x5345_4346
        || base != SLOT_ADDRESS as u32
        || exec != 0x2002_0000
        || load_len < image.len as u32
        || load_len > SLOT_MAX as u32
        || flags != 2
        || entry < exec
        || entry >= exec + image.len as u32
    {
        return Err(b"installed flash table cannot load this bootloader; use UART");
    }
    if slot(image.len) == bytes {
        return Ok(0);
    }
    let end = image.len.div_ceil(CHUNK) * CHUNK;
    for sector in (0..end).step_by(SECTOR) {
        let status = cortex_m::interrupt::free(|_| unsafe {
            nor_op(SLOT_OFFSET + sector as u32, core::ptr::null())
        });
        if status != 0 {
            return Err(b"sector erase failed");
        }
    }
    for at in (0..end).step_by(CHUNK) {
        let chunk = image.bytes[at..at + CHUNK].as_ptr();
        let status =
            cortex_m::interrupt::free(|_| unsafe { nor_op(SLOT_OFFSET + at as u32, chunk) });
        if status != 0 {
            return Err(b"program failed");
        }
    }
    unsafe {
        let mut scb = cortex_m::Peripherals::steal().SCB;
        scb.invalidate_dcache_by_address(SLOT_ADDRESS, end);
    }
    if slot(image.len) != bytes {
        return Err(b"read-back differs: rewrite it over UART before rebooting");
    }
    Ok(image.len)
}

/// Standard base64 into `out`; the bytes written, or `None` on a bad
/// character or more data than `out` holds.
fn decode_into(text: &[u8], out: &mut [u8]) -> Option<usize> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut acc, mut bits, mut n) = (0u32, 0u32, 0usize);
    for &c in text.iter().take_while(|&&c| c != b'=') {
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            *out.get_mut(n)? = (acc >> bits) as u8;
            n += 1;
        }
    }
    Some(n)
}

/// zlib's CRC-32, as the bootloader's download protocol uses.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

// The rules of `bond_flash::program_slot` apply: everything that runs while
// the NOR is busy lives in SRAM and calls nothing in flash. A null `bytes`
// erases the 4 KiB sector at `offset`; otherwise one 64-byte chunk is
// programmed there. Status 0 = done, 1 = not this board's W25Q128JV, 2 = the
// controller did not respond.
#[inline(never)]
#[link_section = ".data.boot_update"]
unsafe fn nor_op(offset: u32, bytes: *const u8) -> u32 {
    let mpi = pac::MPI2;
    let cr = mpi.cr().read();
    let ccr = mpi.ccr1().read();
    let dlr = mpi.dlr1().read();
    let ar = mpi.ar1().read();
    if cr.dfm() {
        return 1;
    }
    if cr.dmae() || cr.cmd2e() || cr.sme2() {
        return 2;
    }
    let mut status = 0;
    let mut last_id = 0;
    let mut id = 0;
    for _ in 0..8 {
        mpi.dlr1().write(|w| w.0 = 2);
        if !command(0x9f, 0, Phase::Read) {
            status = 2;
            break;
        }
        last_id = id;
        id = mpi.dr().read().0 & 0x00ff_ffff;
    }
    if status == 0 && (id != 0x1840ef || last_id != id) {
        status = 1;
    }
    if status == 0 {
        if !command(0x06, 0, Phase::None) {
            status = 2;
        }
        mpi.dlr1().write(|w| w.0 = 0);
        if !command(0x05, 0, Phase::Read) || mpi.dr().read().0 & 2 == 0 {
            status = 2;
        } // write-enable latch
    }
    if status == 0 {
        let started = if bytes.is_null() {
            // Sector erase, 3-byte address, no data: up to 400 ms.
            command(0x20, offset, Phase::Address)
        } else {
            let mut i = 0;
            while i < CHUNK {
                let word = (*bytes.add(i) as u32)
                    | ((*bytes.add(i + 1) as u32) << 8)
                    | ((*bytes.add(i + 2) as u32) << 16)
                    | ((*bytes.add(i + 3) as u32) << 24);
                mpi.dr().write(|w| w.0 = word);
                i += 4;
            }
            mpi.dlr1().write(|w| w.0 = (CHUNK - 1) as u32);
            command(0x02, offset, Phase::Program)
        };
        if !started {
            status = 2;
        }
        // Never return to XIP while the NOR is busy: on a stuck controller
        // reset the MCU from SRAM instead.
        let mut tries = 50_000_000u32;
        loop {
            mpi.dlr1().write(|w| w.0 = 0);
            if !command(0x05, 0, Phase::Read) {
                reset_from_ram();
            }
            if mpi.dr().read().0 & 1 == 0 {
                break;
            }
            tries -= 1;
            if tries == 0 {
                reset_from_ram();
            }
        }
    }
    mpi.fifocr().modify(|w| {
        w.set_rxclr(true);
        w.set_txclr(true);
    });
    mpi.ccr1().write(|w| *w = ccr);
    mpi.dlr1().write(|w| *w = dlr);
    mpi.ar1().write(|w| *w = ar);
    mpi.cr().write(|w| *w = cr);
    status
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Instruction only.
    None,
    /// Instruction, then data read back.
    Read,
    /// Instruction and a 3-byte address, no data.
    Address,
    /// Instruction, a 3-byte address, then the FIFO's data written.
    Program,
}

#[inline(always)]
unsafe fn command(opcode: u8, address: u32, phase: Phase) -> bool {
    let mpi = pac::MPI2;
    let addressed = matches!(phase, Phase::Address | Phase::Program);
    mpi.scr().write(|w| w.set_tcfc(true));
    mpi.ccr1().write(|w| {
        w.set_imode(1);
        w.set_fmode(phase == Phase::Program);
        w.set_dmode(if matches!(phase, Phase::Read | Phase::Program) {
            1
        } else {
            0
        });
        w.set_admode(if addressed { 1 } else { 0 });
        w.set_adsize(if addressed { 2 } else { 0 });
    });
    mpi.ar1().write(|w| w.0 = address);
    mpi.cmdr1().write(|w| w.0 = opcode as u32);
    let mut tries = 1_000_000;
    while !mpi.sr().read().tcf() {
        tries -= 1;
        if tries == 0 {
            return false;
        }
    }
    if phase == Phase::Read {
        let mut tries = 1_000_000;
        while mpi.fifocr().read().rxe() {
            tries -= 1;
            if tries == 0 {
                return false;
            }
        }
    }
    mpi.scr().write(|w| w.set_tcfc(true));
    true
}

#[inline(always)]
unsafe fn reset_from_ram() -> ! {
    core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    core::ptr::write_volatile(0xe000_ed0c as *mut u32, 0x05fa_0004);
    loop {
        core::arch::asm!("nop", options(nostack, preserves_flags));
    }
}
