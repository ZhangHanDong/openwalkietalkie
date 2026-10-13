/* SPDX-License-Identifier: MIT */
#ifndef OWT_PANEL_H
#define OWT_PANEL_H
#include <stdbool.h>
#include <stdint.h>

enum owt_panel_phase { PANEL_AWAKE, PANEL_ASLEEP, PANEL_RESET_LOW,
    PANEL_RESET_HIGH, PANEL_WAKING, PANEL_REFRESHING };
enum owt_panel_action { PANEL_NONE, PANEL_SLEEP, PANEL_ASSERT_RESET,
    PANEL_RELEASE_RESET, PANEL_INITIALIZE, PANEL_REFRESH };
struct owt_panel { enum owt_panel_phase phase; uint32_t at; };

/* Advance only at command-safe boundaries. Once a wake starts, finish it
 * even if the desired state changes; the next pass can sleep again. */
static inline enum owt_panel_action owt_panel_step(struct owt_panel *p,
    uint32_t now, bool want_on, bool busy)
{
    if (busy) return PANEL_NONE;
    uint32_t elapsed = now - p->at;
    enum owt_panel_action action = PANEL_NONE;
    switch (p->phase) {
    case PANEL_AWAKE:
        if (!want_on) { p->phase = PANEL_ASLEEP; action = PANEL_SLEEP; }
        break;
    case PANEL_ASLEEP:
        if (want_on && elapsed >= 120) { p->phase = PANEL_RESET_LOW; action = PANEL_ASSERT_RESET; }
        break;
    case PANEL_RESET_LOW:
        if (elapsed >= 10) { p->phase = PANEL_RESET_HIGH; action = PANEL_RELEASE_RESET; }
        break;
    case PANEL_RESET_HIGH:
        if (elapsed >= 50) { p->phase = PANEL_WAKING; action = PANEL_INITIALIZE; }
        break;
    case PANEL_WAKING:
        if (elapsed >= 120) { p->phase = PANEL_REFRESHING; action = PANEL_REFRESH; }
        break;
    case PANEL_REFRESHING:
        break; /* The application commits AWAKE only after full-frame DMA. */
    }
    if (action != PANEL_NONE) p->at = now;
    return action;
}

static inline bool owt_panel_settling(const struct owt_panel *p)
{
    return p->phase == PANEL_RESET_LOW || p->phase == PANEL_RESET_HIGH || p->phase == PANEL_WAKING;
}
#endif
