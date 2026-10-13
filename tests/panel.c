/* SPDX-License-Identifier: MIT */
#include <assert.h>
#include <stdio.h>
#include "../native/src/panel.h"

static void cycle(uint32_t start)
{
    struct owt_panel p = { .phase = PANEL_AWAKE };
    assert(owt_panel_step(&p, start, false, true) == PANEL_NONE);
    assert(p.phase == PANEL_AWAKE); /* In-flight frame cannot be slept. */
    assert(owt_panel_step(&p, start, false, false) == PANEL_SLEEP);
    assert(owt_panel_step(&p, start + 119, true, false) == PANEL_NONE);
    assert(owt_panel_step(&p, start + 120, true, false) == PANEL_ASSERT_RESET);
    assert(owt_panel_settling(&p));
    assert(owt_panel_step(&p, start + 129, true, false) == PANEL_NONE);
    assert(owt_panel_step(&p, start + 130, true, true) == PANEL_NONE);
    assert(owt_panel_step(&p, start + 130, true, false) == PANEL_RELEASE_RESET);
    assert(owt_panel_step(&p, start + 179, true, false) == PANEL_NONE);
    assert(owt_panel_step(&p, start + 180, true, false) == PANEL_INITIALIZE);
    assert(owt_panel_step(&p, start + 299, true, false) == PANEL_NONE);
    // A late scheduling pass or changed desired state cannot strand wake.
    assert(owt_panel_step(&p, start + 320, false, false) == PANEL_REFRESH);
    assert(!owt_panel_settling(&p));
    assert(p.phase == PANEL_REFRESHING);
    assert(owt_panel_step(&p, start + 400, false, true) == PANEL_NONE);
    assert(owt_panel_step(&p, start + 500, false, false) == PANEL_NONE);
    // Main marks AWAKE only after the final DMA and Display On.
    p.phase = PANEL_AWAKE;
    assert(owt_panel_step(&p, start + 501, false, false) == PANEL_SLEEP);
}

int main(void)
{
    cycle(0);
    cycle(5 * 60 * 60 * 1000U);
    cycle(UINT32_MAX - 150);
    struct owt_panel p = { .phase = PANEL_ASLEEP, .at = 120000 };
    assert(owt_panel_step(&p, 5 * 60 * 60 * 1000U, true, false) == PANEL_ASSERT_RESET);
    puts("panel sleep/wake sequencing passed");
}
