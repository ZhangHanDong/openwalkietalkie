//! Lock-free UI → local speaker command. Host speech always has priority.
use core::sync::atomic::{AtomicU32, Ordering};
pub static CHANGED: embassy_sync::signal::Signal<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    (),
> = embassy_sync::signal::Signal::new();
static COMMAND: AtomicU32 = AtomicU32::new(0);
pub fn publish(word: u32) {
    if COMMAND.swap(word, Ordering::AcqRel) != word {
        CHANGED.signal(());
    }
}
pub fn current() -> u32 {
    COMMAND.load(Ordering::Acquire)
}
