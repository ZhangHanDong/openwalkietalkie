# Hardware design constraints

The [BodyV5 r3 release](../hardware/rev-a/README.md) is the current hardware
authority. This document retains prototype observations and acceptance
requirements; it is not an alternative PCB layout or assembly release.

Design input for our own OpenWalkieTalkie PCB, recorded on 2026-09-10 from the
LilyGO T-Display SF32 / T-SF32-Keyboard prototype. Keep this document with the
schematic review. It records hardware constraints and proposed design
requirements; firmware tuning and runtime results belong in the
[idle power audit](idle-power-audit.md).

The current target is SF32LB52x. Recheck each requirement against the exact
ordered part, package, silicon revision and intended sleep mode before assigning
PCB pins. The recommendations below are not a finalized pin allocation.

The [BodyV5 r3 hardware](../hardware/rev-a/README.md) uses
SF32LB525UC6 and direct GPIO controls. Its datasheet review narrows the generic
SDK wake range: **A3 standby/hibernate wake pins are PA24–PA27 and PA34–PA44**.
PA28–PA33 must not be assigned to essential wake controls on that assumption.

| ID | Constraint | Status | Requirement for our PCB |
| --- | --- | --- | --- |
| HW-001 | Prototype touch/keypad IRQs are outside the dedicated HCPU AON wake-pin range | Pin mapping and SDK behavior verified; Rev A r6 wakes from PMU shutdown on PA26 and PA24 (the `hw-a` soft power-off, bench-verified 2026-10-03); wake on the other six controls is untested | Reserve suitable wake-capable pins for touch, physical controls and wheel events. |
| HW-002 | Keypad and ESP32-C6 share a supply | Verified prototype wiring | Keep essential input circuitry independently powered from optional radios and lighting. |
| HW-003 | Panel power removal must preserve the touch wake path | Rev A r6 cuts both panel rails in its soft power-off (2026-10-03) and comes back cleanly; touch wake with the panel off is untested, and TOUCH_RESET (PA44) has no pull resistor | Provide independent touch/wake and display power control, with defined off-state signal behavior. |
| HW-004 | SF32 battery-powered A3 and older 3.3 V E/G variants have different supply pins | Datasheet reviewed for Rev A | Use the exact ordered-part symbol and power tree; never substitute on the family name alone. |
| HW-005 | USB input current limits and programming access are separate design concerns | Rev A r6: the native USB port carries the application's HID link and the bootloader's recovery mode (`native/bootloader`, a full image flashed over USB-C on 2026-10-04); fixture pads remain for the flash table and bootloader | Connect native USB data to the MCU; provide fixture test points and qualify boot/suspend USB current. |
| HW-006 | 450 mAh capacity and MX1.25 pitch do not specify pack electrical limits or mating polarity | User confirmed nominal voltage/capacity; exact cell/connector still open | Qualify cell protection, charge/discharge current, temperature sensing and both harnesses. |
| HW-007 | USB power transitions must work while the screen and CPU are idle | CPU HardFault captured; firmware correction passed one user-confirmed unplug/touch cycle without restart | Validate rail continuity, retained controls and firmware recovery across USB removal/insertion at minimum idle load. |
| HW-008 | T-Watch PDM microphone shares SF32 pins with the external LF crystal | Datasheet/SDK pinmux verified for Rev A on 2026-09-13 | Use PA22/23 for PDM1, remove XTAL32K, validate calibrated internal LF-clock timing and idle power; power-gate microphone. |
| HW-009 | Encoder sits at a different height from the main PCB | User requires separate PCB and short single-sided FPC, sockets on both boards | Keep the wheel on its own board; retain input bias/filtering on main; qualify cable contact orientation and mechanical support. |
| HW-010 | Terminal voltage changes with charging/load and cannot establish accurate SOC alone | Prototype reports a 30%→55% jump on USB; shared estimator now preserves history | For accurate remaining capacity, provide a characterized cell model or a qualified fuel gauge with appropriate voltage/current/temperature sensing. |

## HW-001 — Reserve wake-capable pins before allocating other peripherals

### Verified constraint

| Function on the LilyGO prototype | MCU pin | Dedicated HCPU AON wake pin in the pinned SF32LB52 HAL? |
| --- | --- | --- |
| CST9220 touch interrupt | PA1 | No |
| TCA8418 keypad interrupt, including simulated PTT and wheel | PA11 | No |
| Supported dedicated wake range | PA24–PA44 | Yes at the HAL mapping level; confirm availability and restrictions for the selected package. |

For SF32LB525UC6, DS5201 V2.5 table 5-2 and the A3 hardware guide list only
PA24–PA27 and PA34–PA44 for standby/hibernate wake. Rev A uses PA24–PA27 for
the four side keys, PA37/PA38 for front macros, PA39/PA40 for wheel phases,
and PA43 for touch IRQ. This corrects the generic HAL assumption; physical
wake/event-retention validation is still required.

The pinned HAL's `HAL_HPAON_QueryWakeupPin()` accepts PA24–PA44. The SDK's
ordinary LIGHT/DEEP handlers mask normal interrupts and enable the AON interrupt.
Our working PA1/PA11 interrupt handlers therefore do not establish a wake path
from those modes. A timer or Bluetooth mailbox event could wake the CPU later,
but that does not guarantee capture of a short touch or immediate physical PTT.

This limitation concerns the documented AON sleep path. Ordinary WFI and the
SDK's separate deep-WFI clock-scaling path must be assessed separately; they
must not be treated as equivalent to full system sleep or hibernate.

The initial 2026-09-10 firmware enabled the SDK's guarded idle clock scaling
and capped HCPU sleep at IDLE, retaining ordinary GPIO interrupt handling.
After a USB-removal failure, the 2026-09-11 mitigation disables that separate
deep-WFI path while retaining 48 MHz run mode and ordinary WFI. The failure
reproduced with both configurations; a later capture found a CPU HardFault
and incorrect RAM placement of firmware clock-transition helpers. The RAM
placement correction passed a user-confirmed unplug/touch/reconnect cycle
with ordinary WFI and continuous uptime. Neither configuration resolves
HW-001 for full system sleep. Validation and remaining checks are recorded in
the [idle power audit](idle-power-audit.md).

The later five-hour test reported failed wake despite this IDLE ceiling. A
subsequent serial capture found a live application with the panel off; it did
not reproduce the original battery-only failure. The later physical wake pass
does not establish that PA1/PA11 support AON sleep or that the CPU entered it.
The firmware includes panel-only reset recovery and explicit run-clock votes;
long-duration wake and battery-current measurements remain necessary.

### Design requirements

- Allocate wake-capable pins to the touch interrupt and physical input wake
  sources before allocating optional peripherals. A GPIO interrupt capability
  alone is insufficient; verify wake support in the intended power mode.
- If a keypad/input expander is used, keep it powered during connected idle and
  route its interrupt to a suitable wake pin. Select an interrupt/event-retention
  behavior that preserves short presses until firmware can read them.
- For the six direct buttons (Enter, Cancel, Shift, PTT, Left, Right) and the
  wheel, provide wake detection for every one: the firmware wakes the screen on
  any of them, and PTT must also act from sleep. Retain enough state to deliver
  both press and release, including a press that ends before CPU resume
  completes.
- For the encoder, preserve the first detent and its direction. Reserve suitable
  wake inputs or a low-power event-capture circuit; waking on one phase alone
  does not prove that direction and step count survive sleep.
- Define interrupt polarity, pulse width or latched-level behavior, pull-up
  supply, debounce and reset defaults. Keep the selected wake circuitry and its
  bias supply alive during the chosen sleep mode.
- Check conflicts with flash/PSRAM, debug, boot functions and peripheral muxes.
  PA24–PA44 is a capability range, not a list of freely available pins.

### Acceptance evidence

Before layout release, record the selected MCU ball/pin, wake function, voltage
domain and sleep mode for each wake source. On the prototype PCB, verify wake
with USB disconnected and no host traffic: short taps, rapid PTT press/release,
both wheel directions and simultaneous inputs. Measure wake latency and check
for lost or stuck controls. Repeat with Bluetooth connected and disconnected,
and verify microphone/speaker startup after prolonged sleep.

Sources: [current IRQ setup](../native/src/board.c),
[pinned SF32LB52 wake mapping](https://github.com/OpenSiFli/SiFli-SDK/blob/006d03293e0621d694335cd4031773279fa2f19a/drivers/hal/bf0_hal_hpaon.c),
[pinned sleep handlers](https://github.com/OpenSiFli/SiFli-SDK/blob/006d03293e0621d694335cd4031773279fa2f19a/middleware/system/bf0_pm.c).

## HW-002 — Separate essential input power from optional peripherals

The LilyGO keyboard's `WIFI_3V3` rail supplies both the TCA8418 and ESP32-C6.
Firmware must assert `WIFI_EN` to use the keypad even though this product does
not use the ESP32-C6. The ESP32-C6 is currently held in reset; its actual residual
current has not been measured.

For our PCB, place the required wake/input circuitry on an independent supply.
Optional radios, key lighting, haptics and other peripherals should be separately
switchable if included. Specify regulator quiescent current, enable defaults
and signal isolation across disabled supplies. Do not power the only wake
controller from a rail that firmware intends to turn off during idle.

Verify that disabling each optional rail leaves touch, buttons and encoder wake
working, and measure current to detect back-power through GPIO or shared buses.

Sources: LilyGO T-SF32-Keyboard V1.0 schematic, available from the
[manufacturer's hardware repository](https://github.com/Xinyuan-LilyGO/T-Display-SF32/tree/master/hardware),
[keypad power sequence](../src/input/keypad.rs),
[expander assignments](../src/input/xl9555.rs).

## HW-003 — Allow display power-off while keeping touch wake available

The prototype exposes AMOLED supply control on PA44, but current firmware keeps
it enabled and uses panel Display Off / Sleep In commands. A complete rail-off
and resume sequence has not been validated, and its additional savings have not
been measured.

For our PCB/display assembly, provide independently controllable panel power
and a retained touch/wake supply. Confirm this separation in the selected panel
and touch-module specifications, including any shared rails inside the module.
Define reset/enable sequencing and GPIO, SPI and I2C states that prevent
back-powering when the panel is off. Expose rail test points or removable links
so panel and retained-touch current can be measured separately.

Acceptance requires waking by touch with panel power removed, then restoring
the panel and drawing a complete frame before display-on. Test repeated cycles,
short wake taps and power-on transients without flashes or spurious commands.

Sources: [prototype rail controls](../native/boards/openwalkietalkie/power.c),
[panel sleep/wake implementation](../native/src/board.c).

## Open design decisions

- Validate the proposed SF32LB525UC6 package and Rev A wake-pin allocation.
- Connected-idle sleep mode and acceptable control/audio wake latency.
- Validate direct GPIO event capture for all buttons and both encoder phases.
- Panel/touch assembly power-domain separation and regulator selection.
- Battery architecture, verified cell ratings and current-measurement access.
  The keypad cell capacity remains unverified; avoid using an assumed combined
  capacity to size the runtime budget.

## HW-007 — Qualify USB removal at minimum load

The prototype lost its Bluetooth application link after USB removal with the
screen asleep, then restarted after USB was reconnected. This was captured
over Bluetooth without opening serial, and the user confirmed no power-switch
operation. A subsequent CPU capture found a HardFault during the wake clock
transition; ELF inspection found clock helpers incorrectly executing from
external NOR while its clock was changing. After the RAM placement correction,
the user confirmed a successful unplug/touch/reconnect cycle; Bluetooth uptime
was continuous and the CPU capture had no fault or lockup. The saved charger
watchdog fault explains the earlier automatic power cycle; no regulator defect
has been established. Rev A still needs its own electrical qualification.

For Rev A, provide access to battery, charger SYS, MCU supply, peripheral rails,
charger nINT and reset for simultaneous capture. Verify unplug/insertion with
the screen asleep, minimum CPU load, both low and full battery, and touch/PTT
activity during the transition. Confirm supply margins and continuous uptime,
or a deliberate recovery with usable input/display and recorded reset cause.
Include independent SWD access so debugging does not depend on restoring USB
power. Keep pre-initialization charger fault evidence available after a reset.

## Additional Rev A constraints

**HW-010:** The prototype firmware measures cell voltage and charger status,
without measured cell current or coulomb counting. USB insertion can raise
terminal voltage through charging and load relief, making a direct discharge
curve lookup report a large SOC increase. Firmware now stabilizes the displayed
estimate across transitions, but that does not establish absolute accuracy.
For the custom PCB, evaluate a low-power fuel gauge and its sensing/temperature
requirements against the selected cell, including connected-idle quiescent
current. Qualify charge/discharge SOC error across load, temperature, aging
and USB transitions. Preserve the cell-specific calibration/provisioning data;
configured charger current is not measured battery current. See the
[estimator investigation](battery-estimation.md).

**HW-008:** The selected T-Watch microphone is SPM1423HM4H-B, a digital PDM
device, not an I2S PCM or analog output. SF32LB525UC6 routes PDM1_CLK/DATA
to PA22/23, shared with XTAL32K; the other pair, PA07/08, is used by the
quad-SPI display. Rev A removes the external 32.768 kHz crystal and keeps
the 48 MHz crystal. The board firmware must configure/calibrate the internal
LF clock and validate drift, Bluetooth timing and connected-idle power
across temperature. If an external LF crystal becomes mandatory, revisit
the pin allocation/interface or microphone selection before routing.

The microphone uses switched retained 3.3 V (TPS22918, PA28 enable, off at
reset), rising-edge receive with SELECT grounded, and a valid 1.0–3.25 MHz
PDM clock. Stop clock and prevent signal back-power before cutting its rail.
Account for its top acoustic port, local C0G bypass area and speaker
vibration isolation. A separate Rev A PDM firmware port is required; the
LilyGO analog capture driver remains appropriate only to the prototype.
Acceptance includes silent idle current, capture startup, acoustic quality
and sleep/reconnect tests. See the [current audio source](../hardware/rev-a/pcb-release-bodyv5-20260917-r3/source/main/src/audio.cohdl).

The speaker amplifier's inputs are a summing node: anything coupled into
the trace between the input resistors and the NS4150B is amplified by its
fixed 300 kΩ feedback whatever the gain resistors are. Place R4/R5 at the
amplifier's input pins, keep AMP_INP/AMP_INN short and away from the flash
(MPI2) and panel QSPI lines with ground between, and never let them cross
those lines on an adjacent layer. BodyV6.13 r6 routes AMP_INN across four
flash lines. Rev A retains the PSRAM execution workaround: code, constants
and fonts are copied to PSRAM, with interrupt vectors in SRAM. Removing it
after the clock correction led to hiss recurring in the 2026-10-20 listening
test (hardware/rev-a/README.md). Rev B and LilyGo execute directly from NOR.

**HW-009:** The Alps EC05E1220401 is on a physically separate raised PCB;
the main board retains A/B pull-ups, series resistors and filters connected
to PA39/PA40. Both boards use four-contact Hirose FH34SRJ-4S-0.5SH(99)
sockets: pin 1 GND, pin 2 A, pin 3 B, pin 4 GND. No supply crosses it.
Finalize socket direction, contact exposure, end stiffeners, bend radius,
length, latch access and support after the mechanical design supplies the
height and offsets. Verify pin-number continuity; single-sided construction
alone does not define the end mapping. Support the wheel mechanically so
the flex does not carry actuation loads. Check first-detent wake and fast
rotation with the final cable. See the [current assembly contract](../hardware/rev-a/pcb-release-bodyv5-20260917-r3/README.md#assembly-changes).

**HW-004:** On SF32LB525UC6, pin 24 is VCC rather than PA21; pin 23 is the
internal PSRAM 1.8 V supply. Charger SYS feeds MCU VCC, while the MCU's own
VSYS output supplies RF. AVDD33_AUD uses VDD33_VOUT1, not MCU VSYS. Keep
these nets distinct. The registry's E/G symbol is not an electrical substitute.

**HW-005:** USB-C D+/D− connect to PA35/PA36 through ESD protection. Removing
the onboard flash/debug interface means an external UART/SWD fixture through
test points, not removal of program NOR. PA34 service reset is active high;
charger nINT is BAT-domain. Passive CC Rd resistors do not enforce a safe
pre-enumeration current limit; the selected SGM41562SA requires board-specific
firmware configuration and input-current qualification. See its
[migration note](../hardware/rev-a/pcb-release-bodyv5-20260917-r3/source/main/docs/charger-qfn-migration.md).

**HW-006:** The new cell is 3.7 V nominal / 450 mAh. Charge voltage, current,
temperature window, protection and peak speaker current need its datasheet.
Two-contact MX1.25 does not carry an NTC signal. Confirm connector families,
polarity and current ratings; both BTL speaker leads are active. A 4 Ω / 3 W
speaker rating does not promise 3 W from a single-cell amplifier supply.

Evidence and unresolved implementation details:
[current assembly/power notes](../hardware/rev-a/pcb-release-bodyv5-20260917-r3/README.md#fabrication-and-assembly-settings),
[authoritative pin/net source](../hardware/rev-a/pcb-release-bodyv5-20260917-r3/source/main/src/main.cohdl),
[manufacturer and mechanical inputs](../hardware/rev-a/mechanical-source/README.md).

When resolving a constraint, record the schematic revision, selected parts/pins
and measured acceptance results here. Keep the original prototype limitation
for context rather than deleting it after the new hardware is built.
