# OpenMicroDial BodyV6 — PCB ECO revision r4

r4 adapts the verified BodyV5 r3 boards to the **BodyV6** case (Fusion export
`mechanical-source/case/BodyV6.step`) and applies three requested component
changes. It was produced by scripted edits of the r3 KiCad 10 boards, not by a
CoHDL rebuild; the CoHDL sources under `source/` were updated by hand to match
but have **not** been re-checked with `cohdl`. Both boards pass native KiCad
10.0.4 DRC with zero violations and zero unconnected items, and all
fabrication/assembly outputs below were exported from those files. **Review
the re-routed lower-left quadrant of the main board in KiCad before ordering**
(see "What to review").

## Files to use

| Board | Editable KiCad PCB | Fabrication upload | Stencil | Assembly BOM / positions | Populated STEP |
|---|---|---|---|---|---|
| Main, 4 layers | [PCB](main/main-45deg.kicad_pcb) | [Gerber + drill ZIP](main/main-gerber-drill.zip) | [paste Gerbers](main/stencil/) | [BOM](main/assembly/bom.csv) / [positions](main/assembly/positions.csv) | [BodyV6 aligned](3d/main-populated-bodyv6.step) / [native](3d/main-populated-native.step) |
| Encoder, 2 layers | [PCB](encoder/encoder-45deg.kicad_pcb) | [Gerber + drill ZIP](encoder/encoder-gerber-drill.zip) | [paste Gerbers](encoder/stencil/) | [BOM](encoder/assembly/bom.csv) / [positions](encoder/assembly/positions.csv) | [BodyV6 aligned](3d/encoder-populated-bodyv6.step) / [native](3d/encoder-populated-native.step) |

Each board has its matching `.kicad_pro`. Keep the sibling `models/` folder
beside both board folders (relative model paths). `gerber/` holds copper,
mask, legend and profile with a job file; `stencil/` the paste layers with
their own job file; `drill/` plated/unplated Excellon files and SVG maps.
Supply the BOM and positions separately for assembly; do not upload the whole
release as a bare Gerber archive.

For structure validation in Fusion use the **BodyV6-aligned** STEP files: they
are in the `BodyV6.step` coordinate frame (X = u − 28, Z = 47.5 − v, board
normal along Y, F/battery side toward −Y) with 1.0 mm nominal board bodies
rebuilt from Edge.Cuts (holes and notches included) and every fitted component
model named `REF (MPN)`. The main board sits with its B face on the case
pillar seats (Y 8.275–9.275); the encoder board with its B face on the case
post shoulders (Y 2.5–3.5, spigots in the Ø2.6 holes; it can float 0.2 mm
toward the cover bosses). [bodyv6-fit-assembly.step](3d/bodyv6-fit-assembly.step)
contains the nine BodyV6 case solids plus both populated boards, and
`verification/bodyv6-fit-model-report.json` is the interference/clearance
report on that assembly. The `*-native.step` files are KiCad's own export
(board body 0.929 mm dielectric stack, drill/place origin, no copper).

## What changed against r3

Main board (`main/main-45deg.kicad_pcb`, 4 layers):

1. **Outline for BodyV6.** The three V6 case pillars (Ø3.6, Ø1.6 screw bore)
   that pass through the PCB at (4.8, 4.8), (52.2, 52.0) and (51.2, 90.2) mm
   are cleared by Ø4.0 open notches in the edge. MH3 (52.2, 52.0) is removed —
   the pillar replaces it. MH4 moves to (34.229, 91.2). MH2 (3.8, 51.0) is
   unchanged. The left-wall rib at v 47.7–48.7 gets a 1.2 × 1.66 mm relief.
   The r2.8 seats around the pillars and the block at the bottom-right corner
   are B-side component keepouts: R39 (1 MΩ reset pull-down) moved from
   (52.7, 87.0) to (52.8, 86.8) rot 0 and its GND via was re-placed.
2. **Display receptacle U3 = DIANWEI 8.03A5.031250 (LCSC C55247264)**, 0.3 mm
   pitch 31P, dual contact, rear flip, H 0.9 mm, for 0.2 mm FPC. Same position
   (36.6, 48.2) on B and same pin/row topology as the previous OCN part, so pad
   x positions are unchanged; the front (odd) pads are 0.3 × 0.8 (span 9.00),
   the rear (even) pads 0.3 × 0.65 (span 8.40), row gap 2.15, plus two
   0.4 × 0.95 fitting nails tied to GND. A 10.9 × 3.6 × 1.0 mm envelope model is
   in `models/8.03A5.031250.step`. Open check: the datasheet FPC width
   E = 10.33 mm versus the 9.6 mm display flex.
3. **Speaker connector J1 (Molex 53261-0271) moved to the F face** (battery /
   back side) at (33.0, 84.9) rot 90, bottom-left corner, mouth toward +u, so
   the speaker seated from the back in the V6 cutout plugs in directly.
   TP10 and TP3 moved 0.65 mm to (34.15, 83.0) / (34.15, 85.0) to make room
   for the two 0.6/0.3 trunk vias at (32.5, 85.525) and (33.2, 84.275).
   SPEAKER_PLUS/MINUS run as 0.5 mm B.Cu trunks with locked F.Cu tails to the
   pads. This displaced the signal bus in the lower-left quadrant: 26 signal
   nets between x 29.4–41 and y 67–92 were ripped up and re-routed by a
   negotiated-congestion router at 0.1/0.104 mm rules, plus 17 nets at the
   bottom-right corner (BR pillar notch) and CELL/PTT on In2 at the right
   pillar notch. All re-routes are 0/45/90°. The trunks are deliberately not
   the shortest path.
4. Inter-board connector J2 stays Hirose FH34SRJ-4S-0.5SH(99) as in r3.

Encoder board (`encoder/encoder-45deg.kicad_pcb`, 2 layers):

- Locating holes MH1/MH2 enlarged from Ø2.0 to Ø2.6 NPTH for the V6 back-cover
  spigots (Ø2.4).
- J1 (FH34SRJ-4S) moved 0.5 mm in −u to (48.0, 16.8) rot 270: the V6 cover
  boss (Ø3.8) around MH2 was 0.05 mm from the r3 connector envelope; it is now
  0.52 mm. `assembly-notes/cable-concept.json` was re-derived (mouth gap
  15.5 mm); re-run the cable fit model before ordering cable.

## Verification status

- **Native KiCad 10.0.4 DRC** with refilled zones: main 0 violations,
  0 unconnected, 0 footprint errors (`main/verification/main-drc-kicad10.rpt`;
  the first run, before the TP10/TP3 courtyard fix and dangling-end trims, is
  kept as `main-drc-kicad10-run1-before-fixes.rpt`); encoder 0/0/0
  (`encoder/verification/encoder-drc-kicad10.rpt`).
- Independent geometry check (`verification/bodyv6-eco-audit.json`, run on the
  KiCad-saved files): 0 clearance, edge, hole or courtyard issues on both
  boards at the r3 project rules; every signal net is a single connected
  component without relying on zones; all track segments are 0/45/90°. 34
  main nets and 3 encoder nets have changed copper against r3; 6 main
  footprints and 1 encoder footprint moved or were removed.
- BodyV6 fit (`verification/bodyv6-fit-audit.json`): no pad, courtyard, track
  or via inside the pillar, rib or MH4 keepouts; no B-side component in the
  r2.8 seats. Y1's courtyard is 0.32 mm from the BR pillar surface (body
  ≥ 0.6 mm). Under the outer part of the BR seat annulus there is one 0.104 mm
  B.Cu track and two tented vias (mask-covered, vias tented front and back in
  the board setup); move them if a bare seat face is required.
- Assembly positions (`assembly/positions.csv`, r3 convention) were checked
  against KiCad's native placement export (`reference/positions-kicad-all.csv`)
  for every fitted component: 120 main, 2 encoder, no mismatches.
- Populated fit model (`verification/bodyv6-fit-model-report.json`, static
  nominal assembly of the BodyV6 case solids and both populated boards):
  three solid intersections — J5 USB-C in the case mouth corners 0.16 mm³
  (unchanged from r3), the wheel shaft inside ENC1's hollow bore 0.62 mm³
  (the intended press-fit, unchanged from r3), and **ENC1 through the new
  back cover, 15.9 mm³**: the encoder body reaches 2.86 mm below the encoder
  board's F face, i.e. through the full 1.0 mm cover plate over u 44.6–50.4 /
  v 6.2–12.0 and 0.35 mm beyond the outer cover face. BodyV5's fit model had
  no back cover, so this is the first time it could be seen; the cover needs
  an opening or pocket at the encoder, or the encoder/board level changes.
  Tight clearances: wheel hub underside coincident with the encoder board
  B face (0 mm); C11 and R7 0.04 / 0.03 mm from the B-side case post at
  (45.4–47.5, 68–69.5) (inherited from r3, same post in BodyV5); R39 0.33 mm
  and Y1 0.61 mm from the new bottom-right pillar block; main board edge
  0.32 mm from the wheel rim and 0.24–0.30 mm from the side-button bodies;
  side switches 0.06 mm to the side buttons and SW3/SW4 0.89 mm to the front
  buttons (as in BodyV5).
- **Not done:** CoHDL `check` of the edited sources; the inter-board cable
  fit model; a physical check of the DIANWEI receptacle against the 9.6 mm
  display flex; display module, battery, cable and fasteners are not in the
  fit model. r3's exported verification reports do not apply to r4 and were
  not copied.

## What to review in KiCad before fabrication

1. Lower-left quadrant of the main board, x 29–41 / y 62–92, all copper
   layers: the speaker trunks (SPEAKER_PLUS west lane at x 32.5, SPEAKER_MINUS
   east lane at x 33.2 rising to C10 on In2) and the 26 re-routed signal nets
   around the TP cluster, U4 and the bus highway at x 29–31.7. These are
   router output, DRC-clean but not hand-tidied everywhere.
2. Bottom-right corner (x 44–55 / y 84–93): R39 and the 17 re-routed nets
   around the BR pillar notch and Y1.
3. Right edge at the (52.2, 52.0) pillar notch: CELL / PTT on In2.
4. The three Ø4.0 edge notches, MH4 at (34.229, 91.2) and the left rib
   relief against the case.
5. U3: pad geometry versus the DIANWEI drawing, and the flex width question.
6. Encoder J1 position (48.0, 16.8) versus the cover boss.
7. In Fusion: the ENC1 / back-cover conflict and the wheel-hub contact on the
   encoder board (see the fit-model report).

## Fabrication and assembly settings

Unchanged from r3: JLCPCB 4/4 mil process, through vias ≥ 0.45 mm diameter
with ≥ 0.20 mm drill (speaker trunk vias 0.60/0.30), 1.0 mm nominal thickness,
C52/C53 DNP, TP1–TP10, J4 bare wire pads and mounting holes excluded from
assembly. Position files use the r3 convention (native origin, Y inverted,
rotation 0…359°). The BOM line for U3 is `8.03A5.031250, DIANWEI,
FP_DIANWEI_8_03A5_031250`; J1/J3 remain two 53261-0271. `models/` is the r3
model set with the OCN envelope replaced by the DIANWEI envelope (the OCN file
is kept as `_superseded-OK-F302-31115.step`).

## Source authority

`main/` and `encoder/` are the routed authority. `source/main/src/display_parts.cohdl`,
`main.cohdl`, `audio.cohdl`, `source/encoder/src/main.cohdl` and the mechanical
DXFs (`main-outline.dxf` regenerated from the r4 Edge.Cuts; `main-mechanical.dxf`
and `encoder-mechanical.dxf` hole updates) were edited by hand to match. The
CoHDL footprint for the DIANWEI part declares nail pads MP1/MP2 that the
H0216F002AM device does not define; bind them to GND (as in the board) when
the source is next compiled. Rebuilding CoHDL emits an unrouted board; do not
replace the routed files with it. r3 (`pcb-release-bodyv5-20260917-r3`) is
untouched and remains the validated BodyV5 release.

## License

Copyright (c) 2026 Tony Huang.

The project's hardware design sources, fabrication outputs and documentation
are licensed under CERN-OHL-P-2.0; see [LICENSE](LICENSE). Third-party material
retains its original licenses and notices, including the MIT attribution in
`source/main/src/display_parts.cohdl` and the model provenance in
[models/ATTRIBUTION.md](models/ATTRIBUTION.md).
