/* SPDX-License-Identifier: MIT */
#include <rtthread.h>
#include <rthw.h>
#include <rtdevice.h>
#include "bf0_hal.h"
#include "bf0_pm.h"
#include "ipc_queue.h"
#include "openwalkietalkie.h"

/* Override the pinned SDK's one-entry weak policy from PM initialization
 * onwards, including startup delays before main(). PA1/PA11 cannot wake the
 * normal AON-only LIGHT/DEEP handlers; see hardware-design-constraints.md.
 * Deep-WFI clock gating is independently disabled in project/proj.conf.
 * Scenario-based run-frequency switching remains enabled. */
const pm_policy_t pm_policy[] = {{0, PM_SLEEP_MODE_IDLE}};

static uint8_t input_wake_policy(rt_tick_t ticks)
{
    (void)ticks;
    return PM_SLEEP_MODE_IDLE;
}

static int install_idle_policy(void)
{
    /* Keep the ceiling independent of any later policy-table registration. */
    rt_pm_override_mode_select(input_wake_policy);
    return RT_EOK;
}
INIT_ENV_EXPORT(install_idle_policy);

static uint32_t idle_calls, hardware_busy, ipc_busy;
static bool ui_clock;

void owt_idle_power_ui(bool active)
{
    if (ui_clock == active) return;
    ui_clock = active;
    /* Use the SDK scenario arbitration: audio retains its own high-speed
     * vote. Release UI only after the last panel command/DMA has completed,
     * and acquire it before resetting/waking or drawing the panel. */
    if (active) pm_scenario_start(PM_SCENARIO_UI);
    else pm_scenario_stop(PM_SCENARIO_UI);
}

void __real_rt_system_power_manager(void);
void __wrap_rt_system_power_manager(void)
{
    bool hardware = rt_pm_is_hw_device_running();
    bool ipc = !ipc_queue_check_idle();
    __real_rt_system_power_manager();
    /* These are entry-condition snapshots, not measured sleep residency or
     * exact deep-WFI counts. Timing this call would also count other threads
     * scheduled by its wake interrupt. Only the idle thread increments. */
    rt_base_t level = rt_hw_interrupt_disable();
    idle_calls++;
    hardware_busy += hardware;
    ipc_busy += ipc;
    rt_hw_interrupt_enable(level);
}

void owt_idle_power_start(void)
{
    owt_idle_power_ui(true);
    rt_kprintf("power: managed idle, AON sleep blocked (PA1/PA11 wake); HCLK=%uMHz\n",
               HAL_RCC_GetHCLKFreq(CORE_ID_HCPU) / 1000000);
#ifdef BSP_PM_FREQ_SCALING
    rt_kprintf("power: deep-WFI enabled\n");
#else
    rt_kprintf("power: ordinary WFI; scenario run-clock switching enabled\n");
#endif
}

void owt_idle_power_stats(void)
{
    rt_base_t level = rt_hw_interrupt_disable();
    uint32_t calls = idle_calls;
    uint32_t hardware = hardware_busy, ipc = ipc_busy;
    idle_calls = hardware_busy = ipc_busy = 0;
    rt_hw_interrupt_enable(level);
    rt_kprintf("power: idle_calls=%u hw_busy=%u ipc_busy=%u sleep_ceiling=IDLE HCLK=%uMHz ui_clock=%u audio_clock=%u\n",
               calls, hardware, ipc, HAL_RCC_GetHCLKFreq(CORE_ID_HCPU) / 1000000,
               ui_clock, !!(hwp_hpsys_rcc->DBGR & HPSYS_RCC_DBGR_FORCE_HP));
}
