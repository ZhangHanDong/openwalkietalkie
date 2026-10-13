//! Append-only journal in the last 64 KiB of the application partition: the
//! bond and the device preferences, each kind's latest valid record winning.
//! Writes only erased 64-byte slots. A torn write leaves the previous valid
//! record available. Identical records are not rewritten. 1024 updates are
//! available; exhaustion is reported rather than erasing keys. The one erase
//! is the factory reset (`erase`), which blanks the whole block.

use super::bond_record::{Prefs, Record, SIZE};
use sifli_hal::pac;

pub const START: usize = 0x1271_0000;
const LENGTH: usize = 64 * 1024;

#[derive(Debug)]
pub enum Error {
    Full,
    UnsupportedFlash,
    Controller,
    Verify,
}

pub fn load() -> Option<Record> {
    let mut latest = None;
    for offset in (0..LENGTH).step_by(SIZE) {
        if let Some(record) = Record::decode(&read_slot(START + offset)) {
            latest = Some(record);
        }
    }
    latest
}

/// The latest preferences record, or the defaults of a fresh deck.
pub fn load_prefs() -> Prefs {
    let mut latest = Prefs::default();
    for offset in (0..LENGTH).step_by(SIZE) {
        if let Some(prefs) = Prefs::decode(&read_slot(START + offset)) {
            latest = prefs;
        }
    }
    latest
}

fn read_slot(address: usize) -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((address + i) as *const u8) };
    }
    bytes
}

pub fn save(record: &Record) -> Result<(), Error> {
    if load().as_ref() == Some(record) {
        return Ok(());
    }
    append(&record.encode())
}

pub fn save_prefs(prefs: &Prefs) -> Result<(), Error> {
    if load_prefs() == *prefs {
        return Ok(());
    }
    append(&prefs.encode())
}

/// Program `bytes` into the first erased slot after every used one.
fn append(bytes: &[u8; SIZE]) -> Result<(), Error> {
    // Append after every previously used slot, including invalid/torn ones.
    let mut next = START;
    for offset in (0..LENGTH).step_by(SIZE) {
        if read_slot(START + offset) != [255; SIZE] {
            next = START + offset + SIZE;
        }
    }
    if next >= START + LENGTH {
        return Err(Error::Full);
    }
    let bytes = *bytes; // Must be in SRAM before flash becomes busy.
    let result = cortex_m::interrupt::free(|_| unsafe {
        program_slot((next - 0x1200_0000) as u32, bytes.as_ptr())
    });
    status(result)?;
    unsafe {
        let mut scb = cortex_m::Peripherals::steal().SCB;
        scb.invalidate_dcache_by_address(next, SIZE);
    }
    if read_slot(next) != bytes {
        return Err(Error::Verify);
    }
    Ok(())
}

/// Factory reset: blank the whole journal (bond and preferences) with one
/// 64 KiB block erase. Takes up to two seconds with interrupts off; the
/// caller reboots afterwards, so the Bluetooth stack never sees the key go.
pub fn erase() -> Result<(), Error> {
    let result = cortex_m::interrupt::free(|_| unsafe { erase_block((START - 0x1200_0000) as u32) });
    status(result)?;
    unsafe {
        let mut scb = cortex_m::Peripherals::steal().SCB;
        scb.invalidate_dcache_by_address(START, LENGTH);
    }
    for offset in (0..LENGTH).step_by(SIZE) {
        if read_slot(START + offset) != [255; SIZE] {
            return Err(Error::Verify);
        }
    }
    Ok(())
}

fn status(result: u32) -> Result<(), Error> {
    match result {
        0 => Ok(()),
        1 => Err(Error::UnsupportedFlash),
        _ => Err(Error::Controller),
    }
}

// ALL code executed while NOR is busy must live in SRAM, with no calls into
// flash (including compiler helpers, panic paths, logging or const tables).
// Check the release disassembly whenever changing these routines. .data is
// copied into SRAM by cortex-m-rt before main; no XIP settings are changed.
// Sequence follows SiFli HAL_QSPIEX_WRITE_PAGE's non-DMA 64-byte FIFO path.
#[inline(never)]
#[link_section = ".data.bond_flash"]
unsafe fn program_slot(offset: u32, bytes: *const u8) -> u32 {
    let mpi = pac::MPI2;
    let cr = mpi.cr().read();
    let saved = (cr.0, mpi.ccr1().read().0, mpi.dlr1().read().0, mpi.ar1().read().0);
    // This board has one 16 MiB SPI NOR on MPI2. Dual flash is unsupported.
    if cr.dfm() {
        return 1;
    }
    if cr.dmae() || cr.cmd2e() || cr.sme2() {
        return 2;
    }
    let mut status = write_enable();
    if status == 0 {
        let mut i = 0;
        while i < SIZE {
            let word = (*bytes.add(i) as u32)
                | ((*bytes.add(i + 1) as u32) << 8)
                | ((*bytes.add(i + 2) as u32) << 16)
                | ((*bytes.add(i + 3) as u32) << 24);
            mpi.dr().write(|w| w.0 = word);
            i += 4;
        }
        mpi.dlr1().write(|w| w.0 = (SIZE - 1) as u32);
        // Program single-line SPI, 3-byte address, one page-aligned 64B slot.
        if !command(0x02, offset, false, true, true) {
            status = 2;
        }
        // Page program: 3 ms at most.
        wait_idle(100_000);
    }
    restore(saved);
    status
}

/// One 64 KiB block erase (SiFli HAL_QSPIEX_BLK64_ERASE's command: 0xD8,
/// 3-byte address, no data), waited out here like a program.
#[inline(never)]
#[link_section = ".data.bond_flash"]
unsafe fn erase_block(offset: u32) -> u32 {
    let mpi = pac::MPI2;
    let cr = mpi.cr().read();
    let saved = (cr.0, mpi.ccr1().read().0, mpi.dlr1().read().0, mpi.ar1().read().0);
    if cr.dfm() {
        return 1;
    }
    if cr.dmae() || cr.cmd2e() || cr.sme2() {
        return 2;
    }
    let mut status = write_enable();
    if status == 0 {
        mpi.dlr1().write(|w| w.0 = 0);
        if !command(0xd8, offset, false, false, true) {
            status = 2;
        }
        // Block erase: 2 s at most on the W25Q128JV, 150 ms typical. A try
        // is one status read of a few microseconds.
        wait_idle(2_000_000);
    }
    restore(saved);
    status
}

/// Identify the W25Q128JV and set its write-enable latch. 0 = ready, 1 =
/// not this flash, 2 = the controller did not answer.
#[inline(always)]
unsafe fn write_enable() -> u32 {
    let mpi = pac::MPI2;
    let mut status = 0;
    // The shared AHB/command receive path can initially expose bytes from
    // the last XIP burst. Repeat this harmless command entirely in SRAM;
    // require the last two replies to identify this board's W25Q128JV.
    // Never clear its FIFO while an AHB read may still be finishing.
    let mut last_id = 0;
    let mut id = 0;
    for _ in 0..8 {
        mpi.dlr1().write(|w| w.0 = 2);
        if !command(0x9f, 0, true, false, false) {
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
        if !command(0x06, 0, false, false, false) {
            status = 2;
        }
        mpi.dlr1().write(|w| w.0 = 0);
        if !command(0x05, 0, true, false, false) || mpi.dr().read().0 & 2 == 0 {
            status = 2;
        } // write-enable latch
    }
    status
}

/// Do not return to XIP while a program or erase is in progress. On a stuck
/// controller reset the MCU from SRAM instead of executing busy NOR.
#[inline(always)]
unsafe fn wait_idle(mut tries: u32) {
    let mpi = pac::MPI2;
    loop {
        mpi.dlr1().write(|w| w.0 = 0);
        if !command(0x05, 0, true, false, false) {
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

/// Put back the XIP command registers (`cr`, `ccr1`, `dlr1`, `ar1`) that
/// the commands above overwrote.
#[inline(always)]
unsafe fn restore((cr, ccr, dlr, ar): (u32, u32, u32, u32)) {
    let mpi = pac::MPI2;
    mpi.fifocr().modify(|w| {
        w.set_rxclr(true);
        w.set_txclr(true);
    });
    mpi.ccr1().write(|w| w.0 = ccr);
    mpi.dlr1().write(|w| w.0 = dlr);
    mpi.ar1().write(|w| w.0 = ar);
    mpi.cr().write(|w| w.0 = cr);
}

/// One command: `read` = a data phase of `dlr1` + 1 bytes comes in, `write`
/// = one goes out (page program, FMODE 1), `addressed` = a 3-byte address
/// phase. These are the SDK flash table's mode columns for 0x9F / 0x05
/// (read), 0x06 (none), 0x02 (write, addressed) and 0xD8 (addressed, no
/// data).
#[inline(always)]
unsafe fn command(opcode: u8, address: u32, read: bool, write: bool, addressed: bool) -> bool {
    let mpi = pac::MPI2;
    mpi.scr().write(|w| w.set_tcfc(true));
    mpi.ccr1().write(|w| {
        w.set_imode(1);
        w.set_fmode(write);
        w.set_dmode(if read || write { 1 } else { 0 });
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
    if read {
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
