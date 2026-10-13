# OpenMicroDial BodyV6.9 — PCB ECO revision r5

r5 adapts the BodyV6 r4 boards to the **BodyV6.9** case (Fusion export
`mechanical-source/case/BodyV6.9.step`). Only the main board changed; the
encoder board is byte-identical to r4 and its r4 exports are carried over.
The edits were scripted on the KiCad 10 board file (no CoHDL rebuild; the
CoHDL placements were updated by hand and not re-checked with `cohdl`). The
main board passes native KiCad 10.0.4 DRC with zero violations and zero
unconnected items, and all fabrication/assembly outputs below were exported
from that file. The review notes for the r4 re-route (lower-left quadrant)
still apply — see `pcb-release-bodyv6-20260922-r4/README.md`.

## Files to use

| Board | Editable KiCad PCB | Fabrication upload | Stencil | Assembly BOM / positions | Populated STEP |
|---|---|---|---|---|---|
| Main, 4 layers | [PCB](main/main-45deg.kicad_pcb) | [Gerber + drill ZIP](main/main-gerber-drill.zip) | [paste Gerbers](main/stencil/) | [BOM](main/assembly/bom.csv) / [positions](main/assembly/positions.csv) | [BodyV6.9 aligned](3d/main-populated-bodyv6.9.step) / [native](3d/main-populated-native.step) |
| Encoder, 2 layers (= r4) | [PCB](encoder/encoder-45deg.kicad_pcb) | [Gerber + drill ZIP](encoder/encoder-gerber-drill.zip) | [paste Gerbers](encoder/stencil/) | [BOM](encoder/assembly/bom.csv) / [positions](encoder/assembly/positions.csv) | [BodyV6.9 aligned](3d/encoder-populated-bodyv6.9.step) / [native](3d/encoder-populated-native.step) |

Keep the sibling `models/` folder beside both board folders. The aligned STEP
files are in the `BodyV6.9.step` coordinate frame (X = u − 28, Z = 47.5 − v,
board normal along Y, F/battery side toward −Y) with 1.0 mm nominal board
bodies and every fitted component named `REF (MPN)`; main B face on the case
pillar seats (Y 8.275–9.275), encoder B face on the case post shoulder
(Y 2.5–3.5). [bodyv6.9-fit-assembly.step](3d/bodyv6.9-fit-assembly.step) is the
BodyV6.9 case (nine solids) plus both populated boards;
`verification/bodyv6.9-fit-model-report.json` is the interference/clearance
report on it.

## What changed in the case (V6 → V6.9) and what the board does about it

| Case change (u, v in board mm) | Board response |
|---|---|
| New seat block at (36.65, 3.6), r3.07, from Y 3.5 (encoder B face) up to the front. It crosses the main-board plane at u 33.58–35.74. | New notch in the top edge: edge at u = 33.339 from v 1.311 to 3.6, then an r3.311 arc (centre (36.65, 3.6)) to (35.894, 6.824) on the existing wheel-cutout diagonal — 0.24 mm clearance. No copper was in the removed strip (J5's shell tabs stay 0.56 mm from the edge). |
| The pillar at (52.2, 52.0) is gone; a B-side ring seat (r2.0–2.4) sits on the board instead, and the cover hole there is filled. | The Ø4.0 notch is removed and the straight r3 edge (u = 54.69, v 48.99–86.50) restored so the seat has full board under it. The r4 CELL/PTT re-route stays as it was. |
| B-side post moved from (48, 72) to (48, 70.5), r4.0. | C11 (1 µF) → (44.6, 66.43) rot 180 (pad edge 0.21 mm, body 0.43 mm from the post); C12 (10 µF) → (42.8, 61.4) rot 0, pads on the existing SYSTEM_SUPPLY / GND vias; R7 (100 k) → (43.4, 68.6) rot 90 with MIC_PDM_DATA re-routed on B.Cu. The SPEAKER_MINUS run L2→C10 was straightened at u 42.75. Two orphaned GND vias removed. |
| TL/BR pillars Ø3.60 → Ø3.75 (screw bore Ø1.6 → Ø1.75); B-side seats r2.8 → r2.88; MH2/MH4 seat bores filled (plain seats, no screws). | Left unchanged: the Ø4.0 notches now give 0.125 mm radial clearance to the pillars (0.2 before); R39 pad edge 0.26 mm and Y1 body 0.54 mm from the BR block. MH2/MH4 Ø2.0 NPTH holes kept (harmless). Enlarge the notches to Ø4.25 if you want the old clearance back — Y1 would then have to move. |
| New B-side ribs: u 4.25–4.75 along v 11.6–48.7 (with tabs at both ends) and u 51.24–51.74 along v 22.4–49.7. | No B-side components there; tracks under them are mask-covered. |
| Encoder: MH1 spigot, bore and cover boss removed (the seat block presses on the encoder B face around MH1); MH2 spigot Ø2.4 → Ø2.55, cover boss Ø3.8 → Ø3.95. | Encoder board unchanged. MH1 is now an unused Ø2.6 hole. **MH2: Ø2.55 spigot in a Ø2.6 hole is 0.025 mm radial** — fine only if the spigot is meant as a press/locating fit; say so or I enlarge the hole to Ø2.8. J1's lateral clearance to the boss is ~0.45 mm. |
| Back cover: no pocket at the encoder. | **ENC1 still passes through the cover (15.9 mm³, u 44.6–50.4 / v 6.2–12.0, 0.35 mm proud of the outer face)** — unchanged from r4's finding. |

## Verification status

- Native KiCad 10.0.4 DRC (zones refilled): main 0 violations, 0 unconnected,
  0 footprint errors (`main/verification/main-drc-kicad10.rpt`; the first run
  flagged one orphaned GND via, fixed — `main-drc-kicad10-run1.rpt`). Encoder:
  r4's clean report applies (`encoder/verification/encoder-drc-kicad10.rpt`).
- `verification/bodyv6.9-eco-audit.json`: independent DRC-lite 0 issues, every
  signal net a single connected component, all segments 0/45/90°; 3
  footprints moved and 4 nets changed copper against r4.
- `verification/bodyv6.9-fit-audit.json`: no case material inside the new
  outline at the board plane; no pad inside a B- or F-side case contact face;
  nearest pads to a contact face: C11 0.21 mm, R39 0.26 mm.
- `verification/bodyv6.9-fit-model-report.json` (3D, nominal): intersections
  are only J5 USB-C 0.16 mm³ (as r3), wheel shaft in ENC1 0.62 mm³ (press-fit,
  as r3) and ENC1 / back cover 15.9 mm³ (as r4). Clearances: C11 0.43,
  R7 0.54, R39 0.25, Y1 0.54 mm to the case; main board edge 0.32 mm to the
  wheel rim, 0.24–0.30 mm to the side-button bodies; side switches 0.06 mm
  to the side buttons and 0.69 mm to the case; encoder board touches the
  wheel hub underside (0 mm) and sits 0.2 mm above the cover bosses.
- Assembly positions (`main/assembly/positions.csv`) match KiCad's placement
  export (`main/reference/positions-kicad-all.csv`) for the three moved parts;
  the rest is unchanged from r4 (verified there).
- Not done: CoHDL `check`; cable fit model; display module, battery, cable and
  fasteners are not in the fit model.

## Source authority

`main/main-45deg.kicad_pcb` (KiCad 10.0.4-saved) is the routed authority.
`source/main/src/main.cohdl` carries the three new placements
(`audio.c_supply`, `audio.c_supply_bulk` rotate 180, `audio.r_mic_data_pd`);
`source/main/mechanical/main-outline.dxf` was regenerated from the r5
Edge.Cuts. Everything else under `source/` is r4's. Rebuilding CoHDL emits an
unrouted board; do not replace the routed files with it.

## License

Copyright (c) 2026 Tony Huang.

The project's hardware design sources, fabrication outputs and documentation
are licensed under CERN-OHL-P-2.0; see [LICENSE](LICENSE). Third-party material
retains its original licenses and notices, including the MIT attribution in
`source/main/src/display_parts.cohdl` and the model provenance in
[models/ATTRIBUTION.md](models/ATTRIBUTION.md).
