# OpenMicroDial BodyV5 — four-pin inter-board PCB revision r3

This matched handoff replaces r2's six-pin inter-board sockets with the exact
four-pin 0.5 mm-pitch variant and updates only their local routing. It retains
r2's display-connector placement, front-button corrections, case-clearance
component moves and TP1–TP10 references. The case, wheel shaft, board outlines,
all other footprint placements and protected USB/RF/crystal routes are unchanged.

## Files to use

| Board | Editable KiCad PCB | Fabrication upload | Assembly BOM / positions | Populated STEP |
|---|---|---|---|---|
| Main, 4 layers | [PCB](main/main-45deg.kicad_pcb) | [Gerber + drill ZIP](main/main-gerber-drill.zip) | [BOM](main/assembly/bom.csv) / [positions](main/assembly/positions.csv) | [Native coordinates](3d/main-populated-native.step) |
| Encoder, 2 layers | [PCB](encoder/encoder-45deg.kicad_pcb) | [Gerber + drill ZIP](encoder/encoder-gerber-drill.zip) | [BOM](encoder/assembly/bom.csv) / [positions](encoder/assembly/positions.csv) | [Native coordinates](3d/encoder-populated-native.step) |

Each board has its matching `.kicad_pro`. Keep the sibling `models/` folder
beside both board folders: the PCB model paths are portable and relative.
Paste Gerbers are in each board's `stencil/`; plated/unplated drills and SVG
drill maps are in `drill/`. Supply the matching BOM and positions separately
for assembly. Do not upload the complete handoff ZIP as a bare Gerber archive.

For direct placement into the existing BodyV5 coordinate system, use
[main aligned STEP](3d/main-populated-bodyv5.step) and
[encoder aligned STEP](3d/encoder-populated-bodyv5.step). The
[case-fit assembly](3d/bodyv5-fit-assembly.step) contains the unchanged case/caps,
both populated boards, and clearly named battery/cable reference envelopes.
No physical display assembly, separate back cover, fasteners or USB plug is
invented in that model.

## Assembly changes

- Display U3 is on B, native rotation 0°, at (36.6, 48.2) mm. Its mouth faces
  increasing board Y. A component-free lane is retained for the supplied
  38 × 9.6 mm display flex. The actual folded cable must clear the pressed
  macro-button caps, not only their resting position.
- Main J2 and encoder J1 are **Hirose FH34SRJ-4S-0.5SH(99)**, dual-contact,
  0.5 mm pitch, 0.3 mm end thickness. Both are on F at rotation 270°;
  their mouths face decreasing board X. See the
  [inter-board cable installation diagram](assembly-notes/interboard-cable-path.png).
  The untwisted ribbon returns beneath the encoder PCB around its free left
  edge. Physical pin n connects to pin n: 1 = GND, 2 = encoder A,
  3 = encoder B, 4 = GND. The nominal cable width is 2.50 ±0.05 mm;
  the static fit model uses a 2.60 mm width guard. The approximately
  31.625 mm seated tip-to-tip path is
  an installation concept, not a released cable fabrication drawing.
  Use this revision's two four-pin boards together with a matching four-way
  cable; do not mix the old six-pin harness/board set with this revision.
- SW3/SW4 face the large front buttons on B. The four side switches stay on F.
  The encoder body is on F, away from the wheel; the intended shaft press-fit
  is retained. No wheel/shaft machining change was made.
- Microphone/power passives were moved clear of case bosses. U2's input
  bypass C5 moves with it, rather than remaining at the previous location.
- Ten fabricated test points now use TP1–TP10. The original `U` allocation
  came from the generic test-point library definition; board-local explicit
  references correct this without renumbering other components. U10 remains
  a real IC. Test pads are not purchased assembly components.

## Fabrication and assembly settings

- Main: 4 copper layers. Encoder: 2 copper layers. JLCPCB 4/4 mil process,
  through vias at least 0.45 mm diameter with at least 0.20 mm drill.
- Nominal board thickness is 1.0 mm. The explicit main stack represented in
  the fit STEP is **1.02968 mm finished**; encoder is **1.00000 mm finished**.
  Confirm the supplied fabrication stack against the enclosure fit. A changed
  finished thickness changes the assembly model's seating faces.
- Main assembly includes 120 fitted components; encoder includes 2. C52/C53
  are intentionally DNP. TP1–TP10, J4 bare wire pads and mounting holes are
  excluded from assembly BOM/positions but remain in fabrication data.
- Position files use millimetres, the native absolute origin, X unchanged,
  and Y inverted from KiCad's screen-down coordinates. Bottom X is not
  mirrored. Rotations are normalized to 0…359° from native footprint angles.
  Confirm catalog-specific placement rotation with the assembler's preview.
- BOMs contain exact MPNs, not guessed JLC stock codes. Match any substituted
  part's footprint, height and pinout before accepting an assembly substitution.
- The battery space modeled here is **41 × 20 × 7 mm**, on the USB/F side,
  plus 0.3 mm insulation. It is not a 9 mm battery-height approval. J3 is
  Molex 53261-0271: pin 1 battery positive, pin 2 ground. Verify the actual
  mating housing and wire polarity before connecting a cell.
- The QFN charger is SGM41562SA. Existing firmware still accepts only the
  older SGM41562S identity (`0x09`) and rejects the SA identity (`0x00`), so
  it will not configure charging on this board until board-specific SA
  support is implemented. Firmware is unchanged. See the
  [charger migration note](source/main/docs/charger-qfn-migration.md).

## Verification and remaining physical checks

The release manifest identifies the exact board hashes used for fabrication,
assembly and populated models. Each final board is checked with official
KiCad 10.0.4, with zero reported DRC violations and zero unconnected items
under its supplied project rules. Independent checks cover every net and
native filled-copper island, exact 0/45/90° track directions, source-to-board
placement/pin/net parity, Gerber outline continuity, drill/slot geometry and
assembly population. Detailed reports are under `verification/`.

These checks do **not** certify the physical build. Before ordering a full run:

1. Test the actual display flex at U3 through full button travel. The supplied
   drawings do not establish both installed connector-mouth heights.
2. Test USB-C shell clearance at the case opening. Its drawing leaves the
   rounded mouth corners undimensioned. The conservative maximum rectangular
   model intersects there by about 0.161 mm³; even nominal rectangular bounds
   intersect slightly. An actual-shell fit or supplier CAD is needed to resolve it.
3. Confirm inter-board cable length, end stiffeners and bend rating with the
   supplier. Do not crease the flex or bend inside its stiffened insertion ends.
4. Check the encoder shaft press-fit, the very tight locating-boss/aperture
   tolerances, printed cap travel/retention, solder heights and the separate cover.

The 3D models use registered vendor/community CAD where available and labelled
dimensioned envelopes for other parts. Refer to `models/manifest.json` for
which is which. They support the requested case-fit trial; they are not a
claim of exact vendor geometry for every component or SI/PI/EMI qualification.

## Source authority

`main/` and `encoder/` contain the routed PCB authority. `source/` contains
the matching CoHDL source and exact dependency/designator locks. Rebuilding
CoHDL emits a fresh **unrouted** board; do not replace the routed files with it.
This new release does not overwrite the earlier handoff or unrelated working
source files. No fabrication order, commit or push is part of this handoff.

## License

Copyright (c) 2026 Tony Huang.

The project's hardware design sources, fabrication outputs and documentation
are licensed under CERN-OHL-P-2.0; see [LICENSE](LICENSE). Third-party material
retains its original licenses and notices, including the MIT attribution in
`source/main/src/display_parts.cohdl` and the model provenance in
[models/ATTRIBUTION.md](models/ATTRIBUTION.md).
