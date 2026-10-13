//! Line logging over the board's debug UART.
//!
//! This board has no debug probe wired: the only wire out of it is the CH343
//! USB-serial bridge, which is the same port `sftool` flashes through. So the
//! bring-up log goes out of UART1 rather than through defmt/RTT, and can be
//! read with any terminal at 1 Mbaud right after a flash.
//!
//! The UART is parked in a global so any task can log without threading a
//! handle through every call. It is blocking and inside a critical section, so
//! a log line at 1 Mbaud costs roughly 10us per character -- fine for bring-up
//! and status, not for anything in a tight loop.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embedded_io::Write;
use sifli_hal::mode::Blocking;
use sifli_hal::peripherals::USART1;
use sifli_hal::usart::Uart;

pub type LogUart = Uart<'static, USART1, Blocking>;

static LOG: Mutex<CriticalSectionRawMutex, RefCell<Option<LogUart>>> =
    Mutex::new(RefCell::new(None));

/// Adopt the UART. Lines logged before this call are dropped.
pub fn init(uart: LogUart) {
    LOG.lock(|slot| *slot.borrow_mut() = Some(uart));
    let _ = log::set_logger(&RADIO_LOG);
    log::set_max_level(if cfg!(feature = "usb-trace") {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    });
}

struct RadioLog;
static RADIO_LOG: RadioLog = RadioLog;
impl log::Log for RadioLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        // The stack's info-level identity logs include the peer's IRK.
        // Pairing outcomes are logged by our transport without key material.
        let limit = if metadata
            .target()
            .starts_with("trouble_host::security_manager")
        {
            log::Level::Warn
        } else if cfg!(feature = "usb-trace") && metadata.target().starts_with("embassy_usb") {
            log::Level::Trace
        } else {
            log::Level::Info
        };
        metadata.level() <= limit
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            write_line(format_args!("{}: {}", record.target(), record.args()));
        }
    }
    fn flush(&self) {}
}

#[doc(hidden)]
pub fn write_line(args: core::fmt::Arguments) {
    LOG.lock(|slot| {
        if let Some(uart) = slot.borrow_mut().as_mut() {
            let _ = uart.write_fmt(args);
            // Terminals expect CRLF; `writeln!` would only give us LF.
            let _ = uart.write_all(b"\r\n");
        }
    });
}

/// Log one line to the debug UART. Same formatting as `println!`.
#[macro_export]
macro_rules! logln {
    () => { $crate::logger::write_line(format_args!("")) };
    ($($arg:tt)*) => { $crate::logger::write_line(format_args!($($arg)*)) };
}
