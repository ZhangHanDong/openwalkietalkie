# Battery state-of-charge estimation

## USB insertion jump (2026-09-15)

The reported symptom was **30% on battery, then 55% on USB insertion**.
Both firmware targets converted each measured terminal voltage directly
through the same generic single-cell discharge curve. The native target had
no temporal filter; the Embassy target averaged voltage but still followed
the charging voltage upward. Neither retained a state-of-charge estimate.

The existing lookup maps 3750 mV to 30% and 3875 mV to 55%. This illustrates
how a 125 mV shift can reproduce the report; those two voltages are a test
case, not readings captured during the user's transition. USB load relief
and charging can change terminal voltage without an equivalent change in
stored charge. Smoothing voltage alone does not remove that bias.

A read-only Bluetooth diagnostic before the update reported uptime
28,461,410 ms, VBAT 3966 mV, charger REG08=0x12 (charging), no current charger
faults and no diagnostic read errors. This confirms the device was charging
without a recent reset; it does not reconstruct the earlier insertion event.

## Implemented behavior

The shared `src/battery/state.rs::Estimator` now supplies the percentage used
by the UI, `device.status` replies and Bluetooth battery notifications in both
targets. Raw calibrated voltage remains available separately for diagnostics.

- Preserve the previous estimate through charger-state transitions and hold
  it for two minutes while voltage settles. Charging indication updates on
  the next normal 15-second sample without waiting for this hold.
- Filter valid voltage samples, then use the voltage lookup as a target.
  Move toward it at no more than one internal percentage point per minute;
  retain the existing 5% display granularity. This is a presentation/tracking
  bound, **not** an assumed charge current or a capacity integration model.
- Allow increases only when the charger reports charging. On battery, or
  USB present with charging inactive, allow decreases but no increases from
  voltage rebound. A missing charger status holds the last estimate.
- Cap charging progress at 95% until a genuine charge-done event is followed
  by at least three fresh voltage samples within 80 mV of the configured
  endpoint, spanning 30 seconds. Near full, read raw charger status every
  second and take an extra ADC sample on each new Done event. Fast-charge
  status after Done no longer discards the event; pre-charge, inactive/input
  loss, unreliable reads, a voltage dip, changed endpoint or a status gap over
  2.5 seconds cancels the pending confirmation. Confirmed completion anchors
  the estimate at 100%, retained on immediate unplug.
- If valid voltage remains at or below 3400 mV for 30 seconds, reduce the
  estimate to at most 5% immediately. One short dip does not trigger this
  override. This is an indicator policy, not battery protection or shutdown.
- Invalid voltage shows unknown rather than an old percentage. Retain
  history internally for recovery; missing samples and gaps over one minute
  restart settling and cannot accumulate unobserved charging credit.
- Use elapsed milliseconds, including 32-bit timer wrap. Stationary readings
  cannot accumulate a budget that permits a later jump.

The estimator does not alter charger current, termination voltage, watchdog
or protection configuration. The native polling path now reads back the
configured termination voltage as the Embassy target already did.

Native UART diagnostics include `uptime_ms`, `mv`, `voltage_pct`, `soc_pct`
and `charge` each battery sample. Charge codes are 0 unknown, 1 battery,
2 USB without charging, 3 charging, 4 charge complete; percentage 255 means
unavailable. `voltage_pct` is the instantaneous uncorrected lookup, while
`soc_pct` is the value shared by display and host. Logging does not wake the
screen. Opening the prototype's USB serial bridge can pulse reset; prefer
an already-open monitor for continuity tests.

## Accuracy limits and next calibration step

This fixes abrupt changes and prevents noncharging USB from inventing stored
charge. It does **not** prove that either the original 30% or 55% represents
true remaining capacity. Charging voltage still biases the target, and
monotonic tracking may retain an underestimate after a heavy load. A cold
boot has no prior estimate, so it starts from voltage; there is no SOC
persistence or additional flash wear.

This board's implemented sensing provides voltage and charger state, not
measured cell current, coulomb counts or battery temperature. Do not integrate
the configured 344 mA as though it were measured current: charge taper,
thermal/input regulation and actual capacity invalidate that assumption.
The uncharacterized generic voltage curve remains approximate.

For absolute accuracy, characterize the actual display-unit cell with a
measured discharge under representative loads and temperature, or add a
qualified fuel gauge on the custom board. Do not use the separately specified
450 mAh custom-board cell as the prototype's measured capacity. This also
means percentage loss over five hours is not a reliable capacity or runtime
measurement by itself. See hardware constraint HW-010.

Primary background: TI's [State of Charge Estimation Using Smart Battery
Charger](https://www.ti.com/lit/an/sluaaa1/sluaaa1.pdf), sections 2–5, describes
cell characterization and the limits of terminal-voltage lookup under varying
loads, temperature and voltage relaxation. Its example table is not used as
calibration data for this cell.

## Verification

All 66 shared host tests and 80 native-core tests pass. Both firmware images
build; the native image contains 1,097,488 load bytes and passes the partition
boundary and all 15 SRAM clock-helper checks. It was flashed using verified
application-only writes, preserving bootloader and bond storage. The first
bootloader connection timed out; the existing retry completed successfully.

The post-flash UART capture shows three consecutive readings at uptime
46,688–76,746 ms: 4160–4166 mV, reported 95%, charging active, watchdog
serviced and no reported I2C errors. This verifies the new estimator is
running; it is not a battery-capacity calibration.

Regressions cover 30→55 voltage jumps, noncharging USB, load relief,
unplugging, gradual charge/discharge, full confirmation, low-voltage dips,
invalid readings, sampling gaps and timer wrap. A native polling integration
test verifies the UI and host getter share the stabilized estimate. Existing
render tests exercise partial battery updates against complete frames.

The user completed a physical USB unplug/replug test and confirmed the
percentage **stays stable**. A concurrent Bluetooth HID capture recorded
battery-only status (REG08=0x00), VBAT down to 4037 mV, then charging status
(REG08=0x12) and VBAT 4168 mV after reconnect. Uptime remained continuous
and HID replies had no read errors; charging resumed without a reset.
A measured full discharge is still required to quantify absolute SOC error.

Build, flash and live captures are in the ignored workspace build cache:
`target/soc-native-build.log`, `target/soc-native-flash.log`,
`target/soc-native-live.log` and `target/soc-native-hid.log`.
The restored, pinned build environment can be reused with
`source target/soc-build-env.sh` before `tools/build-native.sh`.

## Investigation: prolonged charging at 95% (2026-09-17)

Read-only Bluetooth diagnostics captured 104 successful replies over a
227-second interval, without opening USB serial, rebooting, or changing charger
settings. Device uptime advanced continuously from 4,175,856 to 4,402,678 ms.
The raw captures are `target/soc-95-diagnostic.log` and
`target/soc-95-extended.log`.

- REG08 was `0x12` (charging) in 101 replies, `0x0a` (pre-charge) in two, and
  `0x1a` (charge done) in one. Power-good was set throughout the captures.
- The done sample at uptime 4,383,314 ms was bracketed by charging at
  4,381,174 and 4,385,424 ms. This sampled completion was brief; it cannot
  satisfy the estimator's 30-second uninterrupted Full requirement, and the
  application's 15-second battery polling can miss it entirely.
- REG09 was zero and all diagnostic I2C reads succeeded in every captured
  reply. No sampled power-management or thermal-regulation flag was set.
  Fault reads can clear flags; these observations do not rule out events
  between samples.
- Cached ADC readings were usually 4187–4189 mV, but included single updates
  at 3874 and 3729 mV. Each value remains cached for roughly 15 seconds;
  repeated diagnostic replies do not establish that a physical dip lasted
  that long. No current measurement is available.

The immediate display cause is confirmed: Charging is capped at 95%, and a
brief Done report does not qualify for 100%. The existing full-confirmation
regression passes: at 4190 mV, three Full samples spanning 30 seconds set 100%
immediately, bypassing ordinary settling and percentage smoothing. The source
correctly decodes charge status and configures termination enabled at 31 mA;
the configured voltage is 4200 mV. The current diagnostic protocol does not
read live REG03/04/05, so those configuration values come from the verified
initialization code, not this capture.

The reason for brief completion/restarting is not established. The vendor's
[SGM41562S datasheet](https://www.sg-micro.com/rect/assets/213b1edb-e9a1-4f49-9b20-6f1330d7c3ce/SGM41562S.pdf)
requires charge current to taper below the termination threshold. Normal
recharge hysteresis alone does not explain pre-charge at a cached 4.19 V:
pre-charge applies below the configured 3.0 V threshold. ADC timing and a
physical battery/connection issue remain possible. Do not treat the displayed
95% as evidence that exactly 5% capacity remains, or relax the completion
filter solely to hide these inconsistent observations. Charging duration,
USB source, battery-only operation, and direct battery voltage/current checks
are the remaining discriminating observations. No firmware policy or charger
settings were changed during this investigation.

### Completion reporting update

The follow-up implementation separates charger-event observation from fresh
voltage samples. Normal ADC conversion and watchdog service keep their separate
15-second deadline. One-second REG08 reads run only while near full on USB or
while a completion candidate is pending. A Done transition requests a real ADC
conversion immediately; status-only polls never feed cached voltage into the
estimator. Extra conversions do not postpone watchdog servicing. The UI/host
charge state continues to describe the charger, even when SOC has reached 100%.

The estimator requires clean power-good Fast/Done observations, fresh qualifying
voltage and three distinct voltage samples over at least 30 seconds after Done.
The 30-second interval begins with the qualified Done event, not when voltage
first becomes high. Confirmation occurs on a fresh conversion, normally within
30–45 seconds after the event. Repeated Done transitions do not restart its
timer. No voltage-only fallback is used to force 100%, and no charger current,
voltage, termination, watchdog or protection register settings were changed.

The native diagnostic protocol now also supports `--battery`, which reads
REG00–0C, the cached measured voltage, reported SOC/charge, configured endpoint
and completion candidate age. Each register read has an error bit. The existing
v2 status/wake replies are unchanged; this is a new read-only operation on the
same version. Reading REG09 may clear latched fault bits.

All 101 native core and 80 shared host tests pass, including the captured
Done-then-low-voltage pattern, pending-event cancellation, fast-status polling,
fresh ADC on Done, independent watchdog deadlines, and timer wrap. Both firmware
targets build, and the native ELF passes application partition and SRAM
clock-helper checks.

### Live validation and remaining battery check

The completion update was flashed with verification. A 60-second UART capture
and three-minute Bluetooth battery capture showed that the new polling catches
Done events between scheduled battery conversions. The immediate, successful
ADC conversions returned **3961 mV at uptime 13,762 ms** and **3750 mV at
43,970 ms** with charge state Full. Both are below the 4120 mV qualification
threshold, so the candidate was correctly rejected and SOC stayed at 95%.
Other fresh readings were around 4187–4189 mV. These low readings are not just
stale voltages paired with a later status: they were obtained on the Done event.

Live readback confirmed REG02=`2a` (344 mA), REG03=`9f` (termination code15),
REG04=`8d` (4200 mV), REG05=`3c` (termination enabled, original watchdog/safety
configuration), and REG0C=`7a` (500 mA input limit). The captures reported no
charger faults or I2C read errors. Bluetooth reconnected and the UI booted.
Raw data is in `target/soc-completion-live.log` and
`target/soc-completion-hid.log`. This validates the reporting behavior, not a
successful physical full charge.

Source review also found that the SDK's `rt_adc_read()` discards the conversion
error and can return an uninitialized value on timeout. The native wrapper now
calls the same conversion callback with initialized storage, checks its return
status and range, and reports unknown on failure. The driver retains its
locking, calibration and averaging; charger settings are untouched. The captured
low readings had successful SDK conversion logs, so this separate error-path
fix does **not** explain those particular lows. The final native image has
1,098,464 load bytes and passes the same build checks. It was flashed with
verification and booted with encrypted HID, HFP and A2DP reconnected; a further
Done event measured 3931 mV and was rejected. The final 35-second capture is
`target/soc-completion-final-live.log`, with no ADC failure or display fault.

The charger/BAT voltage cycling remains unresolved. The firmware deliberately
does not convert these low-voltage Done events into a false 100% indication.

### Battery-only wake confirmed

The user confirmed that unplugging USB leaves the device powered and that
touch wakes it normally. This rules out a completely absent battery power
source; it does not by itself validate the charging path, contact resistance,
capacity or voltage measurement.

A subsequent read-only Bluetooth capture (`target/soc-after-unplug-test.log`)
contained 29 replies over 59.2 seconds, with USB restored: 4187–4188 mV, SOC95,
REG08=`12` (charging), no faults or diagnostic read errors, and no pending
completion candidate. No completion transition was captured in this interval.

Further source review found no demonstrated successful-conversion defect:
native slot7 selection and units are correct, reference/ADC settling each meet
the documented 200 microseconds, and the SDK trims the minimum/maximum from
22 conversions before averaging the remaining20. A hypothetical stale EOC
after an earlier timeout is not established here; the low samples had successful
conversion logs and no recorded timeout. The hardware activity guard prevents
deep-WFI clock scaling during the conversion; it is not a general lock on all
scenario-based run-clock changes.

The [SiFli hardware guide](https://github.com/OpenSiFli/SiFli-Wiki/blob/main/source/en/hardware/SF32LB520-3-5-7-HW-Application.md)
describes separate battery VBAT sensing and approximately +/-30 mV accuracy.
The much larger captured changes therefore should not be dismissed as ordinary
ADC tolerance. The published LilyGO schematic has inconsistent BAT/VBAT labels
at the battery connector, so it cannot establish every physical connection.
Battery connector identification and a direct cell/BAT-node voltage comparison
during completion remain the next discriminating checks. No additional firmware
or charger setting was changed for this follow-up.

## Rev A: no charger status

The r6 board's BQ24073 has no bus and its CHG output reaches only test pad
TP1, so `hw-a` builds never observe a charge state. They construct the
estimator with `Estimator::voltage_only()`: an `Unknown` charge state then
follows the filtered voltage curve in both directions at the same 1 %/minute
slew, instead of holding as it does on the dev board when a charger read
fails. Consequences: the figure rises while charging only as fast as the
terminal voltage does, reads a few percent high while USB is plugged in
(charging polarization), never shows a charging or full indication, and 100
means the top of the curve, not a confirmed termination. The sustained
low-voltage clamp still applies. The charge codes reported to the host stay
0 (unknown) on Rev A.
