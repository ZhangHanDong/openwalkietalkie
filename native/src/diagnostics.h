/* SPDX-License-Identifier: MIT */
#ifndef OWT_DIAGNOSTICS_H
#define OWT_DIAGNOSTICS_H
#include <stdint.h>
#include <stddef.h>

/* Private vendor report channel, separate from the system RPC (2).
 * All replies are 63-byte bodies; report ID 6 is supplied by HID. */
#define OWT_DIAG_CHANNEL 4
#define OWT_DIAG_VERSION 2
#define OWT_DIAG_STATUS 0
#define OWT_DIAG_WAKE 1
#define OWT_DIAG_BATTERY 2
/* Battery replies retain the v2 header and uptime fields. All multibyte
 * values are little-endian. Each error bit corresponds to its register. */
#define OWT_DIAG_BATTERY_REGS 8
#define OWT_DIAG_BATTERY_ERRORS 21
#define OWT_DIAG_BATTERY_MV 23
#define OWT_DIAG_BATTERY_CORE 25
#define OWT_DIAG_BATTERY_SOC 25
#define OWT_DIAG_BATTERY_CHARGE 26
#define OWT_DIAG_BATTERY_FULL_MV 27
#define OWT_DIAG_BATTERY_FLAGS 29 /* bit 0: completion candidate pending */
#define OWT_DIAG_BATTERY_CANDIDATE_AGE 30 /* u32 milliseconds, 0 if absent */
static inline const uint8_t *owt_diag_body(const uint8_t *data, size_t len)
{
    if (len == 64 && data[0] == 6) { data++; len--; }
    return len == 63 && data[0] == OWT_DIAG_CHANNEL ? data : NULL;
}
static inline void owt_diag_put32(uint8_t *p, uint32_t value)
{
    for (unsigned i = 0; i < 4; i++) p[i] = value >> (8 * i);
}
static inline void owt_diag_put16(uint8_t *p, uint16_t value)
{
    p[0] = value;
    p[1] = value >> 8;
}
static inline uint16_t owt_diag_get16(const uint8_t *p)
{
    return (uint16_t)p[0] | (uint16_t)p[1] << 8;
}
static inline uint32_t owt_diag_get32(const uint8_t *p)
{
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
#endif
