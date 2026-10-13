# OpenMicroDial BodyV6.13 — PCB ECO revision r6

r6 adapts the BodyV6.9 r5 boards to the **BodyV6.13** case (Fusion export
`mechanical-source/case/BodyV6.13.step`). Only the main-board **outline**
changed: footprints, tracks and vias are identical to r5, and the encoder
board is still the r4 board. The main board passes native KiCad 10.0.4 DRC
with zero violations and zero unconnected items, and all fabrication and
assembly outputs below were exported from that file. The review notes for the
r4 re-route (lower-left quadrant) and the r5 part moves (C11/C12/R7) still
apply.

**Updated in place later on 2026-09-23** (still r6, same case): a right-side
screw slot and a cleaner top-left corner were added, see
[r6 update](#r6-update-right-screw-slot-and-top-left-corner). All main-board
fabrication files and 3D models were re-exported after that change. The
encoder board's boss hole MH2 was then enlarged from Ø2.6 to Ø2.8 (see
[encoder holes](#encoder-holes-one-around-the-boss-one-for-the-screw)); it is no longer
identical to r4. Last, the out-of-stock SPM1423HM4H-B microphone was replaced
by the **ST MP34DT05TR-A on the front (B) side**, inside the case's sound post
(see [microphone](#r6-update-microphone-mk1--st-mp34dt05tr-a-on-the-front-side));
the main board's copper around it was re-routed and every main-board output
was exported again. Then the out-of-stock SGM41562SA charger was replaced by
the **TI BQ24073RGTR** (see [charger](#r6-update-charger-u6--ti-bq24073rgtr)),
again with a local re-route and a full re-export. **The battery NTC must now
be 10k and the firmware's SGM41562 driver must go.** On 2026-09-24 the
out-of-stock fixed 3.3 V regulator U7 (RT6150B-33GQW) was replaced by the
adjustable RT6150BGQW (the part U8 already uses) with a feedback divider on the
B side (see [3.3 V regulator](#r6-update-33-v-regulator-u7--rt6150bgqw-with-a-feedback-divider)),
and the main board was re-exported once more. Last, pin-1 dots were added to
the silkscreen next to the ten ICs that had no pin-1 mark (see
[pin-1 dots](#r6-update-pin-1-dots-on-the-silkscreen)); only the two
silkscreen layers changed, and the Gerbers and Gerber zips were rebuilt.
An alternative encoder board with ENC1 on the wheel side, inside a cavity in
the wheel, is in [`encoder-wheel/`](encoder-wheel/README.md) (see
[encoder-wheel variant](#r6-variant-encoder-board-with-enc1-inside-the-wheel)).
It needs a modified wheel.

## Files to use

| Board | Editable KiCad PCB | Fabrication upload | Stencil | Assembly BOM / positions | Populated STEP |
|---|---|---|---|---|---|
| Main, 4 layers (MK1 = MP34DT05TR-A, U6 = BQ24073RGTR, U7 = RT6150BGQW) | [PCB](main/main-45deg.kicad_pcb) | [Gerber + drill ZIP](main/main-gerber-drill.zip) | [paste Gerbers](main/stencil/) | [BOM](main/assembly/bom.csv) / [positions](main/assembly/positions.csv) | [BodyV6.13 aligned](3d/main-populated-bodyv6.13.step) / [native](3d/main-populated-native.step) |
| Encoder, 2 layers (r4 + MH2 Ø2.8) | [PCB](encoder/encoder-45deg.kicad_pcb) | [Gerber + drill ZIP](encoder/encoder-gerber-drill.zip) | [paste Gerbers](encoder/stencil/) | [BOM](encoder/assembly/bom.csv) / [positions](encoder/assembly/positions.csv) | [BodyV6.13 aligned](3d/encoder-populated-bodyv6.13.step) / [native](3d/encoder-populated-native.step) |
| Encoder, wheel-mounted variant (alternative to the row above; ENC1 on the wheel side, needs the [modified wheel](encoder-wheel/mechanical/wheel-with-encoder-cavity-bodyv6.13.step)) | [PCB](encoder-wheel/encoder-wheel-45deg.kicad_pcb) | [Gerber + drill ZIP](encoder-wheel/encoder-wheel-gerber-drill.zip) | [paste Gerbers](encoder-wheel/stencil/) | [BOM](encoder-wheel/assembly/bom.csv) / [positions](encoder-wheel/assembly/positions.csv) | [BodyV6.13 aligned](3d/encoder-wheel-populated-bodyv6.13.step) / [native](3d/encoder-wheel-populated-native.step) |

**Ordering from JLCPCB:** use [`fab-jlcpcb/`](fab-jlcpcb/ORDER-NOTES.md).
It has a Gerber + drill zip per board with paste layers for the stencil, a
BOM with LCSC part numbers and a CPL in JLC's bottom-side rotation
convention. Read `ORDER-NOTES.md` first: Y1 has no LCSC part, and
several lines were out of stock at LCSC on 2026-09-23. The package was
regenerated after the microphone, charger and 3.3 V regulator changes.

Both boards are 1.0 mm nominal. The aligned STEP files are in the
`BodyV6.13.step` frame (X = u − 28, Z = 47.5 − v, board normal along Y,
F/battery side toward −Y), main B face on the pillar seats (Y 8.275–9.275),
encoder B face on the case post shoulder (Y 2.5–3.5).
[bodyv6.13-fit-assembly.step](3d/bodyv6.13-fit-assembly.step) is the V6.13
case (eight solids) plus both populated boards;
`verification/bodyv6.13-fit-model-report.json` is the interference and
clearance report on it.

## What changed in the case (V6.9 → V6.13) and what the board does about it

| Case change (u, v in board mm) | Board response |
|---|---|
| New wall boss on the right side, u 54.0–55.0 × v 49.625–54.375, from the back up through the main-board plane (Y 1–9.28). | The side-button recess on the right edge (edge at u 53.49) now continues straight down to v 54.675 with an r0.5 inner fillet: 0.51 mm to the boss face, 0.30 mm below it. This also removes the old step out to u 54.69 at v 49, so no thin sliver of board is left between recess and notch. No copper was within reach. |
| Left wall rib extended inboard from u 2.0 to u 3.0 (v 47.662–48.662). | Relief extended from u 2.51 to u 3.3 over v 47.342–49.0, r0.5 inner fillets: 0.23 mm minimum. |
| TL and BR pillars joined to the case corner by a diagonal web. | No change needed: the web runs down the existing corner slot with 0.125 mm per side, the same as the pillar clearance. That is tight for a routed edge (fabs typically hold outlines to about ±0.1–0.2 mm) — widen the web gap in CAD or tell me to open the slots if you want more. |
| B-side post at (48, 70.5) hollowed into a ring (outer r4.0 unchanged, bore r3.0). | Nothing to do; C11 / R7 keep 0.43 / 0.54 mm. |
| B-side seat bosses at MH2 (3.8, 51) and MH4 (34.229, 91.2) removed. | Nothing to do; the Ø2.0 holes stay (unused). |
| **Back cover not in the export** (eight solids instead of nine). | Could not be checked: the ENC1-through-cover conflict (15.9 mm³ in V6/V6.9) and the encoder MH2 spigot / cover-boss fit are unverified for V6.13. Export the cover and I will re-run the check. |
| Wheel, side and front buttons, seat block, encoder MH2 spigot | Unchanged. |

Also corrected in r6: the r5 top-edge notch around the seat block was cut on
a wrong assumption about the block's shape (r3.07 about (36.65, 3.6)); the
block is actually r2.875 about the encoder MH1 axis (36.4587, 3.8), so r5's
clearance was 0.159 mm at its tightest, not the 0.24 mm its README claims.
r6 re-cuts that arc concentric with the block: 0.245 mm all round.

## r6 update: right screw slot and top-left corner

| Change (u, v in board mm) | Details |
|---|---|
| **Right screw: open U-slot** into the V6.13 B-side boss at (52.2, 52.0). | 2.2 mm wide, open from the right edge (u 53.49) to a rounded end centred on the boss axis (edges at v 50.9 / 53.1, r1.1 end). The boss has a Ø1.75 blind pilot from its face (Y 9.28) to Y ≈ 12.25, so the screw goes in from the F/battery side through the slot. An F.Cu + B.Cu keepout of r2.5 around the axis keeps tracks, vias, pads, pours and footprints out from under the head and the boss face (nothing was there). Nearest copper is In2.Cu PTT, 1.24 mm from the slot edge; the In1 GND pour is pulled back 0.3 mm. |
| **Top-left corner: sharp tips removed.** | The diagonal pillar/web slot used to run out to the curved corner edge and leave two acute tips at (4.77, 1.94) and (1.94, 4.77). From each end of the pillar notch arc the edge now runs straight to the outer curve (u = 6.214 on the top side, v = 6.214 on the left side), with an r0.5 fillet where it meets the curve. 2.95 mm² of board removed, no copper nearby. Clearance to the web is unchanged at 0.125 mm (at the notch arc ends). |

Screw choice for the new slot — please check against the CAD:

- **Length:** from the board F face to the bottom of the pilot is 3.975 mm, so
  **M2×3** (2.0 mm thread engagement) fits. M2×4 would bottom out.
- **Head:** the F-side right wall rib (face at u 54.0, v 49.625–54.375, down
  to the board F face) is 1.8 mm from the screw axis. A Ø4.0 pan head
  (ISO 7045) overlaps it by 0.2 mm, a Ø3.8 cheese head by 0.1 mm, and a Ø3.6 head just
  touches it; Ø3.5 leaves 0.05 mm. Either use a small head (≤ Ø3.4) or
  relieve the rib locally in Fusion (for example a Ø4.6 pocket in the rib face, ~1.6 mm
  deep toward −Y).
- **Slot width:** 2.2 mm gives 0.1 mm per side around an M2 shank. The board
  can float ±0.125 mm on the pillars, so the slot may rub the screw. Tell me
  if you want it at 2.4 mm (still 1.14 mm to the nearest copper).
- The battery and fasteners are not in the fit model. A pan head sticks
  out ~1.3 mm from the board F face toward the battery.

## r6 update: microphone MK1 → ST MP34DT05TR-A on the front side

The SPM1423HM4H-B was out of stock at JLCPCB, so MK1 is now the ST
**MP34DT05TR-A** (LCSC C2688664, JLC Extended part). It is also a top-port
PDM microphone, in a 3 × 4 × 1 mm HCLGA package
([datasheet DS12239 rev 4](../mechanical-source/datasheets/mp34dt05-a.pdf)).
It now sits on the **B (front) side at (48, 70.5)**, centred in the case's
hollow sound post, with its lid and sound port facing the front-wall sound
hole. The old part was on the F side at (48, 72.0).

| Item | Details |
|---|---|
| Footprint | New `FP_ST_MP34DT05_HCLGA_3x4`, drawn from the datasheet land pattern (Fig. 12) and mirrored for bottom mounting. It has four 0.40 × 0.95 signal pads (1 Vdd, 2 LR, 3 CLK, 4 DOUT) inside a ground ring split into four pad-5 bars. The bars are declared as one internally connected pad group (KiCad "jumper" pads) because the part's ground ring is one piece of metal. Courtyard 3.5 × 4.5. 3D model `models/MP34DT05TR-A.step`: a 3.1 × 4.1 × 1.1 envelope (maximum size), not vendor CAD. |
| Sound port | The Ø0.25 inlet is 0.78 mm off the package centre toward the LR/CLK end, at (48, 69.72). A straight line out of it passes through the Ø2.0 front hole, 0.22 mm inside the hole edge. |
| Fit in the sound post | The post bore is r3.0 from the board face (Y 9.28) to the front wall (Y 14). The package corners are at r2.50 nominal (r2.57 at maximum size), so the gap to the bore wall is 0.50 mm (0.43 mm at maximum size). The lid is about 3.7 mm from the front wall's inner face. The 3D check finds no interference. |
| Connections | Pin 1 Vdd → `audio::MIC_SUPPLY` (U2 load switch output, C7 100 nF). Pin 3 CLK → `audio::MIC_CLOCK` (through R6 22 Ω from U5). Pin 4 DOUT → `MIC_PDM_DATA` (U5 pin 10, R7 100 k pull-down). Pin 2 LR → GND, which selects the left channel: data is valid while CLK is low, the same timing as the old part's SELECT = GND. |
| Ground | Two GND vias on the left ring bar go to the In1 ground plane. The LR pad is tied to the left bar, and the top bar is linked to it. The right and bottom bars connect through the part's own ring. |
| No vias under the part | New rule area "MK1 body: no vias" (all copper layers, 3.1 × 4.1 over the package). |
| Re-route | Copper changed only inside u 43.5–53.3, v 66.1–75.9. The old mic's escapes and its four GND vias are gone. MIC_SUPPLY, MIC_CLOCK, MIC_PDM_DATA and MIC_PDM_CLK were re-routed, and so were AMP_ENABLE, ENTER, MIC_POWER_ENABLE and the thin CELL sense trace (not the 1.0 mm battery trunk). In total 80 track segments and 14 vias were removed and 67 segments and 10 vias added; the board now has 363 vias. MIC_SUPPLY is 0.15 mm wide (was 0.2), AMP_ENABLE and the CELL sense trace partly 0.104 mm, the rest 0.12 mm. All segments are at 0/45/90°. |

Please check these before ordering:

- **PDM clock: 1.2–3.25 MHz.** The MP34DT05 is only specified for clocks in
  that range. Check the SF32LB52 PDM clock setting in firmware (1.536, 2.048,
  2.4 or 3.072 MHz are all fine).
- **Level:** sensitivity is −26 dBFS (SPM1423: −22 dBFS), so the same speech
  comes out about 4 dB quieter. Add about 4 dB of digital gain if levels
  matter. SNR is 64 dB.
- **Decoupling:** the datasheet asks for 100 nF + 1 µF close to Vdd. C7
  (100 nF C0G) is on the F side, about 5 mm of track from pin 1. There is no
  room for a capacitor inside the sound post, so this is the same arrangement
  the old microphone had. If you want the 1 µF, it can go next to C7.
- **Sound post acoustics:** the air in the post in front of the mic (about
  Ø6 × 4.7 mm, 130 mm³) together with the Ø2 × 1 mm hole forms a Helmholtz
  resonator at roughly 5 kHz. That gives a peak in the upper voice band. A
  soft gasket (foam or silicone ring, about 1 mm thick) between the mic lid
  and the front wall, sealed around the port, shrinks that volume and pushes
  the peak well above the voice band.
- **Vias inside the post must be sealed.** Five vias are inside the bore
  (2 GND, 2 MIC_PDM_DATA, 1 AMP_ENABLE) and three touch its edge. Open vias
  would leak the post to the back of the board. Order **epoxy filled &
  capped** vias (already recommended in the JLC notes, now needed for the
  mic too), or at least tented.

## r6 update: charger U6 → TI BQ24073RGTR

The SGM41562SAXTYH10G charger was out of stock at JLCPCB (so was the rest of
the SGM41562 family), so U6 is now the TI **BQ24073RGTR** (LCSC C15220, JLC
Extended part, about 16,900 in stock at LCSC on 2026-09-23;
[datasheet](https://www.ti.com/lit/ds/symlink/bq24073.pdf)). It is a
standalone single-cell Li-ion linear charger with power-path control, in a
3 × 3 mm VQFN-16 with an exposed pad. Unlike the SGM41562 it has **no I²C**:
the charge settings are fixed by pin straps and two resistors. It sits at the
old U6 centre, (15, 10) on the F side.

| Item | Details |
|---|---|
| Fixed by the part | Battery regulation 4.2 V; system output (OUT → SYSTEM_SUPPLY) regulated to 4.4 V while USB is present (the SGM41562 gave about 4.55 V); input overvoltage cut-off 6.6 V; battery supplies the system through the internal FET when USB is absent. |
| Pin straps | EN2 = GND and EN1 = SYSTEM_SUPPLY select **USB500** mode (500 mA input limit). CE = GND: charging is always enabled. TD = GND: charge termination on. TMR open: default safety timers (30 min precharge, 5 h fast charge). PGOOD is not connected. |
| Resistors | R37 **2.55k** on ISET sets the fast-charge current to 890 / 2.55k = **349 mA** (precharge and termination are fractions of it). R38 **3.3k** on ILIM only sets the input limit in the resistor-limit mode, which this board does not use; ILIM must still be connected or the chip won't charge. R37/R38 are the old SGM41562 NTC-network resistors (91k/442k) re-used with new values and positions. |
| Capacitors | C41 4.7 µF on IN (USB_VBUS), C48 4.7 µF on OUT, C40 4.7 µF on BAT, each moved next to the new pins. C51 (the old chip's VDD capacitor) and R33 (its nINT pull-up) are removed. |
| Status | CHG (open drain, low while charging) goes to **TP1**; there is no pull-up on the board. |
| Temperature | TS goes to J4 pin 1 as before (NTC to GND on J4 pin 2). See the NTC note below. |
| Footprint | `FP_TI_RGT_VQFN16_3x3_EP1p68` from the KiCad RGT0016 land pattern: 16 pads 0.825 × 0.25 at 0.5 mm pitch, 1.68 mm exposed pad with four paste windows, five 0.2 mm GND vias in the exposed pad. 3D model `models/BQ24073RGTR.step`: a 3.1 × 3.1 × 1.0 envelope (maximum size), not vendor CAD. |
| Re-route | Copper changed only around the charger (u 10.0–21.4, v 4.9–14.1), plus the charger branches of I2C_SCL and I2C_SDA, which were removed back to the rest of the bus (U5, display connector U3 and pull-ups R28/R29, all unchanged). 83 segments and 13 vias removed, 40 segments and 12 vias added; the board now has 362 vias. Battery and system paths use 0.6 mm tracks, 0.6/0.3 vias and 1.0 mm links into the existing CELL (B.Cu) and SYSTEM_SUPPLY (In2) runs. All segments are at 0/45/90°. |

Please check these before ordering:

- **Battery NTC must be 10k.** The BQ24073's TS input expects a 10 kΩ NTC
  (103AT type, B ≈ 3380–3435, for example Murata NXFT15XH103FA2B050). The
  old design used a 100k NTC (NXFT15WF104FA2B050); with that the charger would
  treat the cell as cold and not charge. With a 10k NTC it charges between
  about 0 °C and 50 °C. With nothing on J4 the TS pin floats and charging
  stays off, so for bench tests without an NTC put a 10 kΩ resistor across J4.
- **Firmware.** The `hw-a` builds skip the SGM41562 I²C driver
  (`src/battery/charger.rs`, address 0x03); nothing answers there any more.
  Charging is automatic and can't be disabled from firmware (CE is tied low).
  Charge state is only visible on TP1; route it to a GPIO in a later revision
  if firmware needs it. The battery voltage sense on U5 is unchanged, and the
  battery estimator runs on it alone (`ChargeState::Unknown`).
- **Heat.** It is a linear charger: at the start of fast charge (cell at
  3.0 V) about 0.5 W is dropped in the battery FET plus up to 0.3 W in the
  input FET. The five filled vias spread that into the ground planes, and
  the chip's own thermal regulation (TJ 125 °C) lowers the charge current if
  needed. Check the case temperature near U6 on the first prototype.
- **Via filling.** The exposed-pad vias need the epoxy filled & capped option
  (already recommended in the JLC notes); open vias would pull solder out of
  the thermal pad.

## r6 update: 3.3 V regulator U7 → RT6150BGQW with a feedback divider

U7 makes **IO_3V3**, the board's 3.3 V rail, from SYSTEM_SUPPLY (the battery,
or 4.4 V from the charger on USB). It is a Richtek RT6150B buck-boost
converter, needed because the battery voltage crosses 3.3 V as it discharges.
IO_3V3 feeds the display logic supply (PANEL_LOGIC, through load switch U10),
the touch controller supply (U3 pin 8) and its I²C pull-ups R28/R29, the
microphone supply (through load switch U2) and the 100k pull-ups
R12–R19/R41. The MCU and the flash run from the MCU's own regulator.

The fixed 3.3 V version, RT6150B-33GQW, was out of stock at JLCPCB, so U7
is now the **adjustable RT6150BGQW** (LCSC C147972, about 1,350 in stock at
LCSC on 2026-09-24), the same part U8 already uses for PANEL_4V. Package,
pinout, placement, inductor and capacitors are unchanged
([datasheet](https://www.mouser.com/datasheet/2/1458/DS6150AB_06-3104893.pdf)).

| Item | Details |
|---|---|
| Change | Pin 10 (FB) was tied straight to the output (fixed version). It now goes to a divider: **R42 1M** from IO_3V3 to FB and **R43 180k** from FB to GND. VOUT = 0.5 V × (1 + 1000/180) = **3.28 V** (VFB 0.495–0.505 V). R43 is inside Richtek's recommended 80k–500k range for the lower resistor. The divider draws 2.8 µA from the always-on rail. |
| Why these values | Both are in large stock at LCSC (1M is the UniOhm part the BOM already uses, 180k is UniOhm C25760, about 165,000 pcs). The datasheet example (487k / 86.6k, 3.31 V) needs a 487k part I couldn't find in stock. |
| Placement | R42 and R43 are 0402s on the **B side under U7**, the only free space near it: R42 at (35.5, 63.4) rot 0, R43 at (36.4, 64.9) rot −90. FB reaches them through a new via next to pin 10 (36.575, 62.975); R42's top end takes IO_3V3 from a new via on the existing In2 IO_3V3 run (35.2, 62.8); R43's ground end goes to U7's thermal-pad via (35.5, 65.0). The In1 ground plane sits between the switching nodes on F and the FB copper on B. |
| Copper | 3 F.Cu segments removed (the old pin 10 strap) and one In2 segment split at the new via; 12 segments and 2 vias added in total. The board now has 364 vias. All segments are at 0/45/90°. |

Things to check on the first board: IO_3V3 should measure about 3.28 V
(3.2–3.35 V with part tolerances). Richtek's feed-forward capacitor formula,
Cff = 20 pF × (487k / R1) − 20 pF, gives no capacitor for an upper resistor
of 487k or more, so none is fitted, as on U8.

## r6 update: pin-1 dots on the silkscreen

Ten ICs on the main board had no pin-1 mark on the silkscreen, so neither
JLC's placement preview, an optical inspection nor someone reworking a board
by hand could check their orientation. Each now has a filled silkscreen dot,
0.3 mm across, next to pin 1, on the same side as the part. The dots are
part of the footprints on the board, so they move with the part.

| Part | Side | Dot at (u, v) | Distance to pin-1 pad centre |
|---|---|---|---|
| U1 NS4150B speaker amplifier, SOP-8 | B | (39.500, 68.475) | 0.84 mm |
| U2 TPS22918 microphone load switch, SOT-23-6 | B | (50.375, 65.800) | 0.83 mm |
| U3 H0216F002AM display + touch FPC connector (31 pins, 0.3 mm pitch) | B | (31.925, 50.325) | 0.72 mm |
| U4 W25Q128JV flash, SOIC-8 | F | (29.887, 73.470) | 1.04 mm |
| U5 SF32LB525UC6 MCU, QFN-68 | F | (51.575, 84.725) | 0.51 mm |
| U7 RT6150BGQW (IO_3V3), WDFN-10 | F | (34.125, 63.050) | 0.47 mm |
| U8 RT6150BGQW (PANEL_4V), WDFN-10 | F | (46.525, 63.050) | 0.51 mm |
| U10 TPS22918 display-logic load switch, SOT-23-6 | B | (36.225, 58.450) | 0.83 mm |
| U20 USBLC6 USB ESD, SOT-23-6 | F | (29.050, 9.725) | 0.87 mm |
| Y1 48 MHz crystal, 2016 4-pad | F | (49.324, 88.606) | 0.74 mm |

How they were placed: each dot is outside the part body where there is room,
at least 0.15 mm from any copper pad on its side, 0.15 mm from other silkscreen,
0.3 mm from the board edge and clear of via copper (the vias are tented). Pin 1
is always the closest pad to the dot. On the U3 connector the pads are so
fine that the mounting pad (1.03 mm) and pin 3 (1.04 mm) are nearly as close,
so there the dot marks the pin-1 end of the row (the low-u end) rather than a
single pad. U6, MK1, J2 and the side switches already had pin-1 marks. Still
without a mark: the 2-pad parts (passives, the AE1 chip antenna, SW3/SW4) and
the connectors J1, J3, J4 and J5.

KiCad 10.0.4 DRC with its silkscreen checks (silk over copper, silk to board
edge, silk overlap) reports 0 errors and 0 warnings. Compared as parsed Gerber
objects, only `F_Silkscreen` (6 dots) and `B_Silkscreen` (4 dots) changed; the
copper, mask, paste and Edge_Cuts layers and both drill files are the same as
before. The files' text differs only by the plot date and the order KiCad
writes objects in. Tracks, vias, footprint positions, the BOM and the CPL are
unchanged. The STEP files contain no silkscreen, so they were not re-exported.

## r6 variant: encoder board with ENC1 inside the wheel

Added on 2026-09-24 next to the r6 encoder board, which is unchanged. Details are in
[encoder-wheel/README.md](encoder-wheel/README.md).

- **Board:** ENC1 (Alps EC05E1220401) moves to the wheel (B) side at the same position.
  The wheel gets a Ø10.6 cavity that houses it. J1 stays on the back (F) side, so the
  board is now assembled on both sides.
- **A/B swap:** terminal A goes to `ENC_B_CONTACT` and terminal B to
  `ENC_A_CONTACT`. That keeps the wheel direction, so firmware is unchanged. The F.Cu
  routing to J1 is the r6 routing; two vias were added.
- **What it saves:** nothing hangs below the encoder board except J1 (bottom Y 1.42).
  On the r6 board, ENC1 reaches Y −0.34. So the back cover needs no pocket for the
  encoder.
- **Shaft direction:** the wheel's hex stub now enters ENC1 from its top face, the
  insertion direction on the Alps drawing (`mechanical-source/datasheets/alps-ec05e.pdf`,
  Drawing No.3).
- **Wheel shaft size (applies to both encoder boards):** the encoder's bore is a
  hexagon, 1.72 across flats (1.73 at the entry face). The V6.13 wheel's hex shaft is
  1.905 across flats, so it doesn't fit either board. Make the wheel's hex about 1.68
  across flats (0/−0.02). That clears the smallest bore (1.70); free play is about
  ±2.5° at the nominal 1.72.
- **Wheel for this board:** [`encoder-wheel/mechanical/wheel-with-encoder-cavity-bodyv6.13.step`](encoder-wheel/mechanical/wheel-with-encoder-cavity-bodyv6.13.step)
  is the V6.13 wheel with the lower shaft and collar removed, a Ø10.6 cavity up to
  Y 6.8 and a 1.68 hex stub. KiCad DRC is 0/0. In the 3D check the encoder is
  ≥ 0.355 mm from the wheel body, and the unmodified wheel would collide with it
  (49 mm³).

## Encoder holes: one around the boss, one for the screw

Checked against BodyV6.13 (encoder board F face Y 2.5, B face Y 3.5):

| Hole | Case feature | Board hole | Result |
|---|---|---|---|
| **MH1** (36.459, 3.8), screw | Seat block face at Y 3.5 (the board's B face sits on it, contact ring r 0.87–2.98) with a Ø1.74 blind pilot from Y 3.5 to Y 9.28 (5.8 mm deep). Nothing on the case within r 2.8 below the board. | Ø2.6, unchanged | Correct: M2 clearance hole (0.3 mm radial around the Ø2.0 shank) over a Ø1.74 pilot. From the board F face to the pilot bottom is 6.78 mm, so **M2×5** (4 mm engagement) or **M2×6** (5 mm engagement, 0.78 mm spare) fits. The head sits on the F/cover side; a Ø4 head stays 0.3 mm inside the board edge, and there is no copper or part under it apart from the GND pour under solder mask. |
| **MH2** (52.2, 19.541), boss | Hollow spigot Ø2.55 (bore Ø1.74) running exactly through the board thickness (Y 2.5–3.5), on a Ø3.89 shoulder that the board's B face sits on. | **Ø2.6 → Ø2.8** | Ø2.6 left only 0.025 mm radial around the spigot, which is a press fit at best once hole tolerance (±0.05–0.1) and a printed or moulded spigot are included. At Ø2.8 it is 0.125 mm radial, the same as the main-board pillars, and the board still rests on a 0.55 mm-wide ring of the shoulder. GND pours pulled back automatically; nearest part (J1 courtyard) is 0.64 mm from the new hole edge. |

The screw head's room toward the back cover can't be checked: V6.13 has no
cover. In V6.9 the cover's inner face under MH1 was at Y 1.0, which leaves
0.2 mm under a 1.3 mm-tall pan head (a 1.6 mm head would touch).

## Verification status

- Native KiCad 10.0.4 DRC (zones refilled) after the pin-1 silkscreen dots:
  main 0 violations, 0 unconnected, 0 footprint errors
  (`main/verification/main-drc-kicad10.rpt`; the first r6 report is
  `main-drc-kicad10-r6-original.rpt`). Encoder after the MH2 change: 0/0
  (`encoder/verification/encoder-drc-kicad10.rpt`; r4's is
  `encoder-drc-kicad10-r4.rpt`). The encoder Gerbers, stencil, drill files,
  zip and STEP files were re-exported; compared with r4 only the MH2 hole, its
  mask opening and the GND pour around it changed.
- Encoder-wheel variant: KiCad 10.0.4 DRC with zones refilled 0/0/0
  (`encoder-wheel/verification/encoder-wheel-drc-kicad10.rpt`). 3D fit with the
  modified wheel in `encoder-wheel/verification/encoder-wheel-fit-report.json` and
  [3d/bodyv6.13-encoder-wheel-fit-assembly.step](3d/bodyv6.13-encoder-wheel-fit-assembly.step):
  no interference. Gerbers compared with `encoder/`: Edge_Cuts and NPTH identical,
  PTH +2 vias, ENC1 pads/mask/paste moved from F to B. The eco audit section is
  `encoder_wheel_variant_2026_09_24`.
- `verification/bodyv6.13-eco-audit.json`: DRC-lite 0 issues, every signal
  net a single connected component, all segments 0/45/90°. Before the
  microphone change the KiCad-saved file was identical to r5 in footprints,
  tracks and vias; the `mk1_mic_swap_2026_09_23` section records the new
  footprint, the re-routed nets and the Gerber comparison (all copper changes
  within u 43.5–53.3, v 66.1–75.9); `u6_charger_swap_2026_09_23` does the
  same for the charger (pin-by-pin nets, parts changed, re-route counts and
  the open points), `u7_regulator_swap_2026_09_24` for the U7 divider, and `pin1_silk_dots_2026_09_24` for the silkscreen dots. Also: no case material inside the
  new outline at the board plane; minimum edge clearances 0.125 (pillars +
  webs), 0.23 (left rib), 0.245 (seat block), 0.30 (right boss), 0.51 (right
  screw slot). The `r6_update_2026_09_23` section records the slot, the
  keepout, the pilot and screw-head measurements and the corner trim.
- `verification/bodyv6.13-fit-model-report.json` (3D, nominal): the only
  intersections are J5 USB-C 0.16 mm³ in the case mouth corners and the wheel
  shaft in ENC1's bore 0.62 mm³ — both as since r3. The bore turned out to be a
  1.72 across-flats hexagon (Alps drawing) and the wheel's hex is 1.905, so
  this is not a usable press fit: the wheel's hex should be about 1.68 across
  flats (see the [encoder-wheel variant](#r6-variant-encoder-board-with-enc1-inside-the-wheel)).
  Re-run on the updated board: identical results. After the microphone change
  the only difference is MK1 itself: 0.43 mm to the sound-post bore (the old
  SPM1423 had 1.01 mm). The charger change alters no result: U6 and its
  re-placed passives are all more than 1.5 mm from the case, and U6 is
  outside the battery reserve. The U7 change (R42/R43 under U7 on the B side)
  alters no result either. Other clearances are unchanged from r5 (R39 0.25, Y1 0.54, C11 0.43, R7 0.54 mm to
  the case; side switches 0.06 mm to the side buttons; encoder board touching
  the wheel hub underside).
- Assembly positions match KiCad's placement export for all 120 fitted parts
  (`main/reference/positions-kicad-all.csv`, re-exported after the U7
  change). Against the microphone version U6, R37, R38, C40, C41 and C48
  changed, C51 and R33 are gone, and R42/R43 are new.
- Gerbers after the microphone change, compared with the ones before it:
  every coordinate change is inside u 42–55, v 62–80, except that In1_Cu's
  Gerber region cut lines move elsewhere (the In1 pour itself changed only
  inside u 42.4–51.1, v 66.1–75.9). Edge_Cuts, F_Silkscreen and the NPTH
  drill file are unchanged; the PTH file lost the old mic's vias and gained
  the new ones. After the charger change: F_Cu, F_Mask, F_Paste and
  F_Silkscreen changed only inside u 10–21.4, v 4.9–12.6, B_Cu only inside
  u 10.4–20.6, v 5.6–12.6; In2_Cu there plus the removed I²C branches (up to
  u 29.9, v 48.8); In1_Cu there plus its region cut lines at the same heights.
  B_Mask, B_Paste, B_Silkscreen, Edge_Cuts and NPTH are unchanged; the PTH
  file lost 13 holes and gained 12. After the U7 change: B_Cu, B_Mask and
  B_Paste changed only inside u 34.9–36.6, v 62.8–65.5, F_Cu only inside
  u 35.2–36.8, v 62.1–63.2 and In2_Cu only at the new IO_3V3 via; In1_Cu
  there plus its region cut lines at the same heights. F_Mask, F_Paste, both
  silkscreens, Edge_Cuts and NPTH are unchanged; the PTH file gained the two
  new vias. After the pin-1 dots: only F_Silkscreen and B_Silkscreen changed
  (10 dots); every other layer and both drill files are unchanged.
- Not done: CoHDL `check`; cable fit model; cover-related checks (cover not
  exported); display module, battery, cable and fasteners (including the
  new right screw) are not modelled.

## Source authority

`main/main-45deg.kicad_pcb` (KiCad 10.0.4-saved) is the routed authority.
`source/main/mechanical/main-outline.dxf` was regenerated from the updated
r6 Edge.Cuts; the rest of `source/` is r5's and still has the SPM1423
microphone, the SGM41562SA charger and the fixed RT6150B-33GQW, and its footprints have
none of the new pin-1 dots. Rebuilding CoHDL emits an unrouted board; do not replace the routed
files with it.

## License

Copyright (c) 2026 Tony Huang.

The project's hardware design sources, fabrication outputs and documentation
are licensed under CERN-OHL-P-2.0; see [LICENSE](LICENSE). Third-party material
retains its original licenses and notices, including the MIT attribution in
`source/main/src/display_parts.cohdl` and the model provenance in
[models/ATTRIBUTION.md](models/ATTRIBUTION.md).
