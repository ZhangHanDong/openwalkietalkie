//! OpenWalkieTalkie firmware.
//!
//! Runs on the LilyGo T-Display SF32 development board by default, on the
//! OpenWalkieTalkie Rev A board with `--features hw-a` and on Rev B with
//! `--features hw-b` (Rev A's map with three pins moved; it implies `hw-a`,
//! so `cfg(feature = "hw-a")` means "our main board" throughout). See `board`
//! for the pin maps, `input::keypad` for the dev board's stand-in wiring of the scroll
//! wheel and buttons, `input::gpio` for the real ones, and `ui` for the screen.

#![no_std]
#![no_main]

mod app;
mod audio;
mod battery;
mod board;
mod link;
mod control;
// `deck` and `ui` are shared trees also mounted by the native core, the host
// tests and the preview; each consumer uses a different subset of them.
#[allow(dead_code)]
mod deck;
mod display;
mod input;
mod logger;
#[path = "display/orientation.rs"]
mod orientation;
#[cfg(feature = "hw-a")]
mod pmu;
mod power;
#[allow(dead_code)]
mod ui;
mod xip;

// Linked for `probe-rs attach`, which is the only way to see a panic message on
// this board. Day-to-day logging goes out of UART1 instead; see `logger`.
use defmt_rtt as _;
use panic_probe as _;

/// Bench builds: a fault (a panic ends in one) reboots into the USB recovery
/// bootloader instead of hanging, so tools/flash-usb.py can replace a bad
/// image without anyone at the board.
#[cfg(all(feature = "hw-a", feature = "audio-debug"))]
#[cortex_m_rt::exception]
unsafe fn HardFault(_: &cortex_m_rt::ExceptionFrame) -> ! {
    pmu::reboot(true)
}

use core::cell::RefCell;
use embassy_executor::Spawner;
use static_cell::StaticCell;

use crate::display::Band;

/// The band framebuffer is 45KB -- far too large to build on the stack and hand
/// to a `StaticCell`, which would need it as a temporary first. Declared const
/// here it costs nothing in flash and lands in `.bss`.
static mut BAND: Band = Band::new();
static I2C_BUS: StaticCell<board::SharedBus> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // What the PMU recorded about this start, read before the HAL's clock
    // setup rewrites its control register; logged once the UART is up.
    #[cfg(feature = "hw-a")]
    let boot = pmu::boot_reason();
    // `board::init` brings the log UART up first and narrates the rest.
    let mut board = board::init().await;
    #[cfg(feature = "hw-a")]
    logln!(
        "boot: {}; chip revision {:?}",
        boot,
        sifli_hal::syscfg::read_idr().revision()
    );
    // A wake from the soft power-off must be a deliberate one: PTT held for
    // `pmu::POWER_ON_HOLD_MS`, else straight back to sleep with the panel
    // still dark (never returns in that case).
    #[cfg(feature = "hw-a")]
    app::power_on_gate(boot, &board.inputs.ptt, &mut board.display).await;
    // Taken exactly once and reused for both the splash and normal UI.
    let band = unsafe { &mut *core::ptr::addr_of_mut!(BAND) };
    if let Some(panel) = board.display.as_mut() {
        if let Err(e) = app::render_splash(panel, band).await {
            logln!("boot: splash render failed: {:?}", e);
        }
        if let Err(e) = panel.show().await {
            logln!("boot: display-on failed: {:?}", e);
        }
        logln!("boot: OpenWalkieTalkie splash ready");
        embassy_time::Timer::after_millis(ui::splash::DURATION_MS as u64).await;
    }

    // Probing before the keyboard board is powered gives the probe in the
    // keypad task something to be compared against.
    #[cfg(not(feature = "hw-a"))]
    {
        logln!("boot: I2C1 before keyboard power-up:");
        input::probe_known(&mut board.bus);
    }
    #[cfg(feature = "hw-a")]
    logln!("boot: Rev A: no keyboard board; the buttons and wheel are direct GPIOs");

    let bus = I2C_BUS.init(board::SharedBus::new(RefCell::new(board.bus)));
    #[cfg(not(feature = "hw-a"))]
    spawner
        .spawn(input::keypad::task(
            board::BusDevice::new(bus),
            board.keypad_interrupt,
        ))
        .unwrap();
    #[cfg(feature = "hw-a")]
    spawner.spawn(input::gpio::task(board.inputs)).unwrap();
    spawner
        .spawn(battery::task(
            board.battery_adc,
            board.battery_channel,
            board.battery_calibration,
            board::BusDevice::new(bus),
            #[cfg(feature = "hw-b")]
            board.charge_status,
        ))
        .unwrap();
    spawner
        .spawn(input::touch::task(
            board::BusDevice::new(bus),
            board.touch_reset,
            board.touch_interrupt,
        ))
        .unwrap();
    spawner
        .spawn(link::task(board.radio, board.bluetooth_address))
        .unwrap();
    // Rev A only: the dev board's USB-C goes to its CH343 bridge, and PA35 is
    // one of its side buttons.
    #[cfg(feature = "hw-a")]
    spawner
        .spawn(link::usb::task(board.usb, board.bluetooth_address))
        .unwrap();
    // Rev A's voice link: the microphone and speaker ride the BLE audio
    // service (`link`), not a Bluetooth audio profile.
    #[cfg(feature = "hw-a")]
    {
        spawner.spawn(audio::mic::task(board.mic)).unwrap();
        spawner.spawn(audio::speaker::task(board.speaker)).unwrap();
        #[cfg(feature = "audio-debug")]
        spawner.spawn(link::debug::burst_task()).unwrap();
    }

    app::run(board.display, band).await
}
