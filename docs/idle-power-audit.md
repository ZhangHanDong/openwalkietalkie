# Idle power investigation — 2026-09-10

## Repeated black screen — 2026-09-11

### HardFault capture and clock-transition placement correction

The next user-confirmed unplug/touch/reconnect reproduction captured a **CPU
HardFault lockup**, before the charger watchdog recovered the board. At USB
return (host time 1789098356.648), SCB CFSR was `00010001` (instruction-access
violation and undefined instruction), HFSR `40000000` (forced HardFault),
DHCSR `05080000` (lockup), PC `effffffe`, xPSR `01000003`, MSP `20001fe0` and
PSP `20030d78`. MMFAR/BFAR validity bits were clear: their values are not usable
fault addresses. The lockup PC is not the original faulting instruction. A
later stack read happened after watchdog recovery and is **not a crash stack**.
Capture: `/tmp/openwalkietalkie-usb-stall-cpu.log`.

Bluetooth status still succeeded just after USB removal at host time
1789098350.432: uptime 596,666 ms, screen off, 48 MHz, normal touch/keypad
configuration, no I2C errors and charger input removed. At lockup RCC CSR was
`000040a3`, CFGR `003c6101`, and DLL1CR `8001da65`, consistent with the wake
transition having selected fast system and MPI clocks. This narrows the
failure to the run-clock transition; disabling deep-WFI had not prevented it.

ELF inspection found two concrete violations of the SDK's RAM execution
requirements during NOR clock changes:

- The board wrapper was named `clock.c`. The SDK linker places `*bsp_init.o`
  in SRAM, so this rename left `BSP_SetFlash1DIV`, `BSP_SetFlash2DIV` and both
  getters in external NOR at `1204f8fc`–`1204f920`. The SDK's SRAM-resident
  `sifli_pm_run` called them after increasing the clock and before restoring
  the NOR divider. Instruction-cache hits can mask this dependency.
- `BSP_Flash_hw2_init` also called `memset` before restoring the divider.
  Rust compiler_builtins supplied that weak symbol at `120c07b6`; its object
  filename does not match the SDK's `*memset*.o` SRAM selector.

The wrapper is now `sdk_bsp_init.c`, preserving the SDK's placement rule and
implementation. `ram_memory.c` supplies a strong SRAM `memset` using the
existing SRAM `rt_memset`. No SDK clock frequencies or transition ordering
were changed. `check-native-image.py` now checks 15 required helpers against
internal SRAM and their executable sections. It rejects the previous image
and six individual symbol-placement mutations. The corrected ELF passes;
disassembly traversal found no XIP branch on the normal `sifli_pm_run` call
path (the assertion-failure handler remains in flash).

The candidate has **1,096,616 load bytes** and was flashed with verification
after one stub-download retry, preserving the bootloader and bond stores.
The 72 native-core tests and C panel-sequencing test pass. Both Mac audio
endpoints reappeared (16 kHz input and 44.1 kHz stereo output). A Bluetooth
wake request moved the device from screen-off / 48 MHz at uptime 148,001 ms
to screen-on / 240 MHz at 150,081 ms with continuous uptime and no I2C errors.
The UART recorded reset start at 148,933 ms and full-refresh completion at
149,162 ms (229 ms). This verifies the connected software wake path, not
visible output or battery-only physical wake.

**Physical acceptance passed:** the user repeated the unplug/touch/reconnect
sequence and confirmed that the screen now works. USB disappeared at host
time 1789099378.248 and returned at 1789099384.104. Bluetooth status recorded
the screen awake on battery at uptime 277,089 ms, with 12 touch interrupts,
no I2C errors and charger input removed. Replies continued across reconnection
without an uptime reset or change in saved boot cause. The automatic CPU
capture found CFSR=0, HFSR=0, no lockup and normal thread execution, unlike
the earlier failed capture. See `/tmp/openwalkietalkie-clock-ram-usb-capture.log`.

The device had reached 48 MHz screen-off idle before the test; an audio vote
briefly raised it to 240 MHz immediately before unplugging. This is one
successful physical cycle with the corrected firmware, not a controlled
measurement of every clock-transition workload. The original faulting
instruction was not recovered. Ordinary WFI remains selected in this
accepted image; re-enabling the separate deep-WFI optimization requires a
new validation pass. Battery-current and runtime measurements remain open.

Logs: `/tmp/openwalkietalkie-clock-ram-build.log`,
`/tmp/openwalkietalkie-clock-ram-flash.log`,
`/tmp/openwalkietalkie-clock-ram-live.log`,
`/tmp/openwalkietalkie-clock-ram-wake-test.log`,
`/tmp/openwalkietalkie-clock-ram-serial.log`.

### USB transition and ordinary-WFI mitigation

**Negative result:** the user repeated unplug/touch with ordinary WFI and the
screen still failed to wake. The Bluetooth link stopped responding after
host time 1789097673.179 (uptime 314,415 ms). After automatic recovery the
saved boot charger status was **08h=9Ah**, with **WTD_FAULT bit 7 set**, fault
register 09h=00h, boot mode 0, and `deep_wfi=0`. The charger watchdog caused
the power cycle after service stopped. Disabling deep-WFI is therefore not a
fix for this failure. The underlying CPU/driver stall still needs diagnosis.

SF32 AHB-debug reads were verified without a reset/stub download: uptime
continued through a read-only register capture. A roughly 400 ms UART break
test with USB power present did not reproduce the failure. An automatic
USB-return capture read fault/clock/UART registers and briefly
halted/sampled/restored the CPU before watchdog recovery (results above). Temporary tool:
`/tmp/openwalkietalkie-debug-read.py` (framing from installed sftool-lib 0.2.5).
Captures: `/tmp/openwalkietalkie-ahb-healthy.log`,
`/tmp/openwalkietalkie-uart-break-test.log`, `/tmp/openwalkietalkie-usb-stall-cpu.log`.

The user confirmed **only unplugging and reconnecting USB**, with no use of
the power switch, during the Bluetooth-only capture below. The restart is
therefore automatic and the failure extends beyond panel wake. This does not
by itself distinguish a power interruption, a CPU fault, or a charger watchdog
recovery after the CPU stopped servicing it. Earlier reboots following serial
attachment cannot all be attributed to RTS on that timing evidence alone.

The current candidate disables `BSP_PM_FREQ_SCALING`, the pinned SDK's
**separate deep-WFI idle path**. The ordinary WFI fallback and scenario-based
240/48 MHz run switching remain compiled in; the IDLE ceiling, display sleep,
touch/keypad polling and audio scenario votes remain active. This is a
conservative mitigation and controlled comparison, **not a proven root cause**.
The image builds (1,096,912 load bytes), passes flash bounds, and was flashed
with verification after two stub-download retries. Symbol inspection found
`rt_hw_cpu_idle` and `sifli_pm_run` but no `sifli_deep_wfi`. Bluetooth readback
confirms `deep_wfi=0` and boot mode 4 (the programming reset), then screen-off
at **48 MHz** with intact input settings and no bus errors. macOS enumeration
still exposes the 16 kHz microphone and 44.1 kHz stereo speaker endpoints.

Diagnostics version 2 preserves charger registers 08h/09h/0Ah before charger
initialization, reports the SDK's saved boot mode/source, and exposes whether
deep-WFI was compiled in. The host watcher timestamps results and reacquires
the HID interface for each sample so it can follow an automatic restart.
The physical unplug/touch test failed as recorded above. No battery-runtime
gain is claimed for this mitigation.

Logs: `/tmp/openwalkietalkie-ordinary-wfi-build.log`,
`/tmp/openwalkietalkie-ordinary-wfi-flash.log`,
`/tmp/openwalkietalkie-ordinary-wfi-live.log`.

On the next user-confirmed reproduction, reattaching serial was followed by
`SFBL` and a cold boot. No valid post-failure register snapshot was obtained.
The first HID probe overlapped that attachment; its timeout cannot independently
establish a dead Bluetooth application before the reboot. The recovered app
answered HID normally. Capture: `/tmp/openwalkietalkie-usb-failed-state.log`.

To avoid repeating this loss of evidence, the next diagnostic image adds a
private encrypted Bluetooth HID status/wake channel and
`tools/diagnose-device.c`. It was built (1,097,492 load bytes), flashed, and
verified with a continuous Bluetooth-only status capture, including screen-off
at 48 MHz. Touch mode readback at 0002h was `00 09 00 00`, keypad CFG=01h,
expander outputs=0084h and direction=0000h, with no I2C failures. No serial
console was open during this reproduction. The capture remained healthy through
uptime 193,995 ms (48 MHz, screen off, normal touch mode, intact keypad and
expander settings, no charger faults). The vendor interface then stopped
replying and disappeared. A fresh Bluetooth-only request returned uptime
22,962 ms and an active screen: **the device had restarted without opening
serial**. The user subsequently confirmed that only USB was unplugged and
reconnected (see mitigation above). Log: `/tmp/openwalkietalkie-wireless-live.log`.

The 72 native-core tests and C panel-sequencing test pass with the diagnostic
transport change. These do not validate the physical USB transition.

The user supplied a specific reproducer: let the display sleep with USB
connected, unplug USB, attempt to wake it, then reconnect USB. Reconnecting
does not restore physical wake. A subsequent `owt_wake_display` command
completed the full refresh in 226 ms and the user **confirmed a visible
screen**. This demonstrates panel recovery by software in that captured
state; it does not yet identify why the physical wake attempt failed.

The user reported another failure after the wake-recovery update and clarified
that the screen was **black**. The prior completed-DMA/debug-wake checks did not
prove that the panel was emitting a visible image. The failure remains open.

The first serial attachment was followed by a cold boot, consistent with the
bridge asserting reset during open. That capture cannot establish the frozen
CPU state or the reset's original cause. The first HID probe overlapped that
attachment, so its missing interface cannot establish a pre-existing HID loss.
Later, with the serial connection held open, the application was alive in
screen-off / 48 MHz mode: 186 I2C transactions per roughly 30 seconds, successful
charger watchdog feeds, zero input-bus errors and no touch/keypad IRQs in those
windows. Independent HID status requests completed. A requested physical wake
attempt has not yet been confirmed, so zero IRQs alone do not prove broken
input hardware. The serial device subsequently disappeared while HID continued
to respond; further wired diagnostics require reconnecting USB.

A diagnostic image (1,097,012 application load bytes) was built and flashed.
It adds the queued debug
command `owt_diagnose`, reads CO5300 ID (04h), power mode (0Ah) and brightness
(52h) at LilyGO's 2 MHz read rate, and records the keypad configuration, expander
outputs, pin levels and last raw touch report. It also schedules a readback
after wake. Readback does not trigger a UI wake or drain the keypad FIFO.
`HAL_OK` alone means the SPI controller completed: validate the returned ID and
status before interpreting a floating/all-zero response as panel state.
On this board, all three panel reads returned zero even with a working screen,
so these SPI results are **not a valid panel-state measurement**. The input
and charger I2C readbacks worked. Before unplugging: keypad CFG=01h, expander
outputs=0084h, IRQ pins=1/1; charger registers 01=a4, 06=c0, 07=73,
08=12/1a (charging/done), 09=00, 0a=60, 0c=7a; PA41/PA44/PA0 were high.
The charger reads are individual register transactions and do not change its
power-path settings. A post-unplug physical-input capture is still pending.

The panel datasheet defines RDDPM bit 4 as Sleep Out and bit 2 as Display On,
and permits this read in Sleep In. Source: [CHIPONE CO5300 datasheet, §7.5.5](https://files.waveshare.com/wiki/common/Co5300_Datasheet.pdf).

Use `tools/monitor.py PORT --interactive` with one continuous connection to
send `owt_diagnose`, `pm_dump`, or `owt_wake_display`. The monitor now releases
RTS/DTR before configuring termios, shortening the incidental reset pulse;
non-resetting attachment still cannot be guaranteed by software. Keep a capture
open before reproducing the fault. USB-attached observations do not establish
battery-only power consumption or wake reliability.

Captures: `/tmp/openwalkietalkie-dead-again-current.log` and
`/tmp/openwalkietalkie-dead-again-continuous.log`. Diagnostic build:
`/tmp/openwalkietalkie-black-screen-diagnostic-build.log`.
USB reproducer work: `/tmp/openwalkietalkie-usb-removal-live.log`,
`/tmp/openwalkietalkie-usb-wake-diagnostic-live.log`,
`/tmp/openwalkietalkie-usb-wake-diagnostic-build.log` and
`/tmp/openwalkietalkie-usb-wake-diagnostic-flash.log`.

## Follow-up: five-hour discharge and failed wake

The subsequent user test dropped from **100% to 25% in about five hours**, and
the user reported that the dark device required a power cycle to recover.
This does **not demonstrate a runtime improvement**. The displayed percentage
comes from the coarse voltage curve rather than a coulomb counter, so this
partial discharge cannot establish average current or total runtime.

Before resetting the device, two UART windows showed the application still
running: screen off, 183–186 I2C transactions, zero panel transfers, and two
successful charger watchdog feeds per roughly 30 seconds. `pm_dump` reported
**Idle Mode / High Speed**, and the audio worker threads were suspended. This
capture does not establish whether the user's earlier failure was input-only,
panel-only or a whole-device hang: attaching serial restores USB power, and no
physical wake attempt was confirmed during the capture.

The run-clock integration was incomplete. The previous change enabled the
SDK's deep-WFI clock scaling, but the application never released its startup
240 MHz run configuration on screen sleep. Only an unrelated SDK audio-client
closure could request the 48 MHz run mode. Deep-WFI entry counters did not prove
that the high-speed run/memory clock configuration had been released.

The follow-up firmware:

- Owns a `PM_SCENARIO_UI` vote from startup through the last display transfer.
  Sleep releases that vote, allowing the SDK to select its 48 MHz run mode and
  lower memory clocks when no other scenario is active. Audio retains its own
  independent high-speed vote. Wake acquires UI speed before panel access.
- Resets **only the panel** on PA9 during wake, restores its initialization
  registers and QSPI clock, waits for Sleep Out, then refreshes the entire
  hidden frame before Display On. Reset/settling waits are nonblocking, so
  input, Bluetooth and audio work continue. PA0 touch reset and the input
  supply stay unchanged. This is recovery hardening, not proof that retained
  panel state caused the reported failure.
- Accepts a latched touch interrupt with a valid release-only report as a wake
  tap. The previous contact-only path could ignore a tap that ended before its
  first read. Empty fallback polls, invalid frames and bus failures do not
  create wake commands. Every physical matrix key can now wake, including
  keys with no action binding. Wake touches remain consumed.
- Logs input IRQs, touch reads, bus errors, wake source, battery millivolts,
  uptime, actual HCLK and UI/audio clock votes. The debug UART command
  `owt_wake_display` queues a wake on the application thread and emits no HID
  action; it helps distinguish panel recovery from physical input detection.

The IDLE ceiling remains in place. No AON-only sleep mode or panel rail removal
was enabled. The 1 kHz tick, the Mac's observed 15 ms / zero-latency BLE link and
the prototype's retained peripheral supplies remain power costs to measure.
Do not claim a runtime multiplier from a CPU frequency ratio.

### Validation for this follow-up

Native-core tests cover release-only wake after five hours, unbound key wake,
no unintended HID actions, and suppression of another tap during panel reset.
The standalone `tests/panel.c` test checks command ordering, minimum settling
times, DMA exclusion, delayed scheduling, repeated sleep and clock wrap.

- Native core **72 tests**, shared host **59 tests**, and the C panel sequencing
  check pass; both firmware targets build. The native application-only flash
  preserves the bootloader and Bluetooth bond stores.
- The first on-device cycle slept at **120,015 ms**. A complete dark window
  recorded **48 MHz HCLK**, UI/audio clock votes off, **183 I2C transactions**,
  zero display transfers, zero input-bus errors and two successful charger
  watchdog feeds. HID status requests completed at 48 MHz.
- Opening the explicitly selected device microphone returned **81,600 samples**
  with nonzero data; HID status also completed during capture. Audio raised the
  CPU to **240 MHz** while the screen stayed asleep. Speaker samples were sent
  through HFP and subsequently stereo A2DP, followed by amplifier/PLL shutdown
  and a return to **48 MHz**. The SDK still logged HFP receive/buffer warnings
  (zero decode errors); this check does not certify audio quality.
- Debug-only wake at **275,451 ms** completed at **275,679 ms**: one complete
  230,400-pixel refresh (45 ms draw/transfer elapsed, 40 ms drawing), with HCLK
  restored to **240 MHz**, no input-bus errors, no DMA timeout and no restart.
  The final timing refinement starts each settling interval after its hardware
  operation, so time spent restoring clocks cannot shorten the reset pulse.
- The **final flashed image** (1,095,472 load bytes) recorded physical touch
  down/up pairs, slept at **133,049 ms** after that activity, and reached
  **48 MHz** with both clock votes off. The timed debug wake at **168,356 ms**
  completed at **168,588 ms** (**232 ms**); `pm_dump` confirmed return to High
  Speed / Idle Mode. No input-bus error, DMA timeout or restart was logged.

Physical touch/keypad wake after prolonged **battery-only** idle and a repeat
discharge measurement remain required. Serial-controlled wake alone cannot
validate the PA1/PA11 physical input path.

Diagnostic captures: `/tmp/openwalkietalkie-wake-failure-current.log`,
`/tmp/openwalkietalkie-wake-input-probe.log`,
`/tmp/openwalkietalkie-wake-recovery-live.log`,
`/tmp/openwalkietalkie-wake-audio-live.log`,
`/tmp/openwalkietalkie-wake-panel-live.log`,
`/tmp/openwalkietalkie-wake-final-live.log`.

## Baseline and conclusion

The user measured approximately **9 hours**, mostly idle with the screen asleep,
on the current firmware (`d104747`). This is a reported runtime, not a measured
current or a controlled discharge trace. The keypad cell capacity, starting
state of charge and end-of-test voltage are not established.

The next priority is HCPU idle clock/power management and unnecessary wakeups.
Dark colors and shorter display timeouts have little additional benefit during
an already screen-off test. Full system sleep needs special care: this board's
touch/keypad interrupt pins are not the SF32LB52's dedicated AON wake pins.

## Verified findings before the follow-up implementation

| Finding | Evidence | Implication |
| --- | --- | --- |
| HCPU power management is disabled | `native/project/proj.conf:3`; generated HCPU config has neither `RT_USING_PM` nor `BSP_USING_PM` | The event wait permits ordinary CPU idle, but does not enable the SDK's managed light/deep sleep or idle frequency scaling. |
| High-speed clocks remain configured | Board `clock.c` includes SDK `customer/boards/sf32lb52-lcd_base/bsp_init.c`: HCPU 240 MHz, DLL2 288 MHz; no application frequency switching | Ordinary WFI pauses execution; it does not establish that the surrounding clocks and memory have entered their lowest-power states. |
| Periodic work continues while dark | Fresh UART windows: `screen=2`, `i2c=1794–1800`, `bands=0` per approximately 30 seconds | About 60 I2C transactions/s remain, predominantly keypad and touch fallback polling every 50 ms. |
| The OS tick is 1 kHz | Generated HCPU `RT_TICK_PER_SECOND=1000`; SDK `drv_common.c` configures SysTick from it | A 50 ms application wait does not mean the CPU receives interrupts only every 50 ms. |
| Display rendering really stops | Same fresh windows show zero display bands | Further rendering optimization will not fix dark-idle drain. |
| Panel supply remains enabled | PA44 is driven high in `native/boards/openwalkietalkie/power.c`; display sleep sends CO5300 commands `0x28` and `0x10` | Remaining panel/regulator current needs measurement; the rail is a candidate for further shutdown. |
| No audio stream was observed during this check | Fresh 40-second and 35-second UART captures contained no microphone/speaker frame lines | No evidence of an audio stream stuck open in these windows; this is not a codec-current measurement. |
| Charger servicing is healthy | Two successful watchdog feeds per 30 seconds, zero errors, `vbat_monitor=0` | The ADC monitor is already disabled between samples; this is not the leading obvious software issue. |
| Several unused keypad functions are already disabled | XL9555 starts all outputs low; only `WIFI_EN` and `KEY_RST` are raised | GPS/physical lighting stay disabled and ESP32-C6 is held in reset. The Wi-Fi supply also powers the keypad, so it cannot simply be cut. |
| The LCPU already includes PM support | Generated LCPU config enables `BSP_USING_PM`, `RT_USING_PM`, `PM_DEEP_ENABLE` | The missing PM integration is principally on the application/HCPU side. |

The diagnostic captures were made with the USB serial connection present. They
establish software activity, not battery-only current or the cause of the entire
9-hour result. Temporary logs: `/tmp/openwalkietalkie-idle-audit-live.log` and
`/tmp/openwalkietalkie-idle-audit-dark.log`.

## Recommended implementation order

### 1. Enable the SDK's clock-saving idle path, with explicit sleep limits

Integrate `BSP_USING_PM` and `BSP_PM_FREQ_SCALING` on HCPU, starting with a policy
that forbids AON-only LIGHT/DEEP/STANDBY entry. The pinned SDK has a distinct
hardware **deep-WFI clock-scaling** path for SF32LB52: HAL initializes it to RC48
divided by 12, and the PM idle hook uses it only when its hardware-device and IPC
guards allow it. This is a candidate for saving power while retaining ordinary
interrupt wakeup; it must be validated on the board.

Audit the direct LCD HAL transfers, I2C operations, flash writes and audio DMA
against those guards. Our display bypasses the normal RT-Thread LCD device, so
the PM layer must be told when a transfer is active. Keep full performance for
rendering and audio; do not blindly change CPU/PSRAM clocks during a transfer.
Add counters for ordinary idle, clock-saving idle and blocked entry reasons.

### 2. Reduce dark-screen polling and host wakeups

Use the existing latched PA1/PA11 interrupts as the immediate wake path. Trial a
500 ms recovery poll only when dark and no input is held, retaining the existing
fast contact/release checks, wheel repeat and 120 ms stale-touch cancellation.
The intended reduction is approximately tenfold in *fallback I2C activity*,
not a promised tenfold reduction in device current. Validate short taps and
burst keypad input before adopting the longer recovery interval.

Treat the 1 kHz OS tick separately. Do not globally reduce it just to obtain
longer idle periods: that would change audio and input timing. Proper tickless
sleep depends on resolving the wake-source constraints below.

### 3. Measure and shut down residual peripheral loads

Measure PA44 panel-supply current after Sleep In. If material, implement a
complete panel rail-off/reset/reinitialization path, keep touch power available,
prevent signal-pin back-powering and redraw while hidden before display-on.

Check the actual LoRa, IMU, SD and audio-codec idle states rather than assuming
an unused driver means an unpowered chip. Use the schematic and register
readback to distinguish disabled rails, reset, standby and active states. Keep
the keypad's shared rail enabled; its unrelated ESP32-C6 is already in reset.

### 4. Tune connected Bluetooth only after measuring its current behavior

Log negotiated BLE interval/latency and Classic link mode/interval. The current
application does not request a separate low-power connected BLE policy.
Peripheral latency may permit skipping empty connection events while retaining
prompt key transmission, subject to the Mac accepting the parameters.

The pinned Classic connection manager already requests sniff after 5 seconds,
with a nominal 798-slot (498.75 ms) interval. It also disables discovery after
connection in the prior device logs (`Classic scan mode=0`). Verify negotiated
behavior before treating either as missing. Preserve HFP/A2DP connections and
leave sniff promptly when audio begins. Advertising changes affect disconnected
operation and are not the first priority for this connected-idle test.

### 5. Add full sleep only with a proven wake path

The dedicated [hardware design constraints](hardware-design-constraints.md)
document tracks this as **HW-001**, with PCB requirements and acceptance checks.

The pinned HAL maps dedicated HCPU AON wake pins to **PA24–PA44**. Touch is on
**PA1**, keypad IRQ on **PA11**. The normal SDK LIGHT/DEEP handlers mask ordinary
interrupts and enable AON wake, so enabling their default policy could strand
touch/PTT or make them wait for a timer/host event.

Options to evaluate are a validated alternative GPIO wake route, a bounded
timer-assisted scheme with demonstrated short-press retention and acceptable
latency, or routing the interrupt signals to suitable wake pins on our own
hardware. Test RAM/PSRAM retention, panel recovery and simultaneous HID/audio
before using full sleep in production. The always-on radio core and mailbox
wake capability do not by themselves prove local touch/PTT can wake HCPU.

## Measurement and acceptance

Measure at the battery interfaces with USB disconnected, accounting for both
cells and their power path. Repeat the same workload and full-charge/start/end
conditions for each change. Separate screen-off connected idle, disconnected
idle, interactive UI, microphone capture and speaker playback.

Record average power, peak current, idle residency, wake latency, packet loss
and charger status. Exercise rapid touch/PTT release, wake-only first touch,
wheel repeat, reconnect, microphone start/stop and playback after long idle.
Use PM activity counters to explain measurements, not to substitute for them.

With the same usable battery energy and workload, improving 9 hours to 18 hours
requires a 50% average-power reduction; 24 hours requires 62.5%. These are target
budgets, not predicted gains. Do not infer current by dividing the display's
470 mAh rating by runtime while the keypad battery contribution is unknown.

LilyGO's factory report measured **22.99 mA** with its CPU active, LCD off,
touch/SD on, and Bluetooth/keypad off at a 4.0 V battery interface. That supports
investigating the always-powered system rather than display pixels alone. Its
**164 µA hibernate** case disables touch, Bluetooth and keypad and is not an
equivalent always-connected target. One other hibernate row reports 95.44 mA;
the report does not explain that discrepancy, so no runtime extrapolation is
based on its hibernate figures.

## Sources

- [SiFli power management](https://docs.sifli.com/projects/sdk/latest/en/sf32lb52x/middleware/pm.html)
- [LilyGO factory power report](https://github.com/Xinyuan-LilyGO/T-Display-SF32/blob/master/lowpowertest/T-SF32-Display-powertest.pdf), dated 2026-06-26, `menu_app_v1.0`
- Pinned SDK `006d03293e0621d694335cd4031773279fa2f19a`: `middleware/system/bf0_pm.c`, `drivers/hal/bf0_hal_hpaon.c`, `drivers/hal/bf0_hal_rcc.c`, `customer/boards/sf32lb52-lcd_base/bsp_init.c`, `rtos/rtthread/bsp/sifli/drivers/drv_common.c`
- Pinned Bluetooth `c3f204157265c275b5a9af8b20ef40663d9feab2`: `service/bt/bt_cm/bt_connection_manager.c`, `service/ble/ble_connection_manager.c`

## Follow-up implementation

The native target now enables guarded HCPU idle clock scaling, with a strong
IDLE-only policy present during SDK initialization and a mode-selector override
installed before application startup. AON-only sleep remains blocked for HW-001.
Direct panel transfers, battery ADC reads and flash/bond updates now participate
in the SDK's hardware-activity guards. Audio and I2C already use those guards.

Native screen-off fallback polling is 500ms, with existing interrupt latches
providing immediate input handling and fast active/release recovery preserved.
The keypad FIFO is rechecked after interrupt acknowledgement; a newly arrived
event schedules another immediate wake instead of waiting for the recovery poll.
The installed host companion checks unchanged layouts every 60 seconds instead
of 10; local nonblocking HID reads detect removal without radio requests, and
reconnection/config changes trigger prompt synchronization.

Both firmware targets build. The native core's 65 tests, shared host's 58 tests
and companion's 10 tests pass. New tests exercise dark polling, interrupt wake,
short PTT press/release, wheel input, timer wrap and an event arriving during
interrupt acknowledgement. Linked ELF inspection
confirms the strong IDLE-only policy replaces the SDK default. Initial device
checks confirm HID status replies and live touch edges with PM enabled.

The 1kHz OS tick, full AON sleep, panel supply removal and negotiated Bluetooth
parameter tuning remain follow-ups. BLE parameter logging was added to inform
that next step. Battery-current measurements and a repeat discharge test are
still needed to establish the runtime gain.

### Device validation — 2026-09-10

- The final native image contains **1,076,412 load bytes** and was flashed with
  verification while preserving the bootloader and bond stores. Linked ELF
  inspection confirms the strong IDLE-only policy, mode override, guarded SDK
  idle hook and keypad acknowledgement retry are present.
- With the new PM/polling behavior, three complete dark windows recorded
  **183–186 I2C transactions per approximately 30 seconds**, versus the earlier
  **1,794–1,800** baseline: approximately **90% fewer transactions**. Each window
  had zero display bands, two successful watchdog feeds, zero watchdog errors
  and `vbat_monitor=0`. These captures preceded the final acknowledgement-race
  fix; that fix does not add traffic during an empty idle poll.
- On the final image the display switched off at **120,006ms**, then completed
  its hidden-frame wake refresh at **134,700ms** following local input. The input
  interrupted the intended final dark measurement window. Rapid-input behavior
  is covered by regression tests; a complete physical PTT/wheel stress test has
  not been confirmed for this image.
- BLE HID, HFP and A2DP reconnected automatically after flashing. The installed
  companion detected removal and restored the configured layout on reconnect.
  The Mac negotiated BLE from 30ms to **15ms, latency 0, timeout 720ms**. No
  connection-parameter tuning was applied in this pass.
- A five-second explicit-device microphone capture received **81,600 samples**
  with nonzero data. A HID status request completed while capture was active.
  No audio was saved. Firmware logged stream closure and amplifier shutdown.
  The SDK also logged HFP receive/buffer warnings with zero reported decode
  errors; sample delivery alone does not establish audio quality or packet-loss
  performance.
- The subsequent speaker probe could not find the deck's audio output (then
  named Codex Micro). Follow-up
  checks found both Bluetooth entries disconnected and the USB serial port
  absent. Therefore final-image A2DP playback and a complete post-audio idle
  window remain **unverified**; the cause of the disconnection is not established.

The activity captures used USB serial and do not measure battery-only current.
The 90% reduction applies to I2C transaction count, not average power or runtime.
Repeat the reported mostly-idle test from a confirmed full charge, with USB
disconnected and the same Bluetooth/audio workload, to determine the gain over
the approximately nine-hour result. Log start/end conditions and reconnect the
device to complete the outstanding speaker and physical-input checks.

Temporary validation logs: `/tmp/openwalkietalkie-idle-final-live.log` (complete
dark windows), `/tmp/openwalkietalkie-idle-release-live.log` (final firmware),
`/tmp/openwalkietalkie-idle-release-mic-hid.log` and
`/tmp/openwalkietalkie-idle-release-speaker.log`.
