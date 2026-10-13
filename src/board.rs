//! Board bring-up and pin map, for the two boards this firmware runs on.
//!
//! The default build is the **LilyGo T-Display SF32** development board that
//! stood in for OpenWalkieTalkie hardware until our own board came back: SF32LB52x
//! (Cortex-M33), 16MB NOR, 8MB PSRAM, a 2.16" 480x480 CO5300 AMOLED with a
//! CST9220 touch controller, and three side buttons. The scroll wheel and the
//! six product buttons do not exist on it; the T-SF32-Keyboard expansion
//! stands in for them (see `input::keypad`).
//!
//! `--features hw-a` is the **OpenWalkieTalkie Rev A** main board (BodyV6.13 r6,
//! `hardware/rev-a/pcb-release-bodyv6.13-20260923-r6`): the same MCU class and
//! display, the panel mounted upside down (`orientation`), the six buttons and
//! the wheel on GPIOs (`input::gpio`), and a standalone charger the firmware
//! cannot talk to.
//!
//! `--features hw-b` is the **OpenWalkieTalkie Rev B** main board
//! (`hardware/rev-b`, the net list of record is `main/main-revb.kicad_pcb`):
//! Rev A's map with three pins moved, which is all the feature changes (it
//! implies `hw-a`). The speaker amplifier's CTRL is on `PA1` (`PA20` is
//! unconnected), the CAL test pad TP3 is on `PA0` (parked as a pull-down
//! input), and the BQ24073's CHG output reaches `PA33` through R46's 100k
//! pull-up, low while the cell charges, so Rev B has a charge state.
//!
//! LilyGo pins come from their board definition and SiFli SDK fork
//! (`Xinyuan-LilyGO/SlFli-SDK-Lilygo`) and were confirmed by the I2C probe at
//! boot. Rev A pins are the net list in `source/main/src/main.cohdl` of the r6
//! release, the single source of truth for that board.
//!
//! | Function              | LilyGo (default)                        | Rev A (`hw-a`)                          |
//! |-----------------------|-----------------------------------------|-----------------------------------------|
//! | Debug UART (USART1)   | RX `PA18` - TX `PA19`, 1 Mbaud          | same (TP6 / TP8; also SWDIO / SWCLK)    |
//! | I2C1 (touch)          | SCL `PA38` - SDA `PA37`                 | SCL `PA42` - SDA `PA41`, 10k pull-ups   |
//! | Assembly power hold   | `PA41` (`3V3_EN`, drive high)           | none (`PA41` is SDA)                    |
//! | Panel supply enable   | `PA44` (`LCD_VADD_EN`, drive high)      | `PA10` (U8 PANEL_4V, 3.99 V)            |
//! | Panel logic switch    | none, always on                         | `PA11` (U10 PANEL_LOGIC, 3.3 V)         |
//! | Display (LCDC1 QSPI)  | TE `PA2` CS `PA3` CLK `PA4` D0-D3 `PA5`-`PA8`, RST `PA9` | same                   |
//! | Touch (CST9220)       | on I2C1, INT `PA1` RST `PA0`            | on I2C1, INT `PA43` RST `PA44`          |
//! | Keypad (TCA8418)      | on I2C1, INT `PA11`                     | none                                    |
//! | Buttons               | keypad keys                             | PTT `PA24` ENTER `PA25` ESC `PA26` SHIFT `PA27` LEFT `PA37` RIGHT `PA38`, active low |
//! | Wheel                 | keypad up / down                        | A `PA39` B `PA40`, 12 detents per turn  |
//! | Charger               | SGM41562S on I2C1 (0x03)                | BQ24073, standalone, CHG on TP1 only; Rev B also on `PA33` |
//! | Side buttons          | A `PA34` B `PA35` C `PA33`              | none (`PA34` is the active-high reset)  |
//! | Microphone (PDM1)     | not used                                | CLK `PA22` DATA `PA23`, power `PA28`    |
//! | Speaker amplifier     | not used                                | NS4150B CTRL `PA20` (DAC out); Rev B `PA1` |
//!
//! The touch task owns the touch pins and performs the controller reset sequence.

use core::cell::RefCell;
use embassy_embedded_hal::shared_bus::blocking::i2c::I2cDevice;
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex};
use sifli_hal::adc::{self, Adc, Channel};
use sifli_hal::bind_interrupts;
use sifli_hal::gpio::{Input, Level, Output, Pull};
use sifli_hal::i2c::{self, I2c};
use sifli_hal::lcdc::InputColorFormat;
use sifli_hal::lcdc::{self, SpiConfig};
use sifli_hal::mode::{Async, Blocking};
use sifli_hal::peripherals::{I2C1, LCDC1};
use sifli_hal::rcc::{ConfigBuilder, Dll, DllStage, Sysclk};
use sifli_hal::time::{mhz, Hertz};
use sifli_hal::usart::{Config as UartConfig, Uart};

use crate::display::{Co5300, PanelRails, HEIGHT as LCD_H, WIDTH as LCD_W};
use crate::logln;

pub const NAME: &str = if cfg!(feature = "hw-b") {
    "OpenWalkieTalkie Rev B (BodyV6.13)"
} else if cfg!(feature = "hw-a") {
    "OpenWalkieTalkie Rev A (BodyV6.13 r6)"
} else {
    "LilyGo T-Display SF32 (dev)"
};

/// The touch controller's wiring, for the touch task's log line.
pub const TOUCH_WIRING: &str = if cfg!(feature = "hw-a") {
    "PA44 reset, PA43 interrupt"
} else {
    "PA0 reset, PA1 interrupt"
};

/// The stock bootloader talks at 1 Mbaud and so does `sftool`, so staying here
/// means the same terminal session shows both the flash and the firmware.
pub const LOG_BAUD: u32 = 1_000_000;

/// 400kHz: what LilyGo's SDK configures for this bus, and comfortably within
/// both the TCA8418's and the XL9555's ratings; the CST9220 on Rev A's bus is
/// rated the same.
pub const I2C_HZ: u32 = 400_000;

pub type Bus = I2c<'static, I2C1, Blocking>;
// All clients run on the same executor. Each blocking transaction releases
// the borrow before the task can yield; interrupts never access this bus.
pub type SharedBus = Mutex<NoopRawMutex, RefCell<Bus>>;
pub type BusDevice = I2cDevice<'static, NoopRawMutex, Bus>;
/// The keypad shim's error type; Rev A has no I2C input device.
#[cfg(not(feature = "hw-a"))]
pub type BusError = embassy_embedded_hal::shared_bus::I2cDeviceError<i2c::Error>;

bind_interrupts!(
    struct Irqs {
        LCDC1 => lcdc::InterruptHandler<LCDC1>;
        GPADC => adc::InterruptHandler;
    }
);

pub struct Board {
    pub radio: crate::link::RadioPeripherals,
    /// Rev A's USB-C data pair reaches the SF32's USB block on PA35 / PA36.
    #[cfg(feature = "hw-a")]
    pub usb: crate::link::usb::Peripherals,
    pub bluetooth_address: [u8; 6],
    pub bus: Bus,
    pub touch_reset: Output<'static>,
    pub touch_interrupt: Input<'static>,
    /// The dev board's keypad scanner interrupt.
    #[cfg(not(feature = "hw-a"))]
    pub keypad_interrupt: Input<'static>,
    /// Rev A's six buttons and the wheel.
    #[cfg(feature = "hw-a")]
    pub inputs: crate::input::gpio::Pins,
    /// Rev A's PDM microphone (power switch `PA28`, clock `PA22`, data
    /// `PA23`) and speaker amplifier (`PA20`; `PA1` on Rev B), for the BLE
    /// voice link.
    #[cfg(feature = "hw-a")]
    pub mic: crate::audio::pdm::Pdm,
    #[cfg(feature = "hw-a")]
    pub speaker: crate::audio::speaker::Speaker,
    /// Rev B: the BQ24073's CHG output (open drain, R46 100k to IO_3V3),
    /// low while the cell is charging.
    #[cfg(feature = "hw-b")]
    pub charge_status: Input<'static>,
    pub battery_adc: Adc<'static, Async>,
    pub battery_channel: Channel<'static>,
    pub battery_calibration: Option<crate::battery::state::Calibration>,
    /// `None` if the panel did not come up. The input path is useful without a
    /// screen, so a display failure is reported and stepped over rather than
    /// taking the firmware down with it.
    pub display: Option<Co5300>,
}

/// A Rev A button or encoder phase: the board pulls it up through 100k and
/// filters it with 1k + 1nF, so no internal pull, and a Schmitt input for
/// the slow edge of a contact.
#[cfg(feature = "hw-a")]
fn direct_input(pin: impl sifli_hal::Peripheral<P = impl sifli_hal::gpio::Pin> + 'static) -> Input<'static> {
    let mut input = Input::new(pin, Pull::None);
    input.set_schmitt(true);
    input
}

pub async fn init() -> Board {
    // 240MHz off DLL1: (stg + 1) * 24MHz, so Mul10 -> 240. The LCDC needs the
    // headroom to clock pixels out at 50MHz; the UART divisor is derived from
    // the running clock afterwards, so logging is unaffected. The HAL's
    // defaults also route this clock to the USB block divided by 4 (`usb:
    // true`, `usbsel: Sysclk`), the exact 60MHz `link::usb` needs: do not
    // add the HAL example's DLL2 route, which would re-trim DLL2 (the flash
    // and PSRAM clock) for nothing.
    let config = sifli_hal::Config::default().with_rcc(
        const {
            ConfigBuilder::new()
                .with_sys(Sysclk::Dll1)
                .with_dll1(Dll::new().with_stg(DllStage::Mul10))
                .checked()
        },
    );

    let p = sifli_hal::init(config);

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = LOG_BAUD;
    // Note the argument order: `new_blocking` takes (peri, rx, tx), and on both
    // boards PA18 is USART1_RXD and PA19 is USART1_TXD -- the reverse of what
    // the pin numbers suggest.
    let log =
        Uart::new_blocking(p.USART1, p.PA18, p.PA19, uart_config).expect("USART1 config rejected");

    // Logging comes up before anything else so the rest of bring-up can
    // narrate itself. On a board with no debug probe, a step that hangs is
    // otherwise indistinguishable from one that never ran at all.
    crate::logger::init(log);
    logln!();
    logln!("openwalkietalkie-fw {}", env!("CARGO_PKG_VERSION"));
    logln!("board: {}", NAME);

    #[cfg(not(feature = "hw-a"))]
    {
        logln!("board: holding power rail (PA41)");
        // PA41 is `3V3_EN`. Dropping this `Output` would release the latch and
        // cut power to the board mid-run, so the pin is deliberately leaked:
        // nothing ever wants it back.
        let power_hold = Output::new(p.PA41, Level::High);
        core::mem::forget(power_hold);
    }

    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = Hertz(I2C_HZ);
    #[cfg(not(feature = "hw-a"))]
    let bus = {
        logln!(
            "board: bringing up I2C1 (SCL PA38 / SDA PA37) at {} Hz",
            I2C_HZ
        );
        I2c::new_blocking(p.I2C1, p.PA38, p.PA37, i2c_config)
    };
    #[cfg(feature = "hw-a")]
    let bus = {
        logln!(
            "board: bringing up I2C1 (SCL PA42 / SDA PA41) at {} Hz",
            I2C_HZ
        );
        I2c::new_blocking(p.I2C1, p.PA42, p.PA41, i2c_config)
    };

    logln!(
        "board: bringing up LCDC1 QSPI + CO5300 ({}x{})",
        LCD_W,
        LCD_H
    );
    let lcdc_config = lcdc::Config {
        width: LCD_W,
        height: LCD_H,
        in_color_format: InputColorFormat::Rgb565,
        interface_config: SpiConfig {
            line_mode: lcdc::SpiLineMode::FourLine4Data,
            // The CO5300 only supports 50MHz in RGB565.
            write_frequency: lcdc::FrequencyConfig::Freq(mhz(50)),
            ..Default::default()
        },
        ..Default::default()
    };
    let lcdc = lcdc::Lcdc::new_qspi(
        p.LCDC1,
        Irqs,
        p.PA2,
        p.PA3,
        p.PA4,
        p.PA5,
        p.PA6,
        p.PA7,
        p.PA8,
        lcdc_config,
    );

    #[cfg(not(feature = "hw-a"))]
    let rails = {
        // LilyGo's BSP_LCD_PowerUp drives PA44 high to enable the panel supply.
        // PA41 alone only holds the assembly's 3V3 rail; LCDC transfers can
        // finish successfully while the display itself is still unpowered.
        logln!("board: enabling display supply (PA44)");
        PanelRails {
            power: Output::new(p.PA44, Level::High),
            logic: None,
        }
    };
    #[cfg(feature = "hw-a")]
    let rails = {
        // Rev A gates both panel rails: U10 switches PANEL_LOGIC (the panel's
        // 3.3 V VCI) on PA11 and U8 makes PANEL_4V (the AMOLED's VBAT, 3.99 V)
        // on PA10. Both have 1M pull-downs, so they are off until driven.
        // Logic first, then the 4 V rail, each given time to come up before
        // the reset sequence runs; the panel's IOVCC (IO_3V3) is always on.
        logln!("board: enabling panel rails (PA11 logic, then PA10 4V)");
        let logic = Output::new(p.PA11, Level::High);
        embassy_time::Timer::after_millis(5).await;
        let power = Output::new(p.PA10, Level::High);
        embassy_time::Timer::after_millis(20).await;
        PanelRails {
            power,
            logic: Some(logic),
        }
    };

    // Reset is PA9 on both boards. The upstream example uses PA0, which on the
    // dev board is the *touch* controller's reset -- driving that instead
    // leaves the panel held in reset and the screen stays black with no error.
    // Rev A pulls PANEL_RESET down through 100k, so the panel stays in reset
    // until this drives it.
    let reset = Output::new(p.PA9, Level::Low);

    let display = match Co5300::new(lcdc, reset, rails).await {
        Ok(display) => {
            logln!("board: display ready");
            Some(display)
        }
        Err(e) => {
            logln!("board: display init FAILED: {:?}", e);
            None
        }
    };

    logln!("board: ready");
    let mut adc_vref = None;
    let mut bluetooth_address = [0xc2, 0x53, 0x46, 0x32, 0x52, 0xc0];
    let battery_calibration = match sifli_hal::efuse::Efuse::new(p.EFUSEC) {
        Ok(efuse) => {
            let uid = efuse.uid();
            for (i, byte) in uid.bytes().iter().enumerate() {
                bluetooth_address[i % 6] ^= byte;
            }
            bluetooth_address[5] |= 0xc0; // stable random-static BLE address
            let c = efuse.calibration();
            sifli_hal::pmu::apply_calibration(c);
            let a = c.primary.low;
            logln!(
                "battery: ADC factory calibration {}/{}mV {}/{}mV VBAT {}/{}mV",
                a.adc_vol1_reg(),
                a.volt1_100mv() as u16 * 100,
                a.adc_vol2_reg(),
                a.volt2_100mv() as u16 * 100,
                a.vbat_reg(),
                a.vbat_volt_100mv() as u16 * 100
            );
            // The HAL uses 3.3V analog supply mode, which uses the primary
            // bank's points. Vol2 is for the separate 1.8V analog mode.
            if c.primary.high.vref_flag() {
                adc_vref = Some(c.primary.high.vref_reg());
            }
            crate::battery::state::Calibration::new(
                a.adc_vol1_reg(),
                a.volt1_100mv() as u16 * 100,
                a.adc_vol2_reg(),
                a.volt2_100mv() as u16 * 100,
                a.vbat_reg(),
                a.vbat_volt_100mv() as u16 * 100,
            )
        }
        Err(e) => {
            logln!("battery: ADC factory calibration unavailable: {:?}", e);
            None
        }
    };
    if battery_calibration.is_none() {
        logln!("battery: using nominal ADC conversion (uncalibrated)");
    }
    // Match HAL_ADC_SetFreq: the nominal ADC timings are specified at 36MHz,
    // so scale them to our PCLK instead of using the HAL's fixed defaults.
    let pclk_khz = sifli_hal::rcc::clocks().pclk.to_hertz().unwrap().0 / 1000;
    let mut adc_config = adc::Config::default();
    adc_config.conv_width = ((76 * pclk_khz + 18_000) / 36_000 - 1) as u8;
    adc_config.data_samp_dly = ((2 * pclk_khz + 18_000) / 36_000) as u8;
    adc_config.sample_width =
        (74 * pclk_khz + 18_000) / 36_000 - adc_config.data_samp_dly as u32 - 1;
    logln!(
        "battery: ADC PCLK {} kHz, timings {}/{}/{}",
        pclk_khz,
        adc_config.sample_width,
        adc_config.conv_width,
        adc_config.data_samp_dly
    );
    let battery_adc = Adc::new(p.GPADC, Irqs, adc_config);
    if let Some(vref) = adc_vref {
        sifli_hal::pac::GPADC
            .cfg_reg1()
            .modify(|w| w.set_anau_gpadc_ldovref_sel(vref));
    }
    // Rev B moved the CAL test pad (TP3, SiFli's calibration tool) to PA0:
    // parked as a pull-down input, as the native image does, so the pad has
    // a level between calibrations. Never handed back.
    #[cfg(feature = "hw-b")]
    core::mem::forget(Input::new(p.PA0, Pull::Down));
    Board {
        radio: crate::link::RadioPeripherals {
            lcpu: p.LCPU,
            mailbox: p.MAILBOX1_CH1,
            dma: p.DMAC2_CH8,
            rng: p.TRNG,
        },
        #[cfg(feature = "hw-a")]
        usb: crate::link::usb::Peripherals {
            usbc: p.USBC,
            dp: p.PA35,
            dm: p.PA36,
        },
        bluetooth_address,
        bus,
        #[cfg(not(feature = "hw-a"))]
        touch_reset: Output::new(p.PA0, Level::High),
        #[cfg(not(feature = "hw-a"))]
        touch_interrupt: Input::new(p.PA1, Pull::None),
        #[cfg(not(feature = "hw-a"))]
        keypad_interrupt: Input::new(p.PA11, Pull::Up),
        // Rev A: TOUCH_IRQ has its own 100k pull-up (R41) on the board.
        #[cfg(feature = "hw-a")]
        touch_reset: Output::new(p.PA44, Level::High),
        #[cfg(feature = "hw-a")]
        touch_interrupt: Input::new(p.PA43, Pull::None),
        #[cfg(feature = "hw-a")]
        inputs: crate::input::gpio::Pins {
            ptt: direct_input(p.PA24),
            enter: direct_input(p.PA25),
            cancel: direct_input(p.PA26),
            shift: direct_input(p.PA27),
            left: direct_input(p.PA37),
            right: direct_input(p.PA38),
            enc_a: direct_input(p.PA39),
            enc_b: direct_input(p.PA40),
        },
        #[cfg(feature = "hw-a")]
        mic: crate::audio::pdm::Pdm::new(p.DMAC1_CH2, p.PA28, p.PA22, p.PA23, p.PDM1),
        #[cfg(all(feature = "hw-a", not(feature = "hw-b")))]
        speaker: crate::audio::speaker::Speaker::new(p.AUDPRC, p.DMAC1_CH1, p.PA20),
        // Rev B: AMP_ENABLE moved to PA1 (PA20 is unconnected there), and
        // the charger's CHG output is readable.
        #[cfg(feature = "hw-b")]
        speaker: crate::audio::speaker::Speaker::new(p.AUDPRC, p.DMAC1_CH1, p.PA1),
        #[cfg(feature = "hw-b")]
        charge_status: Input::new(p.PA33, Pull::None),
        display,
        battery_adc,
        battery_channel: Channel::new_vbat(p.ADC_VBAT),
        battery_calibration,
    }
}
