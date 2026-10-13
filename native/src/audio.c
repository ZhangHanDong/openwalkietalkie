/* SPDX-License-Identifier: MIT */
#include <rtthread.h>
#include <rthw.h>
#include <stdint.h>
#include "audioproc.h"
#include <rtdevice.h>
#include "board.h"
#include "drv_audprc.h"
#include "speaker_pcm.h"

static volatile uint32_t frames, samples, peak;
static volatile uint8_t muted;
static volatile uint32_t playback_frames, playback_samples, playback_peak;
static volatile uint8_t speaker_on, speaker_stereo;
/* The pinned audio server writes 320-byte voice or 960-byte music DMA blocks.
 * Space also covers its 480/1440-byte super-wideband configuration. Only the
 * SDK's speaker worker calls the DMA write hook, which copies synchronously. */
static uint8_t speaker_pcm[1440];

/* The NS4150B's CTRL pin: PA20 on Rev A, PA1 on Rev B (src/board.rs). */
#ifdef OWT_BOARD_HW_B
#define OWT_AMP_PIN 1
#else
#define OWT_AMP_PIN 20
#endif

void audio_hardware_pa_init(void)
{
    rt_pin_mode(OWT_AMP_PIN, PIN_MODE_OUTPUT);
    rt_pin_write(OWT_AMP_PIN, PIN_LOW);
    speaker_on = 0;
}

void audio_hardware_pa_start(uint32_t samplerate, uint32_t reserved)
{
    (void)reserved;
    /* NS4150B mode 1: reset low >=550us, then one rising edge. The SDK
     * starts the DAC muted and unmutes only after this hook returns. */
    rt_pin_write(OWT_AMP_PIN, PIN_LOW);
    HAL_Delay_us(550);
    rt_pin_write(OWT_AMP_PIN, PIN_HIGH);
    rt_thread_mdelay(100);
    speaker_on = 1;
    rt_kprintf("speaker: enabled PA%u mode=1 rate=%u channels=%u\n", OWT_AMP_PIN, samplerate, speaker_stereo ? 2 : 1);
}

void audio_hardware_pa_stop(void)
{
    rt_pin_write(OWT_AMP_PIN, PIN_LOW);
    speaker_on = 0;
    rt_thread_mdelay(10);
    rt_kprintf("speaker: disabled\n");
}

HAL_StatusTypeDef __real_HAL_AUDPRC_Config_TChanel(AUDPRC_HandleTypeDef *handle, int channel, AUDPRC_ChnlCfgTypeDef *config);
HAL_StatusTypeDef __wrap_HAL_AUDPRC_Config_TChanel(AUDPRC_HandleTypeDef *handle, int channel, AUDPRC_ChnlCfgTypeDef *config)
{
    HAL_StatusTypeDef result = __real_HAL_AUDPRC_Config_TChanel(handle, channel, config);
    if (result == HAL_OK && channel == HAL_AUDPRC_TX_CH0)
        speaker_stereo = config->format == 0 && config->mode == 1;
    return result;
}

void __real_bf0_audprc_device_write(rt_device_t dev, rt_off_t pos, const void *buffer, rt_size_t size);
void __wrap_bf0_audprc_device_write(rt_device_t dev, rt_off_t pos, const void *buffer, rt_size_t size)
{
    const uint8_t *pcm = buffer;
    if (speaker_stereo) {
        RT_ASSERT(size <= sizeof(speaker_pcm) && size % 4 == 0);
        owt_speaker_mono(pcm, speaker_pcm, size);
        pcm = speaker_pcm;
    }
    uint32_t maximum = 0;
    for (unsigned i = 0; i + 1 < size; i += 2) {
        int32_t sample = (int16_t)((uint16_t)pcm[i] | ((uint16_t)pcm[i + 1] << 8));
        uint32_t magnitude = sample < 0 ? -sample : sample;
        if (magnitude > maximum) maximum = magnitude;
    }
    __real_bf0_audprc_device_write(dev, pos, pcm, size);
    rt_base_t level = rt_hw_interrupt_disable();
    playback_frames++;
    playback_samples += size / 2;
    if (maximum > playback_peak) playback_peak = maximum;
    rt_hw_interrupt_enable(level);
}

void __real_audio_3a_uplink(uint8_t *pcm, uint16_t bytes, uint8_t is_mute, uint8_t is_bt_voice);

void __wrap_audio_3a_uplink(uint8_t *pcm, uint16_t bytes, uint8_t is_mute, uint8_t is_bt_voice)
{
    if (is_bt_voice)
    {
        uint32_t maximum = 0;
        /* Measure the actual microphone DMA data; never log or retain samples. */
        for (unsigned i = 0; i + 1 < bytes; i += 2)
        {
            int32_t sample = (int16_t)((uint16_t)pcm[i] | ((uint16_t)pcm[i + 1] << 8));
            uint32_t magnitude = sample < 0 ? -sample : sample;
            if (magnitude > maximum) maximum = magnitude;
        }
        rt_base_t level = rt_hw_interrupt_disable();
        frames++;
        samples += bytes / 2;
        if (maximum > peak) peak = maximum;
        muted = is_mute;
        rt_hw_interrupt_enable(level);
    }
    __real_audio_3a_uplink(pcm, bytes, is_mute, is_bt_voice);
}

void owt_audio_poll(uint32_t now)
{
    static uint32_t last;
    if (now - last < 1000) return;
    last = now;
    rt_base_t level = rt_hw_interrupt_disable();
    uint32_t count = frames, sample_count = samples, maximum = peak;
    uint8_t is_mute = muted;
    uint32_t out_frames = playback_frames, out_samples = playback_samples, out_peak = playback_peak;
    frames = samples = peak = 0;
    playback_frames = playback_samples = playback_peak = 0;
    rt_hw_interrupt_enable(level);
    if (count) rt_kprintf("microphone: frames=%u samples=%u peak=%u muted=%u\n",
                         count, sample_count, maximum, is_mute);
    if (out_frames) rt_kprintf("speaker: frames=%u samples=%u peak=%u enabled=%u\n",
                              out_frames, out_samples, out_peak, speaker_on);
}
