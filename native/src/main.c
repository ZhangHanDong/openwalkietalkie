/* SPDX-License-Identifier: MIT */
#include <rtthread.h>
#include "bf0_hal.h"
#include "drv_io.h"
#include "openwalkietalkie.h"
#include "panel.h"

static void owt_wake_display(void)
{
    const struct owt_message message = { .type = OWT_WAKE_DISPLAY };
    if (!owt_post(&message)) rt_kprintf("display: wake queue full\n");
}
MSH_CMD_EXPORT(owt_wake_display, Wake the display without sending a key);

static void owt_diagnose(void)
{
    const struct owt_message message = { .type = OWT_DIAGNOSE };
    if (!owt_post(&message)) rt_kprintf("diagnostic: queue full\n");
}
MSH_CMD_EXPORT(owt_diagnose, Read panel and input status without waking the UI);

static rt_mq_t messages;
static rt_event_t wakeups;

void owt_wake(void)
{
    if (wakeups) rt_event_send(wakeups, 1);
}

static void wait_for_work(uint32_t ms)
{
    rt_uint32_t received;
    rt_event_recv(wakeups, 1, RT_EVENT_FLAG_OR | RT_EVENT_FLAG_CLEAR,
                  rt_tick_from_millisecond(ms), &received);
}

void HAL_MspInit(void) { BSP_IO_Init(); }

bool owt_post(const struct owt_message *message)
{
    bool posted = messages && rt_mq_send(messages, (void *)message, sizeof(*message)) == RT_EOK;
    if (posted) owt_wake();
    return posted;
}

void owt_platform_panic(void)
{
    rt_kprintf("openwalkietalkie: fatal Rust error\n");
    HAL_NVIC_SystemReset();
    while (1) {}
}

int main(void)
{
    wakeups = rt_event_create("owtwake", RT_IPC_FLAG_FIFO);
    RT_ASSERT(wakeups);
    /* A deck host's full sync is ~110 reports; a stall must not drop the link. */
    messages = rt_mq_create("owt", sizeof(struct owt_message), 64, RT_IPC_FLAG_FIFO);
    RT_ASSERT(messages);
    owt_idle_power_start();
    owt_board_start();
    for (unsigned band = 0; band < 10; band++)
        owt_display_band(band, owt_core_draw_splash_band(band));
    owt_display_on();
    uint32_t splash_at = rt_tick_get_millisecond();
    uint32_t splash_duration = owt_core_splash_duration();
    bool splash_visible = true;
    rt_kprintf("UI: OpenWalkieTalkie boot splash ready\n");
    rt_kprintf("openwalkietalkie: Bluetooth microphone + speaker + HID\n");
    uint32_t devices = owt_core_init();
    rt_kprintf("board: charger=%u keypad=%u touch=%u direct_inputs=%u\n", !!(devices & 1),
               !!(devices & 2), !!(devices & 4), !!(devices & 8));
    /* Both sides name the board they were built for (OWT_BOARD, which
     * tools/build-native.sh turns into the C defines and the core's feature);
     * one archive path serves every board, so a stale core is caught here. */
    static const char *const board_names[] = { "lilygo", "hw-a", "hw-b" };
#if defined(OWT_BOARD_HW_B)
    const unsigned expected_board = 2;
#elif defined(OWT_BOARD_HW_A)
    const unsigned expected_board = 1;
#else
    const unsigned expected_board = 0;
#endif
    unsigned core_board = owt_core_board();
    if (core_board > 2) core_board = 0;
    rt_kprintf("board: %s image, rotation=%udeg\n", board_names[core_board], owt_core_rotation());
    if (core_board != expected_board)
        rt_kprintf("board: MISMATCH: the C side was built for %s but the Rust core for %s;"
                   " rebuild both with OWT_BOARD=%s tools/build-native.sh\n",
                   board_names[expected_board], board_names[core_board], board_names[core_board]);
    owt_bluetooth_start();

    bool frame_active = false, frame_exhausted = false, force_frame = true;
    const uint8_t *prepared = NULL;
    struct owt_region prepared_region;
    uint8_t draw_slot = 0;
    uint32_t frame_at = 0, draw_ms = 0, frame_pixels = 0;
    uint32_t stats_at = 0, frames = 0, frame_ms = 0, render_ms = 0;
    unsigned partial_traces = 0;
    bool dirty = true;
    bool first_frame = true;
    uint8_t battery = 255;
    uint8_t screen = OWT_SCREEN_ACTIVE;
    uint8_t brightness = 0;
    struct owt_panel panel = { .phase = PANEL_AWAKE };
    bool diagnose_pending = true;
    uint32_t diagnose_at = 0;
    rt_kprintf("power: dim=30s off=120s; input wakes, only PTT also acts\n");
    while (1)
    {
        struct owt_message message;
        /* Bound each batch so continuous host traffic cannot starve releases. */
        for (unsigned i = 0; i < 16; i++)
        {
            if (rt_mq_recv(messages, &message, sizeof(message), 0) != RT_EOK) break;
            if (message.type == OWT_WAKE_DISPLAY) owt_core_wake(rt_tick_get_millisecond());
            else if (message.type == OWT_DIAGNOSE) { diagnose_pending = true; diagnose_at = rt_tick_get_millisecond(); }
            else owt_bluetooth_message(&message);
        }
        uint32_t now = rt_tick_get_millisecond();
        owt_audio_poll(now);
        /* Continue host/audio work during the splash, but don't interpret
         * touches against controls that are not visible yet. */
        if (splash_visible && now - splash_at < splash_duration) {
            rt_thread_mdelay(5);
            continue;
        }
        splash_visible = false;
        dirty |= owt_core_poll(now);
        uint8_t percent = owt_core_battery_percent();
        if (percent != battery)
        {
            battery = percent;
            owt_bluetooth_battery(percent);
        }
        uint8_t desired = owt_core_screen();
        bool display_busy = owt_display_busy();
        enum owt_panel_action action = owt_panel_step(&panel, now,
            desired != OWT_SCREEN_OFF, frame_active || display_busy);
        switch (action) {
        case PANEL_SLEEP:
            owt_display_sleep();
            owt_core_panel_slept();
            owt_idle_power_ui(false);
            screen = OWT_SCREEN_OFF;
            dirty = true;
            force_frame = true;
            rt_kprintf("power: screen off at %ums\n", now);
            break;
        case PANEL_ASSERT_RESET:
            owt_idle_power_ui(true);
            owt_display_reset(true);
            rt_kprintf("power: panel wake reset at %ums\n", now);
            break;
        case PANEL_RELEASE_RESET:
            owt_display_reset(false);
            break;
        case PANEL_INITIALIZE:
            owt_display_wake();
            break;
        case PANEL_REFRESH:
            force_frame = dirty = true;
            break;
        case PANEL_NONE:
            break;
        }
        /* Count settling from the actual pin/command operation, after any
         * clock-switch or host/IRQ delay, not the loop's earlier timestamp. */
        if (action != PANEL_NONE) panel.at = rt_tick_get_millisecond();
        if (desired != OWT_SCREEN_OFF && panel.phase == PANEL_AWAKE && !display_busy) {
            /* Dim/active transitions and the Settings brightness rows. */
            uint8_t level = owt_core_brightness();
            if (level != brightness) owt_display_brightness(brightness = level);
            if (desired != screen) {
                screen = desired;
                rt_kprintf("power: screen %s at %ums\n",
                           screen == OWT_SCREEN_DIM ? "dim" : "active", now);
            }
        }
        bool drawable = panel.phase == PANEL_AWAKE || panel.phase == PANEL_REFRESHING;
        if (diagnose_pending && (int32_t)(now - diagnose_at) >= 0 &&
            !frame_active && !display_busy && !owt_panel_settling(&panel) && panel.phase != PANEL_REFRESHING) {
            rt_kprintf("diagnostic: uptime=%u desired=%u panel=%u\n", now, desired, panel.phase);
            owt_board_diagnose();
            diagnose_pending = false;
        }
        if (drawable && !frame_active && dirty && (desired != OWT_SCREEN_OFF || panel.phase == PANEL_REFRESHING)) {
            frame_active = owt_core_begin_frame(force_frame);
            force_frame = dirty = frame_exhausted = false;
            prepared = NULL;
            frame_at = rt_tick_get_millisecond();
            draw_ms = frame_pixels = 0;
        }
        if (frame_active)
        {
            if (!prepared && !frame_exhausted) {
                uint32_t started = rt_tick_get_millisecond();
                prepared = owt_core_draw_next(draw_slot, &prepared_region);
                draw_ms += rt_tick_get_millisecond() - started;
                frame_exhausted = prepared == NULL;
            }
            if (prepared && !owt_display_busy()) {
                owt_display_region(&prepared_region, prepared);
                frame_pixels += (prepared_region.x1 - prepared_region.x0) * (prepared_region.y1 - prepared_region.y0);
                prepared = NULL;
                draw_slot ^= 1;
            }
            if (frame_exhausted && !owt_display_busy()) {
                frame_active = false;
                uint32_t elapsed = rt_tick_get_millisecond() - frame_at;
                frames++;
                frame_ms += elapsed;
                render_ms += draw_ms;
                if (panel.phase == PANEL_REFRESHING) {
                    owt_display_brightness(brightness = owt_core_brightness());
                    owt_display_enable();
                    panel.phase = PANEL_AWAKE;
                    screen = desired;
                    owt_core_frame_ready();
                    diagnose_pending = true;
                    diagnose_at = rt_tick_get_millisecond() + 80;
                    rt_kprintf("power: screen awake after full refresh at %ums\n", rt_tick_get_millisecond());
                }
                if (first_frame) {
                    first_frame = false;
                    rt_kprintf("UI: first frame ready\n");
                    rt_kprintf("display: full frame=%ums draw=%ums pixels=%u; partial + DMA overlap\n",
                               elapsed, draw_ms, frame_pixels);
                } else if (frame_pixels < 480U * 480U && partial_traces < 4) {
                    partial_traces++;
                    rt_kprintf("display: partial pixels=%u frame=%ums draw=%ums\n", frame_pixels, elapsed, draw_ms);
                }
            }
        }
        if (now - stats_at >= 30000) {
            stats_at = now;
            rt_kprintf("display: updates=%u frame_ms=%u draw_ms=%u\n", frames, frame_ms, render_ms);
            frames = frame_ms = render_ms = 0;
        }
        owt_power_stats(now, screen);
        /* No artificial delay while a free buffer can be drawn. Input and
         * host queues are serviced between regions; DMA completion wakes us.
         * A bounded fallback detects a lost DMA IRQ instead of hanging. */
        if (messages->entry) owt_wake();
        uint32_t wait_ms = frame_active ? ((prepared || frame_exhausted) ? 1 : 0)
            : owt_panel_settling(&panel) ? 5 : dirty && drawable && desired != OWT_SCREEN_OFF ? 0 : owt_core_wait_ms();
        wait_for_work(wait_ms);
    }
}
