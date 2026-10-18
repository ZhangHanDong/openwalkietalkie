# OpenWalkieTalkie hardware — BodyV5 r3

This directory keeps one current PCB design: the verified **BodyV5 r3**
four-pin inter-board revision, dated 2026-09-17. Superseded schematics, layouts,
manufacturing snapshots and BodyV4 assemblies are no longer part of this tree.

## BodyV6.13 r6 (2026-09-23) — current ECO under review

[pcb-release-bodyv6.13-20260923-r6](pcb-release-bodyv6.13-20260923-r6/README.md)
adapts r5 to the **BodyV6.13** case (`mechanical-source/case/BodyV6.13.step`).
Main-board outline only (copper and parts identical to r5): the right-edge
recess now clears the new wall boss at v 49.6–54.4, the left-rib relief is
deeper, and the seat-block notch is re-cut concentric with the block (r5's
cut left 0.16 mm there, not the 0.24 mm its README states). Main board native
KiCad 10.0.4 DRC 0/0; encoder board identical to r4. The V6.13 export has no
back cover, so cover-related fit checks are open. Updated in place later on
the same day with an open U-slot on the right edge for an M2 screw into the
V6.13 boss at (52.2, 52.0), and with the two sharp tips beside the top-left
pillar slot removed (DRC 0/0 again, all main-board outputs re-exported).
The encoder board's MH2 (the hole around the case spigot) was then enlarged
from Ø2.6 to Ø2.8 mm; MH1 stays Ø2.6 for the M2 screw into the seat block
(encoder DRC 0/0, outputs re-exported). Last, the microphone MK1 (SPM1423HM4H-B,
out of stock at JLCPCB) was replaced by the top-port PDM **ST MP34DT05TR-A**
on the front (B) side inside the case sound post, with a local re-route of the
main board (DRC 0/0; Gerbers, drill, STEP, fit model and the `fab-jlcpcb/`
package regenerated). Datasheet: `mechanical-source/datasheets/mp34dt05-a.pdf`.
Then the charger U6 (SGM41562SA, also out of stock) was replaced by the
standalone **TI BQ24073RGTR** (no I²C; 349 mA charge, 500 mA input limit,
4.4 V system rail), again with a local re-route, DRC 0/0 and a full re-export.
It needs a **10k battery NTC** instead of the 100k one. The firmware's `hw-a`
builds skip the SGM41562 driver: the BQ24073 has no bus, so charging runs on
its own and the charge state is not visible to firmware.
On 2026-09-24 the out-of-stock fixed 3.3 V regulator U7 (RT6150B-33GQW) was
replaced by the adjustable RT6150BGQW that U8 already uses, with a new 1M /
180k feedback divider on the B side (IO_3V3 ≈ 3.28 V); DRC 0/0 and all
main-board outputs re-exported. The same day, silkscreen pin-1 dots were added
next to the ten ICs that had none (U1–U5, U7, U8, U10, U20, Y1); only the
silkscreen layers changed (DRC 0/0, Gerbers and Gerber zips rebuilt).
An alternative encoder board, `encoder-wheel/`, puts the encoder on the wheel
side inside a cavity in the wheel. It needs the modified wheel in
`encoder-wheel/mechanical/`, and the wheel's hex should be about 1.68 across
flats for either board.
The first assembled r6 main board was brought up on 2026-10-03: every rail
measured as designed, and the board was flashed over UART1 on TP6/TP8 (a
DAPLink's UART side, `sftool --compat true`) with the repo-built flash table
and bootloader (`OMD_BOOT_IMAGES=1`) plus the `hw-a` firmware, which boots,
drives the panel rails and advertises over Bluetooth. With the module first
fitted the glass stayed black and the CST9220 touch controller did not answer
on I²C although PANEL_4V and PANEL_LOGIC were present; reseating the 31-pin
FPC fixed both. (`display.TP_VCC` is the module's own 3.3 V touch-supply
output, so leaving it unconnected is correct.) The same day the firmware's
USB HID link was brought up over J5: the Mac enumerates the deck on PA35/PA36
once macOS's accessory prompt is allowed, and `deckd` syncs over the cable
with 0.7 ms report writes; the USB wiring needed no change and the
unconnected MCU VBUS pin did not matter. With the native image the speaker
played the quiet channel checks audibly and the MP34DT05 microphone captured
speech over the voice link through PDM1 (power switched on PA28), but a
full-scale tone at maximum volume with the board on USB and no cell fitted
reset the MCU after about a second: the charger's 500 mA input limit cannot
carry the NS4150B's peak draw, so SYSTEM_SUPPLY drops out. Loud tests need
the cell; a volume ceiling for cell-less operation is open.
Earlier bench tests found speaker hiss at every volume setting. The original
diagnosis and temporary workaround below are historical: the clock-configuration
correction subsequently resolved the issue. As of 2026-10-17 the firmware
executes directly from NOR again, with no code/font copy into PSRAM.

The original diagnosis excluded the DAC
(measured 2026-10-04/05 with the deck's own microphone through the
`audio-debug` bench RPCs, R4/R5 22 kΩ as released): it is a train of clicks, one per external-flash read burst, coupled
into the amplifier's input summing node. AMP_INN runs on In2.Cu across
FLASH_CLK, FLASH_IO0, FLASH_IO2 and FLASH_IO3, so every execute-in-place
cache fill from the NOR clicks; with the radio busy the Bluetooth host stack
runs at each 15 ms connection event and the clicks form a 66.7 Hz comb that
sounds like hiss. Controlled tests: flash-read bursts at 66 Hz with the radio
quiet reproduce it (−57 dBFS(A) at the mic for 256 lines a burst) while the
same bursts from SRAM or PSRAM, or with the amplifier off, stay at the floor
(−80 to −83); no DAC or codec setting (mute, DAC/AMP/VCM enables, chopper,
low-power and bias modes) and no radio TX power change moved it, and halving
the flash pads' drive strength bought only 3 dB. Because the injection is at
the summing node, its level is set by the NS4150B's 300 kΩ feedback, not by
R_in: changing R4/R5 would not reduce this noise. The temporary workaround,
in `hw-a` builds from 2026-10-05, ran the code from the in-package PSRAM,
whose bus never reaches the PCB. The former linker script placed `.text`
and `.rodata` at 0x60000000, and the pre-init hook copied and verified them
and moved the vector table to SRAM. That workaround has now been removed. Speaker
on, silence streamed over BLE: −60 → −81.9 dBFS(A) (speaker off −82.1); BLE
mic loopback over silence on the shipping image −59 → −81.7; USB RPC traffic
−64…−69 → −79…−81. For the next revision: put R4/R5 at the NS4150B's input
pins so the high-impedance AMP_INP/AMP_INN stubs are millimetres long, and
route the amplifier input pair away from the flash (MPI2) and panel QSPI
lines, with a ground plane between them, never crossing on an adjacent layer.
The board has no power switch and the IO_3V3 regulator's enable is tied to
SYSTEM_SUPPLY, so with a cell on J3 the MCU runs until firmware stops it.
On 2026-10-03 the `hw-a` firmware gained a soft power-off (hold CANCEL 3 s):
it unpowers the panel and enters the SF32LB52x PMU's shutdown state with
PA26 (CANCEL) and PA24 (PTT) armed as the two PMU wake pins through their
RTC-domain PBR pads; IO_3V3, the touch controller, the control pull-ups and
the BQ24073 stay up, and USB still charges the cell. Verified the same day
on the first r6 board (chip revision A3): two off/on cycles, one woken by
CANCEL and one by PTT, each a clean cold boot through the bootloader. Since
2026-10-10 only PA24 (PTT) is armed, and the firmware goes back to sleep
unless PTT is still down 2 s after the application starts
(`pmu::POWER_ON_HOLD_MS`); CANCEL no longer wakes the deck.
The same day the bootloader in the 64 KB slot at 0x12010000 was replaced by
`native/bootloader`, the SDK's second stage plus a USB recovery mode (on J5,
as the application's own "Codex Micro" HID device, so no host prompt) entered
by holding SHIFT and CANCEL at power-on, by an invalid application image, or
at the application's request; `tools/flash-usb.py` flashes through it (a full
image in about 32 s, verified 2026-10-04 including an erased vector table),
so TP6/TP8 are needed only for the flash table and the bootloader itself. Two things to measure
and carry into the next revision: the off-state cell current at TP2 with USB
removed (nothing in this release states the RT6150B, BQ24073 or TPS22918
quiescent figures, and the panel's QSPI lines float at its inputs while
IOVCC stays on), and TOUCH_RESET (PA44), the one control line with no pull
resistor, which floats in hibernate while the CST9220 stays powered from
IO_3V3; a 100 kΩ pull-down there would hold the controller in reset when the
deck is off.

## BodyV6.9 r5 (2026-09-23) — superseded by r6

[pcb-release-bodyv6.9-20260922-r5](pcb-release-bodyv6.9-20260922-r5/README.md)
adapts r4 to the **BodyV6.9** case (`mechanical-source/case/BodyV6.9.step`):
a new top-edge notch for the case seat block at (36.65, 3.6), the right-pillar
notch removed (no pillar there any more), and C11/C12/R7 moved off the
relocated B-side post. Main board native KiCad 10.0.4 DRC 0/0; encoder board
identical to r4. Its README has the V6 → V6.9 change table and the open
mechanical points (ENC1 through the back cover; encoder MH2 spigot fit).

## BodyV6 r4 (2026-09-22) — superseded by r5

[pcb-release-bodyv6-20260922-r4](pcb-release-bodyv6-20260922-r4/README.md) adapts
r3 to the **BodyV6** case (`mechanical-source/case/BodyV6.step`): new outline
with pillar notches, MH3 removed and MH4 moved, display receptacle U3 changed
to DIANWEI 8.03A5.031250 (LCSC C55247264), speaker connector J1 moved to the
F face, encoder locating holes Ø2.6 and encoder J1 shifted 0.5 mm. Both r4
boards pass native KiCad 10.0.4 DRC (0 violations, 0 unconnected) and carry
their own Gerber/drill/stencil/position/STEP exports and `manifest.json`.
The main-board lower-left quadrant was re-routed by script and should be
reviewed in KiCad before ordering; the CoHDL sources in r4 were hand-edited
and not re-checked, and `check.sh` / `validate_release.py` still validate r3
only. r3 below is unchanged and remains the validated BodyV5 release.

## Open the routed boards

- [Main PCB, 4 layers](pcb-release-bodyv5-20260917-r3/main/main-45deg.kicad_pcb)
- [Encoder PCB, 2 layers](pcb-release-bodyv5-20260917-r3/encoder/encoder-45deg.kicad_pcb)
- [Complete fabrication and 3D handoff](pcb-release-bodyv5-20260917-r3/openmicrodial-bodyv5-r3.zip)
- [Assembly instructions, BOM/position links and remaining checks](pcb-release-bodyv5-20260917-r3/README.md)
- [Case-aligned populated assembly](pcb-release-bodyv5-20260917-r3/3d/bodyv5-fit-assembly.step)

Keep the release's sibling `models/` directory with the board folders. The
KiCad boards use relative model paths. Main J2 and encoder J1 are four-pin
Hirose FH34SRJ-4S-0.5SH(99): 1 GND, 2 encoder A, 3 encoder B, 4 GND.
Use the matching four-way cable and both boards from this release together.
All five mounting holes are 2.00 mm NPTH.

## Source and mechanical authority

The checked CoHDL projects and exact locks are in
[source/main](pcb-release-bodyv5-20260917-r3/source/main) and
[source/encoder](pcb-release-bodyv5-20260917-r3/source/encoder).
The routed board files above are the fabrication authority. Compiling CoHDL
produces **unrouted** boards; never copy those over the routed release.

[mechanical-source](mechanical-source/README.md) retains the editable BodyV5
case, the exact case STEP used by r3's fit audit, the selected display/cable
drawings, and original component CAD/evidence. The editable Fusion assembly
contains older PCB reference bodies; use it for case editing, not PCB fabrication.
The r3 populated STEP files are the current board-fit references.

Release files and their manifest are retained byte-for-byte. Some source
comments and verification reports record historical paths to scripts or scratch
files used during development. These are provenance, not build dependencies;
the obsolete placement scripts must not be applied to the final boards.

## Validate or compile

CoHDL **0.7.1** is the compiler used for this source snapshot. Set `COHDL_BIN`
if the default executable is another version. With the pinned dependencies
already installed, run:

```sh
COHDL_BIN=/path/to/cohdl-0.7.1 sh hardware/rev-a/check.sh
```

The check verifies every release hash, ZIP member, model, recorded DRC result
and preserved mechanical input, then checks both CoHDL projects without writing
to the release. To install missing dependency versions, run `cohdl install` on
each source project explicitly. To compile, copy a source project to a separate
working directory first, then run `cohdl build COPY --emit kicad_pcb`.
Dependency downloads are not part of the validation script.

Both supplied boards have zero reported KiCad DRC violations and zero unconnected
items under their supplied project rules. This is not physical qualification:
display/cable folds, USB shell clearance, assembly tolerances and the 7 mm battery
still need the documented prototype checks. The SGM41562SA charger also requires
board-specific firmware support; the existing SGM41562S driver is not sufficient.

Firmware, native-target work and prototype runtime documentation are separate
from this hardware release.

## License

The design files in this directory (CoHDL sources, KiCad projects, fabrication
outputs, BOMs, case CAD and release notes) are CERN-OHL-P-2.0, see
[`../LICENSE`](../LICENSE) and [`../README.md`](../README.md). `check.sh` and
`validate_release.py` are software and stay MIT like the rest of the
repository. Third-party models and drawings keep their own licenses, see each
release's `models/ATTRIBUTION.md` and `mechanical-source/vendor/`.
