/* SPDX-License-Identifier: MIT */
#ifndef OWT_SPEAKER_PCM_H
#define OWT_SPEAKER_PCM_H
#include <stddef.h>
#include <stdint.h>

/* Preserve the stereo DMA format, duplicating (L+R)/2 for the board's one DAC.
 * The input stays untouched; signed 32-bit arithmetic prevents overflow. */
static inline void owt_speaker_mono(const uint8_t *src, uint8_t *dst, size_t bytes)
{
    for (size_t i = 0; i + 3 < bytes; i += 4) {
        int32_t left = (int16_t)((uint16_t)src[i] | ((uint16_t)src[i + 1] << 8));
        int32_t right = (int16_t)((uint16_t)src[i + 2] | ((uint16_t)src[i + 3] << 8));
        uint16_t mono = (uint16_t)((left + right) / 2);
        dst[i] = dst[i + 2] = (uint8_t)mono;
        dst[i + 1] = dst[i + 3] = (uint8_t)(mono >> 8);
    }
}

static inline uint8_t owt_speaker_volume(uint8_t absolute, uint8_t maximum)
{
    if (absolute > 127) absolute = 127;
    return (uint8_t)(((uint32_t)absolute * maximum + 63) / 127);
}
#endif
