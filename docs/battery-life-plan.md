# Battery life plan

Follow-up: the user reports approximately 9 hours of mostly screen-off idle
runtime. See the [2026-09-10 idle power investigation](idle-power-audit.md) for
current-device evidence, the HCPU wake-pin constraint and the next optimization
sequence.

## Product behavior

Use the approved defaults: dim after 30 seconds without local input and switch
the screen off after 2 minutes. Touch or a physical control restores brightness.
The first contact on a dark touchscreen only wakes it; its complete gesture is
consumed, so waking cannot accept a request or start recording accidentally.
Physical PTT wakes and acts, so the radio works blind. Every other button and
wheel detent pressed while the screen is dark, or before the wake frame is on
the glass, only wakes; its release is consumed too, so a blind press cannot
answer a card or move a list. Held controls keep the screen awake. System
RPC requests, deck messages and battery updates do not reset the
inactivity timer. A new Agent Command Deck card lights the screen for 15 seconds
without counting as input (`Policy::notify`): it neither resets the inactivity
timer nor delays idle-home, and the built-in demo's notices never wake the
screen. The lit level follows Settings brightness 1–5 (40/80/130/190/255);
dim is 32, or the lit level if lower. Audio and Bluetooth connections remain
available with the screen off. Use the requested dark theme: black canvas,
charcoal keys, light Lucide glyphs and purple accents, preserving geometry and
host lighting. Keep the CoHDL boot splash.

## Baseline findings

- The CO5300 is set to brightness 255 continuously. There is no idle dim/off path.
- Native firmware wakes its application loop every 5 ms and reads the keypad
  FIFO on every iteration, even with no input. Touch has an IRQ latch plus a
  20 ms fallback, which is needed to recover missed short interrupt pulses.
- Lighting animation is calculated every 80 ms and can redraw the entire screen
  indefinitely. Battery ADC and charger transactions run every 5 seconds.
- The XL9555 enables the unused physical keypad LED supply. The keypad shares
  its supply with the Wi-Fi module, so cutting that shared rail would lose input.
- The SDK already executes CPU idle instructions when threads block, and its
  Classic connection manager already supports sniff mode. The speaker amplifier
  already shuts down between sessions, including early A2DP suspension.
- The native target has no validated system deep-sleep/resume path for the
  directly driven LCD, PSRAM and audio. Display sleep and CPU idle are separate.

## Implementation sequence

1. Switch both renderers to the shared dark OLED palette. Add a shared,
   clock-wrap-safe active/dim/off policy with tests for timeout
   boundaries, waking, held PTT, cancelled contacts and synthetic releases.
2. Add CO5300 dim/sleep/wake operations. Rebuild the current frame while hidden
   before enabling the screen. Stop rendering and lighting animation while off;
   continue accepting host configuration and battery state for the next frame.
3. Replace the native fixed 5 ms idle loop with an interrupt/message-woken wait.
   Keep fast scanning while a contact/key is active, retain fallback polling,
   and use a slower idle fallback. Avoid losing releases or starving HID replies.
4. Leave unused keypad lighting off without changing the shared keypad supply,
   reset states, charger limits or existing audio shutdown behavior. Verify the
   manufacturer's wiring before changing supply controls.
5. Reduce battery sampling/watchdog service to 15 seconds, comfortably inside
   the charger's existing 64-second watchdog. The pinned ADC driver already
   disables its VBAT monitor after each reading; verify that behavior on-device.
   Keep battery/status reporting live.
6. Use the native SDK's 30-second fast BLE discovery phase at a 60 ms interval,
   followed by indefinite background advertising at 1 second. Disconnection
   restarts fast discovery. Preserve connected HID and Classic audio parameters.
7. Add bounded power diagnostics for state transitions, rendering and polling,
   then build, test and flash. Exercise idle transitions, wake, audio while dark,
   and charging service. Document observed results separately from estimates.

## Validation and acceptance

- Host tests cover timeout boundaries and 32-bit timer wrap, wake-only touch,
  hardware PTT press/release, cancellation and host updates while sleeping.
- Both firmware targets build. Native image passes flash-boundary checks and
  flashes with verification while preserving bootloader and Bluetooth bonds.
- Device logs show dim at 30 seconds and off at 120 seconds, no display transfers
  while off, and continued charger service. Waking redraws the current UI.
- A silent speaker open/close and microphone capture with the display off verify
  audio remains usable; physical touch and PTT need observation on the device.
- Measure each cell with a power analyzer, USB disconnected: active static UI,
  animated UI, dim idle, dark connected idle, disconnected idle, microphone and
  speaker playback. Include the keypad module's separate battery in the budget.
  Calculate runtime from measured usable capacity and workload-weighted current.
  Battery voltage and software counters alone cannot establish a runtime gain.

## Results on 2026-09-09

- Implemented the shared dark theme, inactivity policy, panel sleep/wake,
  interrupt-based input waits, disabled unused keypad lighting and reduced
  battery sampling in both targets. The native target also uses the SDK's
  background advertising and emits bounded power diagnostics.
- All 62 native Rust core tests and 58 shared host tests pass. Both release
  targets build. The native image contains 1,062,588 load bytes, passes the flash
  boundary check and was flashed with verification, preserving bond storage.
- Device logs show dim at 30,017 ms and off at 120,022 ms after boot. Idle
  30-second windows while off show zero display bands, approximately 1,800 I2C
  transactions, two successful charger watchdog feeds, zero feed errors and the
  VBAT monitor disabled. These are software activity counts, not current readings.
- Both touch and keypad interrupts initialize successfully. BLE advertising
  changes from 60 ms to 1 second after the initial discovery window. Mac HFP and
  A2DP profiles reconnect after flashing.
- With the screen off, a five-second Mac microphone probe received 81,920
  samples with nonzero data without saving audio. Silent playback open/close
  succeeds; after HFP closes, the amplifier disables and the Mac returns to
  44.1 kHz stereo output. A second silent A2DP session also suspends and disables
  the amplifier. SDK voice buffer/drop warnings occurred during this check;
  these checks establish stream availability, not speech quality or radio
  stability.
- Physical wake-only touch behavior and PTT still need user observation. BLE
  HID did not reconnect in the initial run because its renewed macOS pairing
  prompt from the earlier appearance revision was pending. Host status while asleep and
  wake-only input behavior pass the native core regression tests.
- After the user completed pairing, encrypted HID (then still the Codex
  Micro emulation, since removed) resumed alongside HFP/A2DP. All three reconnected automatically after flashing
  the pad-alignment correction (1,062,524 load bytes). The rendered idle and
  swipe previews show concentric circles on both gesture pads. Device logs also
  confirm a complete hidden-frame refresh on wake. System Settings still omits
  the working speaker from its Output list, while Control Center shows it selected;
  this separate listing issue remains unresolved.
- Battery current and discharge runtime have not been measured. System deep
  sleep and rail removal described below remain follow-up work.

## Further hardware-dependent work

See [hardware design constraints](hardware-design-constraints.md) for the
dedicated PCB reference covering wake pins and input/display power domains.

The [idle-power follow-up](idle-power-audit.md#follow-up-implementation) enables
guarded HCPU idle clock scaling while preserving full active performance and
ordinary GPIO interrupts. Full system sleep and powering down the AMOLED supply
still need validation of RAM/PSRAM retention, DMA exclusion, touch/key wake
sources and simultaneous Bluetooth audio. Enable them only after those resume
paths pass. A fuel gauge and verified low-cell shutdown/restart thresholds remain
board-level follow-ups.

References: [SiFli power management](https://docs.sifli.com/projects/sdk/latest/en/sf32lb52x/middleware/pm.html),
[SF32LB52 PM example](https://docs.sifli.com/projects/sdk/latest/en/sf32lb52x/example/pm/classical/README.html),
[LilyGo hardware and power tests](https://github.com/Xinyuan-LilyGO/T-Display-SF32).
Implementation decisions also use the project's pinned SDK and board drivers.
