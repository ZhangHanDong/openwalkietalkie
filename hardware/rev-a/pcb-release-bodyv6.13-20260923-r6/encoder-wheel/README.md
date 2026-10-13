# Encoder board, wheel-mounted variant (`encoder-wheel`)

This folder holds an **alternative** to the r6 encoder board in
[`../encoder/`](../encoder/). The two boards have the same outline, holes,
connector J1 and its position, but the Alps EC05E1220401 encoder (ENC1) sits
on the other side:

| | `encoder/` (r6) | `encoder-wheel/` (this folder) |
|---|---|---|
| ENC1 side | F (back-cover side), hanging below the board to Y −0.34 | **B (wheel side), inside a cavity in the wheel** |
| Wheel | BodyV6.13 wheel as exported: hex shaft (1.905 across flats) through the board into ENC1 from the PCB side | **Modified wheel** ([`mechanical/wheel-with-encoder-cavity-bodyv6.13.step`](mechanical/wheel-with-encoder-cavity-bodyv6.13.step)): hex stub 1.68 across flats into ENC1 from its top face, the direction the Alps drawing specifies |
| Below the board (toward the back cover) | ENC1 (2.84 mm) and J1 | J1 only (1.1 mm, bottom at Y 1.42) |
| Assembly | one side (F) | **two sides**: ENC1 on B, J1 on F |
| Wheel direction | reference | the same: terminals A and B are swapped on the board, so firmware needs no change |

Use one or the other, not both. This board needs the modified wheel: the
unmodified V6.13 wheel collides with the flipped encoder.

## What changed on the board

- ENC1 flipped to B.Cu at the same place (47.5, 8.5), terminals toward J1
  (the part is turned over about the v axis, so terminal A is now at
  u 49.5 and terminal B at u 45.5). The Alps land pattern, the 3.0 × 3.0 PCB
  cut-out under the rotor and the two Ø0.62 positioning holes are unchanged.
- **A/B swap:** terminal A goes to `ENC_B_CONTACT` (J1 pin 3) and terminal B to
  `ENC_A_CONTACT` (J1 pin 2). Turning the encoder over reverses its
  rotation sense as seen from the wheel side. The swap cancels that, so the wheel
  reports the same direction as with the r6 board. As a side effect, the F.Cu
  routing to J1 is exactly the r6 routing.
- Copper: the three short F.Cu pad stubs became B.Cu stubs, and two vias
  (0.45/0.2) were added at (45.5, 13.9) and (50.65, 13.8). Minimum clearances
  0.175 mm (as r6), GND pours on both layers as before.
- Silkscreen: "OMD ENC-W r6" on the back (F) side, so the two boards can be told
  apart. User.Comments shows the wheel (Ø17) and its cavity (Ø10.6).

## Files

| What | File |
|---|---|
| KiCad board / project | [encoder-wheel-45deg.kicad_pcb](encoder-wheel-45deg.kicad_pcb), [.kicad_pro](encoder-wheel-45deg.kicad_pro) |
| Gerbers + drill | [encoder-wheel-gerber-drill.zip](encoder-wheel-gerber-drill.zip) ([gerber/](gerber/), [drill/](drill/)); paste layers in [stencil/](stencil/) |
| Assembly | [assembly/bom.csv](assembly/bom.csv), [assembly/positions.csv](assembly/positions.csv) (raw KiCad angles, like `../encoder/assembly/`) |
| KiCad placement export | [reference/positions-kicad-all.csv](reference/positions-kicad-all.csv) |
| JLCPCB package | [../fab-jlcpcb/encoder-wheel/](../fab-jlcpcb/encoder-wheel/) (see `../fab-jlcpcb/ORDER-NOTES.md`) |
| 3D | [../3d/encoder-wheel-populated-native.step](../3d/encoder-wheel-populated-native.step), [../3d/encoder-wheel-populated-bodyv6.13.step](../3d/encoder-wheel-populated-bodyv6.13.step) (BodyV6.13 frame), [../3d/bodyv6.13-encoder-wheel-fit-assembly.step](../3d/bodyv6.13-encoder-wheel-fit-assembly.step) (case with the modified wheel + main + this board) |
| Modified wheel (proposal) | [mechanical/wheel-with-encoder-cavity-bodyv6.13.step](mechanical/wheel-with-encoder-cavity-bodyv6.13.step) |
| Checks | [verification/encoder-wheel-drc-kicad10.rpt](verification/encoder-wheel-drc-kicad10.rpt), [verification/drill-report.txt](verification/drill-report.txt), [verification/encoder-wheel-fit-report.json](verification/encoder-wheel-fit-report.json) |

## Wheel changes (the mechanical side of this board)

In BodyV6.13 coordinates (wheel axis at Fusion X 19.5, Z 39.0):

- Remove the lower hex shaft (Y 0.5–3.5) and the Ø6.2 collar (Y 3.5–4.0). The
  wheel's underside stays at Y 4.0, 0.5 mm above the board.
- Cavity Ø10.6 from the underside up to **Y 6.8**. The rim wall is 2.2 mm at the grip
  valleys.
- Hex stub **1.68 across flats (0/−0.02)** from the cavity roof down to Y 4.1:
  about 2.1 mm into ENC1's 2.7 mm hex bore (1.72 across flats, minimum 1.70),
  tip 0.6 mm above the PCB, 0.2 mm chamfer on the tip. Print it in resin
  (SLA/MJF) or mould it; FDM is too coarse for this size.
- Suggested: a thrust ring about 0.3 mm tall under the rim at r ≈ 6.5 that
  rides on the bare board, so the wheel never rests on the encoder. Tighten
  the wheel's top bore (Ø2.6 on the case's Ø2.19 pin) to about Ø2.3 so side
  loads go to the case, not to the encoder rotor.

## Checks

- KiCad 10.0.4 DRC with zones refilled: 0 violations, 0 unconnected, 0
  footprint errors.
- Compared with `../encoder/` as parsed Gerber objects: Edge_Cuts and the
  NPTH drill file are identical. The PTH file gained the two vias. ENC1's
  pads, mask and paste moved from F to B. F_Cu changed only at the pad
  stubs and vias, and the F silkscreen gained the label.
- 3D (KiCad's own export, aligned with the board B face at Y 3.5): no
  interference with the modified wheel, the case or the main board. ENC1 is
  ≥ 0.355 mm from the wheel body, and the only near contact is the hex stub
  in the bore (0.02 mm per side). Wheel to board 0.5 mm; wheel to case 0.2 mm
  and to the main board 0.32 mm (unchanged).
