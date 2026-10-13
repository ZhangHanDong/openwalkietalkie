//! The deck's vendor collection over the SF32's own USB port (Rev A).
//!
//! Rev A wires the USB-C data pair through U20 to PA35 / PA36 and has no
//! USB-serial bridge, so the cable carries the same HID device the Bluetooth
//! link does: VID 1209 / PID 5754, one vendor collection with report id 6 and
//! 63-byte input and output reports (`wire::REPORT_DESC`). `deckd` opens
//! whichever instance it finds and prefers this one; the hosts' lines are
//! dispatched and answered exactly as over BLE (`super::dispatch_frame`,
//! `super::transmit_task`), the only difference being that a USB report
//! travels with its report-id byte in both directions. The voice frames
//! that the audio service carries over Bluetooth travel here too, on
//! report channel 7 (`super::voice_report`, `super::send_voice`), so the
//! cable offers everything the radio does.
//!
//! The MCU's VBUS pin is not connected on Rev A. The HAL asserts the
//! session-valid condition in software, so D+ is pulled up from boot and the
//! device never sees a cable without a host; `Handler::configured` and
//! `suspended` are this link's up and down edges.
//!
//! The clock needs no change: `board::init`'s 240 MHz system clock already
//! feeds the USB block its 60 MHz (the HAL's defaults `usb: true`,
//! `usbsel: Sysclk`, divider 4), and `Driver::new` checks that.

use super::{
    dispatch_frame, link_down, link_up, reset_hosts, transmit_task, wire, Inbound, Link,
    LinkError, ReportSink,
};
use crate::logln;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_futures::{
    join::join3,
    select::{select, Either},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{with_timeout, Duration};
use embassy_usb::class::hid::{
    Config as HidConfig, HidReader, HidReaderWriter, HidWriter, ReadError, ReportId,
    RequestHandler, State,
};
use embassy_usb::control::OutResponse;
use embassy_usb::{Builder, Config, Handler};
use sifli_hal::usb::{Driver, InterruptHandler};
use sifli_hal::{bind_interrupts, peripherals};
use static_cell::{ConstStaticCell, StaticCell};

bind_interrupts!(struct Irqs { USBC => InterruptHandler<peripherals::USBC>; });

/// The USB block and its two PHY pads (alternate function `USB11_DP`/`DM`).
pub struct Peripherals {
    pub usbc: peripherals::USBC,
    pub dp: peripherals::PA35,
    pub dm: peripherals::PA36,
}

/// The same identity the BLE Device Information service advertises.
pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x5754;

type UsbDriver = Driver<'static, peripherals::USBC>;
const REPORT_LEN: usize = wire::REPORT_LEN;

/// Output reports that arrive through the control pipe (`SET_REPORT`). macOS
/// uses the interrupt OUT endpoint for a collection that has one; other hosts
/// may not, and both roads end in the same dispatcher.
static CONTROL_REPORTS: Channel<CriticalSectionRawMutex, [u8; REPORT_LEN], 4> = Channel::new();

struct Requests;

impl RequestHandler for Requests {
    fn set_report(&mut self, id: ReportId, data: &[u8]) -> OutResponse {
        let ReportId::Out(_) = id else {
            return OutResponse::Rejected;
        };
        // With report ids in use the data stage starts with the id; the
        // dispatcher accepts a frame with or without it.
        let mut frame = [0u8; REPORT_LEN];
        let n = data.len().min(frame.len());
        frame[..n].copy_from_slice(&data[..n]);
        if CONTROL_REPORTS.try_send(frame).is_err() {
            logln!("usb: control-pipe report dropped (queue full)");
        }
        OutResponse::Accepted
    }
}

/// The bus state, turned into this link's up and down edges.
struct DeviceState {
    configured: AtomicBool,
}

impl DeviceState {
    const fn new() -> Self {
        Self {
            configured: AtomicBool::new(false),
        }
    }
}

impl Handler for DeviceState {
    fn enabled(&mut self, enabled: bool) {
        self.configured.store(false, Ordering::Relaxed);
        if !enabled {
            link_down(Link::Usb);
        }
        logln!("usb: {}", if enabled { "enabled" } else { "disabled" });
    }

    fn reset(&mut self) {
        self.configured.store(false, Ordering::Relaxed);
        link_down(Link::Usb);
        logln!("usb: bus reset");
    }

    fn addressed(&mut self, addr: u8) {
        logln!("usb: address {}", addr);
    }

    fn configured(&mut self, configured: bool) {
        self.configured.store(configured, Ordering::Relaxed);
        if configured {
            logln!("usb: configured; a host is attached");
            link_up(Link::Usb);
        } else {
            logln!("usb: unconfigured");
            link_down(Link::Usb);
        }
    }

    fn suspended(&mut self, suspended: bool) {
        if suspended {
            logln!("usb: suspended");
            link_down(Link::Usb);
        } else if self.configured.load(Ordering::Relaxed) {
            logln!("usb: resumed");
            link_up(Link::Usb);
        }
    }
}

/// Serve the vendor collection for as long as the firmware runs. `address`
/// (the Bluetooth address) doubles as the serial number, so a host that
/// sees both instances can tell they are the same board.
#[embassy_executor::task]
pub async fn task(p: Peripherals, address: [u8; 6]) {
    static SERIAL: StaticCell<[u8; 12]> = StaticCell::new();
    let serial = core::str::from_utf8(SERIAL.init(hex(address))).unwrap_or("000000000000");

    let driver = Driver::new(p.usbc, Irqs, p.dp, p.dm);

    let mut config = Config::new(VID, PID);
    config.manufacturer = core::str::from_utf8(super::MANUFACTURER).ok();
    config.product = core::str::from_utf8(super::NAME).ok();
    config.serial_number = Some(serial);
    // The BLE PnP ID advertises release 1.1 as well.
    config.device_release = 0x0101;
    // One HID interface, so the class is declared per interface like every
    // USB HID device, not embassy-usb's composite-with-IADs default.
    config.device_class = 0;
    config.device_sub_class = 0;
    config.device_protocol = 0;
    config.composite_with_iads = false;
    // The host's power budget, not the board's draw: the BQ24073 charges at
    // up to 500 mA by its hardware strap whatever this says, and a 500 mA
    // request is refused (device left unconfigured) by a bus-powered hub or a
    // dock, so ask for what a hub always has.
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static DEVICE: StaticCell<DeviceState> = StaticCell::new();
    static REQUESTS: StaticCell<Requests> = StaticCell::new();
    static HID_STATE: StaticCell<State> = StaticCell::new();
    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 256]);
    #[cfg(feature = "usb-trace")]
    let config_descriptor_bytes: *const [u8; 256] = config_descriptor;
    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        BOS_DESCRIPTOR.init([0; 256]),
        &mut [],
        CONTROL_BUF.init([0; 64]),
    );
    builder.handler(DEVICE.init(DeviceState::new()));
    let hid = HidReaderWriter::<_, REPORT_LEN, REPORT_LEN>::new(
        &mut builder,
        HID_STATE.init(State::new()),
        HidConfig {
            report_descriptor: wire::REPORT_DESC,
            request_handler: Some(REQUESTS.init(Requests)),
            // 1 ms interrupt polling: the host's `deckd` drains its queue
            // every 2 ms and nothing here costs CPU while idle.
            poll_ms: 1,
            max_packet_size: REPORT_LEN as u16,
        },
    );
    let mut device = builder.build();
    let (mut reader, mut writer) = hid.split();
    #[cfg(feature = "usb-trace")]
    {
        // The configuration descriptor as the host will read it, for the
        // bring-up log. Read-only, after the builder has finished writing it.
        let bytes = unsafe { &*config_descriptor_bytes };
        let used = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        logln!("usb: configuration descriptor ({} bytes): {:02x?}", used, &bytes[..used]);
    }
    logln!(
        "usb: HID device on PA35/PA36 (VID 1209 PID 5754, serial {}); waiting for a host",
        serial
    );
    join3(device.run(), receive(&mut reader), send(&mut writer)).await;
}

/// Output reports from the host, by either road, into the shared dispatcher.
async fn receive(reader: &mut HidReader<'static, UsbDriver, REPORT_LEN>) {
    // 2 KB of assemblers: const-initialised in RAM rather than built on the
    // stack and moved.
    static INBOUND: ConstStaticCell<Inbound> = ConstStaticCell::new(Inbound::new());
    let inbound = INBOUND.take();
    let mut frame = [0u8; REPORT_LEN];
    loop {
        match select(reader.read(&mut frame), CONTROL_REPORTS.receive()).await {
            Either::First(Ok(n)) => dispatch_frame(Link::Usb, inbound, &frame[..n]).await,
            Either::First(Err(ReadError::Disabled)) => reader.ready().await,
            Either::First(Err(e)) => logln!("usb: read error: {:?}", e),
            Either::Second(control) => dispatch_frame(Link::Usb, inbound, &control).await,
        }
    }
}

/// This link's half of `transmit_task`, restarted after an overflow or a
/// failed write once the endpoint is enabled again.
async fn send(writer: &mut HidWriter<'static, UsbDriver, REPORT_LEN>) {
    loop {
        writer.ready().await;
        let mut sink = UsbSink { writer };
        transmit_task(Link::Usb, &mut sink).await;
    }
}

struct UsbSink<'a> {
    writer: &'a mut HidWriter<'static, UsbDriver, REPORT_LEN>,
}

impl ReportSink for UsbSink<'_> {
    /// The whole report, id first. The host controller polls the endpoint
    /// every millisecond while the device is configured, so a write that
    /// takes longer than this means the bus is suspended or gone.
    async fn send(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), LinkError> {
        match with_timeout(Duration::from_millis(250), self.writer.write(report)).await {
            Ok(result) => result.map_err(LinkError::Usb),
            Err(_) => Err(LinkError::Timeout),
        }
    }

    /// Nothing to disconnect: forget the hosts so their next `hello` or
    /// request starts a clean session.
    fn drop_link(&mut self) {
        reset_hosts(Link::Usb);
    }
}

fn hex(address: [u8; 6]) -> [u8; 12] {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 12];
    for (i, byte) in address.iter().enumerate() {
        out[2 * i] = DIGITS[usize::from(byte >> 4)];
        out[2 * i + 1] = DIGITS[usize::from(byte & 15)];
    }
    out
}
