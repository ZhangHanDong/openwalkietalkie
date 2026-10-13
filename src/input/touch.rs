//! Board wiring and asynchronous report delivery for the CST9220.
use super::{
    cst9220::{Cst9220, Report},
    touch_state::Contact,
};
use crate::{
    board::BusDevice,
    control::{stamp, Control, CONTROLS},
    logln,
};
use embassy_time::{with_timeout, Duration, Instant, Timer};
use sifli_hal::gpio::{Input, Output};

#[embassy_executor::task]
pub async fn task(bus: BusDevice, mut reset: Output<'static>, mut interrupt: Input<'static>) {
    let mut controller = Cst9220::new(bus);
    logln!(
        "touch: resetting CST9220 ({}, I2C 0x5A)",
        crate::board::TOUCH_WIRING
    );
    reset.set_low();
    Timer::after_millis(3).await;
    reset.set_high();
    Timer::after_millis(50).await;
    if let Err(error) = controller.enter_info_mode() {
        logln!("touch: information mode failed: {:?}", error);
        return;
    }
    Timer::after_millis(40).await;
    let identity = controller.identity();
    // Always leave diagnostic mode, including when identification fails.
    if let Err(error) = controller.enter_normal_mode() {
        logln!("touch: normal mode failed: {:?}", error);
        return;
    }
    match identity {
        Ok(id) if id.chip == 0x9220 && id.checkcode & 0xffff0000 == 0xcaca0000 => {
            logln!(
                "touch: CST{:04X}, project 0x{:04X}, check 0x{:08X}",
                id.chip,
                id.project,
                id.checkcode
            );
        }
        other => {
            logln!("touch: unexpected controller identity: {:?}", other);
            return;
        }
    }
    Timer::after_millis(5).await;
    logln!("touch: ready, interrupts with 20ms active / 50ms idle fallback");
    let mut contact = Contact::new();
    let mut last_report = Instant::now();
    let mut errors = 0;
    loop {
        // A short INT pulse can finish while a band is being rendered, even
        // before the first contact. Poll idle and active states to recover it.
        let fallback = if contact.active() {
            20
        } else {
            crate::power::IDLE_POLL_MS as u64
        };
        let _ = with_timeout(Duration::from_millis(fallback), interrupt.wait_for_low()).await;
        let event = match controller.read_report() {
            Ok(report) => {
                errors = 0;
                if report == Report::NotReady {
                    // Do not confuse a just-acknowledged buffer with finger-up.
                    // If reports stop entirely, clear the hold without activating
                    // ACCEPT/DECLINE from an unconfirmed release.
                    if contact.active() && last_report.elapsed() >= Duration::from_millis(120) {
                        logln!("touch: stale contact cancelled");
                        contact.cancel()
                    } else {
                        None
                    }
                } else {
                    last_report = Instant::now();
                    contact.sample(report)
                }
            }
            Err(error) => {
                errors += 1;
                logln!("touch: read/ack failed: {:?}", error);
                contact.cancel()
            }
        };
        if let Some(event) = event {
            // Await capacity so a release/cancellation is never silently lost.
            CONTROLS.send(stamp(Control::Touch(event))).await;
        }
        if errors >= 3 {
            logln!("touch: repeated bus errors; disabling touch until reboot");
            reset.set_low();
            return;
        }
        // Yield without the previous 10ms blind interval between reports.
        Timer::after_millis(1).await;
    }
}
