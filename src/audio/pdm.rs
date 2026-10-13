//! Rev A's MP34DT05 PDM microphone on PDM1: clock PA22, data PA23, supply
//! switched by PA28 (a TPS22918 load switch with a 1M pull-down, so it is off
//! until driven). The pinned HAL has no PDM driver, so this is the SiFli SDK's
//! `HAL_PDM_Init` / `HAL_PDM_Receive_DMA` sequence on the PAC, with the
//! HAL's DMA ring buffer.
//!
//! The block decimates in hardware: the 9.6 MHz crystal-derived clock / 6 is
//! the microphone's 1.6 MHz bit clock, a third-order SINC divides by 50 and
//! the low-pass filter by 2, giving 16 kHz; a high-pass removes DC and a
//! 0.5 dB-step gain stage scales it. The CPU only drains DMA.
use embassy_time::Timer;
use sifli_hal::dma::{ReadableRingBuffer, Request, TransferOptions};
use sifli_hal::gpio::{Level, Output};
use sifli_hal::pac;
use sifli_hal::peripherals::{DMAC1_CH2, PA22, PA23, PA28, PDM1};

/// PDM1's left-channel FIFO (`DATA_L`); the PAC does not list it.
const DATA_L: *mut u32 = 0x5009_A03C as *mut u32;
/// The SDK uses 0x26 (+4 dB). The Rev A microphone peaks at about -23 dBFS
/// for speech at that gain (measured over HFP, 2026-10-03), so add 12 dB:
/// 0.5 dB a step.
const GAIN: u8 = 0x26 + 24;
/// The SINC and filter chain settles for about 60 ms after the clock starts
/// (the SDK drops the same six 10 ms frames).
const SETTLE_SAMPLES: usize = 960;
/// DMA ring: 640 words, woken every half (320 words). Two samples a word,
/// so 80 ms.
const RING_WORDS: usize = 640;
/// What an overrun costs: the HAL ring resets itself, dropping up to a
/// ring's worth of samples.
pub const RING_SAMPLES: usize = 2 * RING_WORDS;
/// Microphone supply settle before its clock starts (the native image's
/// 10 ms, from the board wrap of HAL_PDM_Init).
const POWER_SETTLE_MS: u64 = 10;

static mut RING: [u32; RING_WORDS] = [0; RING_WORDS];

// The FIFO packs two 16-bit samples into each 32-bit word DMA moves,
// `[second:hi16 | first:lo16]`: what the SDK's 640-byte, 320-sample buffers
// say, and what the first captures on Rev A showed (2026-10-04: words like
// `ff27ffe4`, two unrelated halves, and speech-band spectra when read so).

pub struct Pdm {
    dma: DMAC1_CH2,
    power: Output<'static>,
    _clock: PA22,
    _data: PA23,
    _block: PDM1,
}

impl Pdm {
    pub fn new(dma: DMAC1_CH2, power: PA28, clock: PA22, data: PA23, block: PDM1) -> Self {
        Self {
            dma,
            power: Output::new(power, Level::Low),
            _clock: clock,
            _data: data,
            _block: block,
        }
    }

    /// Power the microphone, configure the block and start DMA. The first
    /// `read` waits out the filter settle.
    pub async fn start(&mut self) -> Session<'_> {
        self.power.set_high();
        Timer::after_millis(POWER_SETTLE_MS).await;
        // PDM1 runs from the crystal; the audio buffer the SDK enables with
        // its audio clocks costs little and is not documented as unneeded.
        pac::PMUC.hxt_cr1().modify(|w| w.set_buf_aud_en(true));
        sifli_hal::rcc::enable_and_reset::<PDM1>();
        pins();
        let pdm = pac::PDM1;
        pdm.cfg0().modify(|w| w.set_pdmcoreen(false));
        pdm.cfg0().modify(|w| {
            w.set_left_en(true);
            w.set_right_en(false);
            w.set_stereo_en(false);
            w.set_swap_en(false);
            // 9.6 MHz crystal source / 6 = 1.6 MHz bit clock.
            w.set_clk_sel(false);
            w.set_clk_div(6);
        });
        pdm.sinc_cfg().modify(|w| {
            w.set_sinc_rate(50);
            w.set_sinc_order_sel(false);
        });
        // The SDK's reading of LPF_DS: 0 is "down-sample by 2".
        pdm.lpf_cfg6().modify(|w| {
            w.set_lpf_ds(false);
            w.set_lpf_bypass(false);
        });
        pdm.hpf_cfg().modify(|w| {
            w.set_hpf_bypass(false);
            w.set_hpf_coeff(7);
        });
        pdm.pga_cfg().modify(|w| {
            w.set_pga_gain_l(GAIN);
            w.set_pga_gain_r(GAIN);
        });
        // Left only, 16-bit words, the right channel's DMA request masked.
        pdm.fifo_cfg().modify(|w| {
            w.set_byte_con(false);
            w.set_byte_trunc(1);
            w.set_pdm_shift(0);
            w.set_rx_dma_msk_l(false);
            w.set_rx_dma_msk_r(true);
        });
        // SAFETY: RING is only ever lent to the one ring buffer alive at a
        // time, and `Session` borrows `self` mutably for its whole life.
        let buffer = unsafe { &mut *core::ptr::addr_of_mut!(RING) };
        let mut options = TransferOptions::default();
        options.priority = sifli_hal::dma::Priority::Low;
        // SAFETY: DATA_L is PDM1's receive FIFO, read as 32-bit words; the
        // block is configured above and enabled only after DMA is armed.
        let mut ring = unsafe { ReadableRingBuffer::new(&mut self.dma, Request::Pdm1L, DATA_L, buffer, options) };
        ring.start();
        pdm.cfg0().modify(|w| w.set_pdmcoreen(true));
        Session {
            ring,
            power: &mut self.power,
            discard: SETTLE_SAMPLES,
            words: [0; super::codec::FRAME_SAMPLES / 2],
        }
    }
}

/// Back to plain inputs with pull-downs (function 0), so neither pad is
/// driven or floating while the microphone is off.
fn release_pins() {
    use pac::hpsys_pinmux::vals::Ps;
    for pad in [22, 23] {
        pac::HPSYS_PINMUX.pad_pa0_38(pad).modify(|w| {
            w.set_fsel(0);
            w.set_pe(true);
            w.set_ps(Ps::Down);
            w.set_ie(false);
        });
    }
}

/// PA22 PDM1_CLK (no pull), PA23 PDM1_DATA (pull-down): function 3 on both,
/// written through the PAC because the HAL's alternate-function type is
/// crate-private.
fn pins() {
    use pac::hpsys_pinmux::vals::Ps;
    pac::HPSYS_PINMUX.pad_pa0_38(22).modify(|w| {
        w.set_fsel(3);
        w.set_pe(false);
        w.set_ie(true);
    });
    pac::HPSYS_PINMUX.pad_pa0_38(23).modify(|w| {
        w.set_fsel(3);
        w.set_pe(true);
        w.set_ps(Ps::Down);
        w.set_ie(true);
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// DMA overwrote samples before they were read.
    Overrun,
}

pub struct Session<'a> {
    ring: ReadableRingBuffer<'a, u32>,
    power: &'a mut Output<'static>,
    discard: usize,
    words: [u32; super::codec::FRAME_SAMPLES / 2],
}

impl Session<'_> {
    /// The next `out.len()` samples (an even count, at most one 20 ms
    /// frame), after the start-up settle. On `Overrun` the ring has already
    /// reset itself and the next read carries on with fresh samples.
    pub async fn read(&mut self, out: &mut [i16]) -> Result<(), Error> {
        while self.discard > 0 {
            let n = self.discard.min(out.len());
            self.fill(&mut out[..n]).await?;
            self.discard -= n;
        }
        self.fill(out).await
    }

    async fn fill(&mut self, out: &mut [i16]) -> Result<(), Error> {
        let words = &mut self.words[..out.len() / 2];
        self.ring.read_exact(words).await.map_err(|_| Error::Overrun)?;
        for (pair, word) in out.chunks_exact_mut(2).zip(words.iter()) {
            pair[0] = *word as u16 as i16;
            pair[1] = (*word >> 16) as u16 as i16;
        }
        Ok(())
    }

    /// Stop the block and DMA, cut the microphone's supply and gate the
    /// block's clock.
    pub async fn stop(mut self) {
        pac::PDM1.cfg0().modify(|w| w.set_pdmcoreen(false));
        self.ring.request_stop();
        while self.ring.is_running() {
            Timer::after_millis(1).await;
        }
        self.power.set_low();
        release_pins();
        sifli_hal::rcc::disable::<PDM1>();
        if !super::voice::PLAYING.load(core::sync::atomic::Ordering::Acquire) {
            pac::PMUC.hxt_cr1().modify(|w| w.set_buf_aud_en(false));
        }
    }
}
