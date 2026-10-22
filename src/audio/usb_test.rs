//! Read-only counters for the opt-in USB sample-rate experiment.
use core::sync::atomic::{AtomicU32, Ordering};

pub static STARTS: AtomicU32 = AtomicU32::new(0);
pub static ENDED: AtomicU32 = AtomicU32::new(0);
pub static RATE: AtomicU32 = AtomicU32::new(0);
pub static FRAMES: AtomicU32 = AtomicU32::new(0);
pub static SAMPLES: AtomicU32 = AtomicU32::new(0);
pub static GAPS: AtomicU32 = AtomicU32::new(0);
pub static UNDERRUNS: AtomicU32 = AtomicU32::new(0);
pub static QUEUE_DROPS: AtomicU32 = AtomicU32::new(0);
pub static SEQ_ERRORS: AtomicU32 = AtomicU32::new(0);
pub static PLAYING: AtomicU32 = AtomicU32::new(0);

pub fn begin(rate_khz: u8) {
    RATE.store(u32::from(rate_khz) * 1000, Ordering::Relaxed);
    STARTS.fetch_add(1, Ordering::Relaxed);
    PLAYING.store(1, Ordering::Relaxed);
}

pub fn decoded(samples: usize, seq_ok: bool) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    SAMPLES.fetch_add(samples as u32, Ordering::Relaxed);
    if !seq_ok { SEQ_ERRORS.fetch_add(1, Ordering::Relaxed); }
}

pub fn queue_drop() { QUEUE_DROPS.fetch_add(1, Ordering::Relaxed); }

pub fn finish(gaps: u32, underruns: u32, ended: bool) {
    GAPS.fetch_add(gaps, Ordering::Relaxed);
    UNDERRUNS.fetch_add(underruns, Ordering::Relaxed);
    ENDED.fetch_add(u32::from(ended), Ordering::Relaxed);
    PLAYING.store(0, Ordering::Relaxed);
}
