//! Bench only (`audio-debug`): read and write the audio path's registers
//! from the host, over the same request channel as everything else.
//!
//! `{"method":"dbg.peek","params":{"a":ADDR}}` answers `{"v":VALUE}`;
//! `dbg.poke` takes `v` and an optional mask `m` (bits outside it keep their
//! value) and answers what the register reads afterwards. Numbers are
//! decimal, as everywhere on this protocol. Only the blocks the speaker and
//! microphone use are reachable: this is for finding the codec settings a
//! quiet speaker needs with the deck's own microphone as the meter, and a
//! stray write elsewhere could do anything.
//!
//! `dbg.rec {"n":SAMPLES,"spk":0|1,"tone":PEAK}` records the microphone into
//! RAM (`audio::voice::bench`) with nothing on the radio; with `spk` the
//! speaker plays its own silence, or a tone of that peak, meanwhile.
//! `dbg.stat` says how far it is, and `dbg.read {"o":OFFSET}` hands the
//! samples back, base64 of little-endian 16-bit, a reply's worth at a time.
use super::wire::{as_str, find_key, parse_u32, Out};
use crate::audio::voice::bench;

/// Samples in one `dbg.read` reply (base64 of 2 bytes each, in `TX_CAP`).
const READ_SAMPLES: usize = 216;

/// Any peripheral register (HPSYS at 0x5000_0000, LPSYS at 0x4000_0000):
/// the hunt went from the codec to the radio to the pads of the flash bus,
/// and a bench build may as well reach them all. A stray write can do
/// anything, which is one more reason this feature never ships.
fn allowed(addr: u32) -> bool {
    addr % 4 == 0
        && ((0x4000_0000..0x5100_0000).contains(&addr)
            || (0x2000_0000..0x2008_0000).contains(&addr)
            || (0x6000_0000..0x6080_0000).contains(&addr)
            || (0x1200_0000..0x1300_0000).contains(&addr))
}

/// Copy a two-instruction function (`movs r0, #42; bx lr`) to `addr` and
/// call it: the CPU can execute from there when this answers 42.
fn exec_test(addr: u32) -> u32 {
    let at = addr as *mut u16;
    // SAFETY: a bench-only write into PSRAM or SRAM nothing else uses.
    unsafe {
        at.write_volatile(0x202A);
        at.add(1).write_volatile(0x4770);
        let mut cp = cortex_m::Peripherals::steal();
        cp.SCB.clean_dcache_by_address(addr as usize, 32);
        cp.SCB.invalidate_icache();
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        let f: extern "C" fn() -> u32 = core::mem::transmute(addr | 1);
        f()
    }
}

/// Bounds of the diagnostic area reserved by `memory.x` / `psram.x`.
fn psram_scratch() -> (usize, usize) {
    extern "C" {
        static __psram_scratch_start: u8;
        static __psram_scratch_end: u8;
    }
    let start = core::ptr::addr_of!(__psram_scratch_start) as usize;
    let end = core::ptr::addr_of!(__psram_scratch_end) as usize;
    (start, end - start)
}

/// Write a pattern over at most `bytes` of reserved PSRAM and read it back:
/// (mismatching words, milliseconds).
fn psram_test(bytes: u32, seed: u32) -> (u32, u32) {
    let (start, capacity) = psram_scratch();
    let base = start as *mut u32;
    let words = (bytes as usize).min(capacity) / 4;
    let started = embassy_time::Instant::now();
    let mut x = seed | 1;
    for i in 0..words {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        // SAFETY: inside the 8 MB PSRAM, which nothing else uses.
        unsafe { base.add(i).write_volatile(x) };
    }
    let mut x = seed | 1;
    let mut bad = 0u32;
    for i in 0..words {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        // SAFETY: as above.
        if unsafe { base.add(i).read_volatile() } != x {
            bad += 1;
        }
    }
    (bad, started.elapsed().as_millis() as u32)
}

/// Answer a `dbg.*` request into `tx`; `None` for anything else.
pub fn handle(req: &[u8], tx: &mut [u8]) -> Option<usize> {
    let method = find_key(req, b"method").and_then(as_str)?;
    if !method.starts_with(b"dbg.") {
        return None;
    }
    let params = find_key(req, b"params");
    let arg = |key: &[u8]| params.and_then(|p| find_key(p, key)).and_then(parse_u32);
    let mut out = Out::new(tx);
    out.push(b"{\"id\":");
    match find_key(req, b"id") {
        Some(id) if id.len() <= 40 => out.push(id),
        _ => out.push(b"null"),
    }
    let poke = match method {
        b"dbg.peek" => false,
        b"dbg.poke" => true,
        b"dbg.rec" => {
            bench::start(
                arg(b"n").unwrap_or(bench::SAMPLES as u32),
                arg(b"spk").unwrap_or(0) != 0,
                arg(b"tone").unwrap_or(0).min(u32::from(u16::MAX)) as u16,
            );
            out.push(b",\"result\":{\"ok\":true}}");
            return Some(out.finish());
        }
        b"dbg.burst" => {
            BURST_HZ.store(arg(b"hz").unwrap_or(66), Ordering::Release);
            BURST_N.store(arg(b"n").unwrap_or(0), Ordering::Release);
            BURST_KIND.store(arg(b"kind").unwrap_or(0), Ordering::Release);
            out.push(b",\"result\":{\"ok\":true}}");
            return Some(out.finish());
        }
        b"dbg.ptest" => {
            let (bad, ms) = psram_test(arg(b"n").unwrap_or(0x10_0000), arg(b"seed").unwrap_or(1));
            out.push(b",\"result\":{\"bad\":");
            out.push_u32(bad);
            out.push(b",\"ms\":");
            out.push_u32(ms);
            out.push(b"}}");
            return Some(out.finish());
        }
        b"dbg.pexec" => {
            let r = exec_test(arg(b"a").unwrap_or(psram_scratch().0 as u32));
            out.push(b",\"result\":{\"r\":");
            out.push_u32(r);
            out.push(b"}}");
            return Some(out.finish());
        }
        b"dbg.stat" => {
            out.push(b",\"result\":{\"have\":");
            out.push_u32(bench::HAVE.load(Ordering::Acquire));
            out.push(b",\"want\":");
            out.push_u32(bench::WANT.load(Ordering::Acquire));
            out.push(b",\"busy\":");
            out.push_u32(u32::from(bench::LOCAL.load(Ordering::Acquire)));
            out.push(b"}}");
            return Some(out.finish());
        }
        b"dbg.read" => {
            let mut samples = [0i16; READ_SAMPLES];
            let n = bench::read(arg(b"o").unwrap_or(0) as usize, &mut samples);
            let mut bytes = [0u8; 2 * READ_SAMPLES];
            for (pair, sample) in bytes.chunks_exact_mut(2).zip(&samples[..n]) {
                pair.copy_from_slice(&sample.to_le_bytes());
            }
            out.push(b",\"result\":{\"d\":\"");
            out.push_base64(&bytes[..2 * n]);
            out.push(b"\"}}");
            return Some(out.finish());
        }
        _ => {
            out.push(b",\"error\":{\"code\":-32601,\"message\":\"method\"}}");
            return Some(out.finish());
        }
    };
    match arg(b"a").filter(|addr| allowed(*addr)) {
        Some(addr) => {
            let reg = addr as *mut u32;
            // SAFETY: an aligned address inside one of the peripheral blocks
            // above; a volatile access is what the PAC does to them too.
            let value = unsafe {
                if let (true, Some(v)) = (poke, arg(b"v")) {
                    let mask = arg(b"m").unwrap_or(u32::MAX);
                    let old = reg.read_volatile();
                    reg.write_volatile((old & !mask) | (v & mask));
                }
                reg.read_volatile()
            };
            out.push(b",\"result\":{\"v\":");
            out.push_u32(value);
            out.push(b"}}");
        }
        None => out.push(b",\"error\":{\"code\":-32602,\"message\":\"address\"}}"),
    }
    Some(out.finish())
}

// ---------------------------------------------------------------------------
// Activity bursts: what does the amplifier pick up?
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicU32, Ordering};

/// `dbg.burst {"kind":K,"hz":F,"n":N}`: every 1/F s, do N units of one kind
/// of work, so a capture can tell which activity the speaker path hears.
/// 0 stops; 1 spins in RAM (the CPU busy, no bus traffic); 2 reads external
/// flash a cache line at a time over a range too big to cache; 3 reads
/// PSRAM (inside the package) the same way; 4 reads SRAM.
static BURST_KIND: AtomicU32 = AtomicU32::new(0);
static BURST_HZ: AtomicU32 = AtomicU32::new(66);
static BURST_N: AtomicU32 = AtomicU32::new(0);

/// Spin without touching memory, from RAM: no flash traffic at all.
#[link_section = ".data.bench_spin"]
#[inline(never)]
fn spin(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}

/// Read `lines` cache lines starting at `*at`, wrapping inside
/// `base..base+span`; runs from RAM so only the reads touch the bus.
#[link_section = ".data.bench_read"]
#[inline(never)]
fn read_lines(base: u32, span: u32, at: &mut u32, lines: u32) -> u32 {
    let mut sum = 0u32;
    for _ in 0..lines {
        // SAFETY: inside a mapped memory region; a volatile read.
        sum = sum.wrapping_add(unsafe { ((base + *at) as *const u32).read_volatile() });
        *at = (*at + 32) % span;
    }
    sum
}

#[embassy_executor::task]
pub async fn burst_task() {
    use embassy_time::{Duration, Timer};
    let (mut flash_at, mut psram_at, mut sram_at) = (0u32, 0u32, 0u32);
    loop {
        let kind = BURST_KIND.load(Ordering::Acquire);
        if kind == 0 {
            Timer::after(Duration::from_millis(50)).await;
            continue;
        }
        let hz = BURST_HZ.load(Ordering::Acquire).clamp(1, 2000);
        Timer::after(Duration::from_micros(u64::from(1_000_000 / hz))).await;
        let n = BURST_N.load(Ordering::Acquire);
        match kind {
            1 => spin(n),
            // 4 MB of the application window (erased past the image).
            2 => {
                read_lines(0x1202_0000, 0x40_0000, &mut flash_at, n);
            }
            3 => {
                read_lines(0x6010_0000, 0x40_0000, &mut psram_at, n);
            }
            _ => {
                read_lines(0x2004_0000, 0x2_0000, &mut sram_at, n);
            }
        }
    }
}
