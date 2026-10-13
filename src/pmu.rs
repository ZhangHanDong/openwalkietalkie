//! Soft power-off for Rev A: the SF32LB52x PMU's hibernate mode (shutdown,
//! when its 10 kHz RC is running) with PTT armed as the wake pin, and the
//! matching boot-reason check.
//!
//! The pinned HAL has no low-power API, so this follows the SiFli SDK's
//! `HAL_PMU_EnterShutdown` (drivers/hal/bf0_hal_pmu.c) step by step through
//! the PAC, with raw pointers for the RTC block the PAC leaves out. The chip
//! has two deep-sleep wake slots (PMUC `PIN0`/`PIN1`), each selecting one
//! pad of PA24..PA44; `WAKE` uses slot 0 for PTT and leaves slot 1 disarmed,
//! so nothing else wakes the deck. A wake is a cold boot through the ROM and
//! the SiFli bootloader; `boot_reason` reads and clears what the PMU
//! recorded, and `app::power_on_gate` then insists on PTT staying down for
//! `POWER_ON_HOLD_MS` before the deck counts as switched on.
//!
//! Everything from the LDO2 switch-off onwards runs from RAM (`enter`): on
//! pre-A4 silicon that LDO powers the flash, which the SDK drops so the part
//! comes back in 3-byte address mode for the bootloader.
//!
//! `reboot` is the same tail ending in the PMU's REBOOT bit (the SDK's
//! `HAL_PMU_Reboot`); with `recovery` it first leaves the bootloader's
//! request word in RTC backup register 7, and the OpenWalkieTalkie bootloader
//! (native/bootloader) then stays in its USB DFU recovery mode.
use core::arch::asm;
use core::fmt;

use sifli_hal::pac::hpsys_rcc::vals::mux::Wdtsel;
use sifli_hal::pac::{lpsys_rcc, pmuc, HPSYS_AON, HPSYS_CFG, LPSYS_AON, LPSYS_RCC, PMUC};

/// PMUC wake slot 0: the pad's AON wake index (PA number - 24) and the
/// control on it. PA24 (PTT) is detected by its RTC-domain PBR pad, one of
/// the four the SDK itself uses for hibernate wake. CANCEL used to be slot
/// 1; it is disarmed so a pocketed deck cannot come on by its back button.
const WAKE: (u8, &str) = (24 - 24, "PTT");
pub const WAKE_BUTTONS: &str = "PTT held 2 s";
/// How long PTT must stay down after the application starts for a wake to
/// become a power-on (`app::power_on_gate`); a shorter press goes back to
/// sleep with the panel still dark. The ROM and bootloader add their own
/// few hundred milliseconds before this clock starts.
pub const POWER_ON_HOLD_MS: u32 = 2_000;

/// Falling edge: the buttons idle high on the board's 100k pull-ups, and a
/// PTT that is still down when the PMU is armed must not wake it at once (a
/// level trigger would).
const PIN_MODE_NEG_EDGE: u8 = 3;

// The RTC block is not in the PAC. PBR0R..PBR3R are the RTC-domain pad
// registers of PA24..PA27 (`hwp_pbr` in the SDK aliases them).
const RTC_BASE: usize = 0x500C_B000;
const RTC_ISR: *mut u32 = (RTC_BASE + 0x0C) as *mut u32;
const RTC_BKP0R: *mut u32 = (RTC_BASE + 0x30) as *mut u32;
/// BKP7R: the bootloader's recovery request (the SDK labels it BT_TXPWR and
/// never writes it on this chip).
const RTC_BKP7R: *mut u32 = (RTC_BASE + 0x4C) as *mut u32;
const RTC_PBR0R: *mut u32 = (RTC_BASE + 0x5C) as *mut u32;
/// "OWTR": what native/bootloader/owt/recovery.c looks for.
const RECOVERY_MAGIC: u32 = 0x4F57_5452;

/// What `enter` does once the flash may be unpowered.
const MODE_HIBERNATE: u32 = 0;
const MODE_SHUTDOWN: u32 = 1;
const MODE_REBOOT: u32 = 2;
const RTC_ISR_INIT: u32 = 1 << 10;
const PBR_OE: u32 = 1 << 1;
const PBR_IE: u32 = 1 << 2;
const PBR_PE: u32 = 1 << 3;
const PBR_PS: u32 = 1 << 4;

/// Why the chip started, as recorded by the PMU.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boot {
    Cold,
    Reboot,
    /// Woke from a soft power-off. `wsr` is the PMU's wake status word
    /// (bit 3 = slot 0, bit 4 = slot 1, bit 7 = low battery).
    Wake { wsr: u32, shutdown: bool },
}

impl fmt::Display for Boot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Boot::Cold => f.write_str("cold start"),
            Boot::Reboot => f.write_str("software reboot"),
            Boot::Wake { wsr, shutdown } => {
                let by = if wsr & (1 << 3) != 0 {
                    WAKE.1
                } else {
                    "the PMU"
                };
                write!(
                    f,
                    "woke from {} by {} (wake flags 0x{:03X})",
                    if shutdown { "shutdown" } else { "hibernate" },
                    by,
                    wsr
                )
            }
        }
    }
}

/// The PMU state `power_off` enters: shutdown while the 10 kHz RC runs (the
/// ROM's default, which the HAL never disables), else hibernate.
pub fn mode() -> &'static str {
    if PMUC.lrc10_cr().read().en() {
        "shutdown"
    } else {
        "hibernate"
    }
}

/// Read and clear the PMU's record of why the chip started. Call once, before
/// the HAL's clock setup rewrites `PMUC.CR` (the SDK's `HAL_PMU_CheckBootMode`).
pub fn boot_reason() -> Boot {
    let cr = PMUC.cr().read();
    if cr.hiber_en() {
        // Shutdown left the PMU on the 10 kHz RC and the RTC stopped; either
        // mark will do, in case a bootloader touched the clock select.
        // SAFETY: a plain register of the always-on RTC block.
        let isr = unsafe { RTC_ISR.read_volatile() };
        let shutdown = cr.sel_lpclk() == Wdtsel::Lrc10 || isr & RTC_ISR_INIT != 0;
        if isr & RTC_ISR_INIT != 0 {
            // SAFETY: as above; restarts the RTC the shutdown stopped.
            unsafe { RTC_ISR.write_volatile(isr & !RTC_ISR_INIT) };
        }
        PMUC.cr().modify(|w| w.set_hiber_en(false));
        let wsr = PMUC.wsr().read().0;
        PMUC.wcr().write_value(pmuc::regs::Wcr(wsr));
        Boot::Wake { wsr, shutdown }
    } else if cr.reboot() {
        PMUC.cr().modify(|w| w.set_reboot(false));
        Boot::Reboot
    } else {
        Boot::Cold
    }
}

/// Power the deck off. Never returns: the chip sits in hibernate until a
/// wake pin fires, then boots cold through the bootloader.
pub fn power_off() -> ! {
    cortex_m::interrupt::disable();
    // Letter-series silicon (A4/B4) keeps LDO2: the SDK skips that step there.
    let letter = sifli_hal::syscfg::read_idr().revision().is_letter_series();
    let hclk = sifli_hal::rcc::clocks().hclk.to_hertz().map_or(48_000_000, |hz| hz.0);
    arm_wake_pins();
    // Drop the D+ pull-up: the host sees a detach, not a dead device.
    HPSYS_CFG.usbcr().modify(|w| {
        w.set_dp_en(false);
        w.set_usb_en(false);
    });
    halt_lcpu();
    // VBAT_LDO down to 3.0 V (SDK: no leakage path once VCC may sit below 3.3 V).
    PMUC.aon_ldo().modify(|w| w.set_vbat_ldo_set_vout(0));
    // Shutdown differs from hibernate only by running the PMU from the 10 kHz
    // RC with the RTC stopped; without that RC the chip hibernates instead.
    let mode = if PMUC.lrc10_cr().read().en() {
        MODE_SHUTDOWN
    } else {
        MODE_HIBERNATE
    };
    // SAFETY: interrupts are off, the LCPU is parked, and `enter` runs from
    // RAM with nothing to call.
    unsafe { enter(letter, mode, turns_per_ms(hclk)) }
}

/// Reboot the chip (the SDK's `HAL_PMU_Reboot`). With `recovery`, the
/// bootloader finds the request word and stays in USB DFU recovery; without
/// it the application starts again. Never returns.
pub fn reboot(recovery: bool) -> ! {
    cortex_m::interrupt::disable();
    if recovery {
        // SAFETY: a backup register of the always-on RTC block.
        unsafe { RTC_BKP7R.write_volatile(RECOVERY_MAGIC) };
    }
    let letter = sifli_hal::syscfg::read_idr().revision().is_letter_series();
    let hclk = sifli_hal::rcc::clocks().hclk.to_hertz().map_or(48_000_000, |hz| hz.0);
    // Drop the D+ pull-up: the host sees a detach before the new identity.
    HPSYS_CFG.usbcr().modify(|w| {
        w.set_dp_en(false);
        w.set_usb_en(false);
    });
    halt_lcpu();
    // SAFETY: as in `power_off`.
    unsafe { enter(letter, MODE_REBOOT, turns_per_ms(hclk)) }
}

/// `spin` turns per millisecond, budgeted at three cycles per turn (it may
/// run at two: every wait in `enter` is doubled against the SDK's).
fn turns_per_ms(hclk: u32) -> u32 {
    (hclk / 1000 / 3).max(1)
}

/// Route the wake pad to PMUC slot 0 (`HAL_PMU_SelectWakeupPin` /
/// `HAL_PMU_EnablePinWakeup`): the PBR pad is made an input without internal
/// pull, a stale flag is cleared (write 1), the edge is set and the slot
/// enabled; slot 1 is disabled and its flag cleared, in case an older image
/// left CANCEL armed there. Nothing else re-arms them: the HAL's `Input`
/// setup only touches the HPSYS pad registers, which power down with the
/// core.
fn arm_wake_pins() {
    let (index, _) = WAKE;
    if index <= 3 {
        let reg = RTC_PBR0R.wrapping_add(index as usize);
        // SAFETY: PBR0R..PBR3R of the always-on RTC block.
        unsafe {
            let v = reg.read_volatile();
            reg.write_volatile((v & !(PBR_OE | PBR_PE | PBR_PS)) | PBR_IE);
        }
    }
    PMUC.cr().modify(|w| {
        w.set_pin0_sel(index);
        w.set_pin0_mode(PIN_MODE_NEG_EDGE);
    });
    PMUC.wer().modify(|w| w.set_pin1(false));
    PMUC.wcr().write(|w| {
        w.set_pin0(true);
        w.set_pin1(true);
    });
    PMUC.wer().modify(|w| w.set_pin0(true));
    // What `HAL_PMU_Init` programs; the field is otherwise undocumented.
    PMUC.wkup_cnt().write_value(pmuc::regs::WkupCnt(0x0001_0001));
}

/// Wake the LCPU (the Bluetooth controller) so it can be reset and parked,
/// as the SDK does before any PMU mode. Every wait is bounded: a dead
/// controller must not hang the power-off.
fn halt_lcpu() {
    HPSYS_AON.issr().modify(|w| w.set_hp2lp_req(true));
    sifli_hal::cortex_m_blocking_delay_us(230);
    wait_for(|| HPSYS_AON.issr().read().lp_active(), 10_000);
    sifli_hal::cortex_m_blocking_delay_us(30);
    wait_for(|| HPSYS_AON.issr().read().lp_active(), 10_000);
    if LPSYS_AON.pmr().read().cpuwait() {
        return;
    }
    LPSYS_AON.pmr().modify(|w| w.set_cpuwait(true));
    LPSYS_RCC.rstr1().write_value(lpsys_rcc::regs::Rstr1(0xFFFF_FFFF));
    wait_for(|| LPSYS_RCC.rstr1().read().0 != 0, 10_000);
    if LPSYS_AON.slp_ctrl().read().sleep_status() {
        LPSYS_AON.slp_ctrl().modify(|w| w.set_wkup_req(true));
        wait_for(|| !LPSYS_AON.slp_ctrl().read().sleep_status(), 10_000);
    }
    LPSYS_RCC.rstr1().write_value(lpsys_rcc::regs::Rstr1(0));
}

fn wait_for(done: impl Fn() -> bool, limit_us: u32) {
    let mut waited = 0;
    while !done() && waited < limit_us {
        sifli_hal::cortex_m_blocking_delay_us(10);
        waited += 10;
    }
}

/// About three cycles per turn from zero-wait RAM.
#[inline(always)]
fn spin(turns: u32) {
    let mut n = turns.max(1);
    // SAFETY: a counted loop on one register; touches no memory.
    unsafe {
        asm!(
            "1:",
            "subs {0}, {0}, #1",
            "bne 1b",
            inout(reg) n,
            options(nomem, nostack),
        );
    }
    let _ = n;
}

/// The SDK's `HAL_PMU_EnterShutdown` (or `HAL_PMU_Reboot`) tail, from RAM,
/// with no calls out. The flash may lose power at the first step, so every
/// register is a literal address (PMUC 0x500CA000, HPSYS_RCC 0x50000000,
/// HPSYS_AON 0x500C0000).
#[link_section = ".data.pmu_enter"]
#[inline(never)]
unsafe fn enter(letter: bool, mode: u32, turns_per_ms: u32) -> ! {
    const PMUC_CR: *mut u32 = 0x500C_A000 as *mut u32;
    const PMUC_WSR: *mut u32 = 0x500C_A008 as *mut u32;
    const PMUC_WCR: *mut u32 = 0x500C_A00C as *mut u32;
    const PMUC_LXT_CR: *mut u32 = 0x500C_A020 as *mut u32;
    const PMUC_PERI_LDO: *mut u32 = 0x500C_A05C as *mut u32;
    const PMUC_HRC_CR: *mut u32 = 0x500C_A074 as *mut u32;
    const HPSYS_RCC_CSR: *mut u32 = 0x5000_0020 as *mut u32;
    const HPSYS_AON_ACR: *mut u32 = 0x500C_0010 as *mut u32;
    const CR_SEL_LPCLK: u32 = 1 << 0;
    const CR_HIBER_EN: u32 = 1 << 1;
    const CR_REBOOT: u32 = 1 << 2;

    if !letter {
        // LDO2 off (EN bit 8 clear, PD bit 13 set), then the SDK's 5 ms.
        let v = PMUC_PERI_LDO.read_volatile();
        PMUC_PERI_LDO.write_volatile((v & !(1 << 8)) | (1 << 13));
        spin(10 * turns_per_ms);
    }
    // The SDK's 8 ms.
    spin(16 * turns_per_ms);
    // Off the 48 MHz RC before its trim is reset: run from the crystal.
    if HPSYS_RCC_CSR.read_volatile() & 0x3 == 0 {
        HPSYS_AON_ACR.write_volatile(HPSYS_AON_ACR.read_volatile() | (1 << 1));
        while HPSYS_AON_ACR.read_volatile() & (1 << 31) == 0 {}
        let csr = HPSYS_RCC_CSR.read_volatile();
        HPSYS_RCC_CSR.write_volatile((csr & !0x3) | 0x1);
    }
    // RC48 back to its default trim for the ROM.
    PMUC_HRC_CR.write_volatile(0x0A50_C015);
    // Flash power-cycle delays for the bootloader (5 ms down, 5 ms up).
    let bkp = RTC_BKP0R.read_volatile();
    if bkp & 0x00FF_FFF0 == 0 {
        RTC_BKP0R.write_volatile(bkp | (5 << 4) | (5 << 12));
    }
    // Clear every pending wake flag (write 1 to clear).
    PMUC_WCR.write_volatile(PMUC_WSR.read_volatile());
    if mode == MODE_REBOOT {
        // A stale REBOOT bit is cleared first (the SDK's 100 ms), then set.
        let cr = PMUC_CR.read_volatile();
        if cr & CR_REBOOT != 0 {
            PMUC_CR.write_volatile(cr & !CR_REBOOT);
            spin(200 * turns_per_ms);
        }
        PMUC_CR.write_volatile(PMUC_CR.read_volatile() | CR_REBOOT);
        loop {
            asm!("wfi", options(nomem, nostack));
        }
    }
    if mode == MODE_SHUTDOWN {
        PMUC_CR.write_volatile(PMUC_CR.read_volatile() & !CR_SEL_LPCLK);
        // The SDK's 100 us for the clock switch.
        spin(turns_per_ms);
        PMUC_LXT_CR.write_volatile(PMUC_LXT_CR.read_volatile() & !0x3);
        RTC_ISR.write_volatile(RTC_ISR.read_volatile() | RTC_ISR_INIT);
    }
    // A clean 0 -> 1 on HIBER_EN is what the analog detects: low for the
    // SDK's 100 ms first.
    let cr = PMUC_CR.read_volatile() & !CR_HIBER_EN;
    PMUC_CR.write_volatile(cr);
    spin(200 * turns_per_ms);
    PMUC_CR.write_volatile(cr | CR_HIBER_EN);
    loop {
        asm!("wfi", options(nomem, nostack));
    }
}
