# JLCPCB order package — OpenWalkieTalkie Rev B main board (BodyV6.13)

Prepared 2026-10-05 and rebuilt 2026-10-06 (R4/R5 back to 22k) from
`../main/main-revb.kicad_pcb`, using KiCad 10.0.4.
The board passes DRC with zones refilled at all severities: 0 violations,
0 unconnected.

This package covers the main board only. The encoder board did not change:
order r6's `encoder/` (or `encoder-wheel/`) package from
`../../rev-a/pcb-release-bodyv6.13-20260923-r6/fab-jlcpcb/`.

| Upload this | File |
|---|---|
| Gerber + drill (PCB order) | `main-revb-jlcpcb-gerber.zip` |
| BOM (PCBA) | `main-bom-jlcpcb.csv` |
| CPL / pick-and-place (PCBA) | `main-cpl-jlcpcb.csv` |
| Placement check list | `main-orientation-check.csv` |

The zip holds 13 files:
- 4 copper layers, 2 mask, 2 silkscreen, 2 paste (for the stencil) and the
  outline
- PTH and NPTH Excellon drill files

The same plots are in `../main/gerber`, `../main/stencil` and `../main/drill`.

## PCB order options

| Option | Main |
|---|---|
| Layers | 4 |
| Size | 53.43 × 92.43 mm (overall unchanged; bottom-right corner rounded vs r6 to match the top-left) |
| Thickness | **1.0 mm.** JLC's default is 1.6 mm, which would shift every case datum. |
| Stackup | **JLC04101H-3313**: 3313 prepreg 0.0994 mm + 0.7 mm core; 1 oz outer, 0.5 oz inner. The 50 Ω RF line on B.Cu (0.16 mm trace, 0.20 mm coplanar gaps over the In2 GND plane) is sized for this stackup. Do not substitute another stackup. |
| Surface finish | ENIG |
| Smallest via | 0.20 mm hole / 0.45 mm pad (all 477 vias) |
| Smallest track / gap | 0.12 / 0.10 mm |
| Via covering | **Epoxy filled & capped** (see below) |
| Order-number mark | **Remove order number.** The board has no "JLCJLCJLCJLC" placeholder; to use "Specify location", add one to the silkscreen first. |

**Why the vias must be filled and capped.** 48 vias sit in, or touch, SMD pads.
Open or tented vias there would wick solder:
- 16 GND vias in U5's exposed pad
- the thermal-pad vias of U6 (5), U7 (4) and U8 (4)
- vias in or at the pads of C1, C5, C7, C11, C12, C14/C15, C16, C39, R12,
  R27, R28, R29, U1, U20 and J5
- the CAL via in U5 pin 51, a 0.25 mm pad at 0.35 mm pitch
- the GND via beside the L6 shunt pad

## PCBA settings

- Assemble **both sides**: 125 parts on 51 BOM lines. 65 go on the bottom (B,
  the display side, which now carries the MCU group) and 60 on the top (F,
  the battery side).
- Not assembled; these exist in the Gerbers only:
  - C52, C53, C58 and L6, the RF tuning shunts (value "NF", DNP, no paste)
  - R44, the 10k bench NTC substitute (DNP, no paste)
  - test pads TP1–TP10
  - mounting holes MH2 and MH4

  The DNP parts and test pads are flagged excluded from BOM and position files
  in the board.
- CPL convention (same as r6):
  - millimetres, KiCad's absolute origin (the Gerber origin), Y negated
  - bottom-side parts: X not mirrored, rotation = 180 − KiCad angle
  - top-side parts: rotation = KiCad angle
- Positions are footprint origins. `main-orientation-check.csv` lists, for
  every polarized part:
  - the JLC rotation
  - where pin 1 must land, seen from the top
  - the offset from the origin to the centre of the bounding box of all pads,
    locating holes included: J5 1.48 mm, J1/J3 0.60 mm, SW1/SW2/SW5/SW6
    0.55 mm, J2 0.10 mm, U3 0.03 mm, as on r6

  Check every listed part in JLC's placement preview. Every IC and the crystal
  has a silkscreen pin-1 mark: a dot, except U6, which has the footprint's
  corner triangle.
- J5 (USB-C) has through-hole shell tabs, as on r6.
- USB-C and the side switches overhang the outline, as on r6, and there are
  no fiducials. JLC will add process rails; they must clear the overhangs.

## Parts that need a decision before ordering

| Line | What to do |
|---|---|
| Y1 E1SB48E001G00E (48 MHz crystal) | Not at LCSC/JLC (same as r6). Either use global sourcing, or review C54561124 (TKD SX20Y048000B81T001: 48 MHz, 8 pF, ±10 ppm, ESR not listed) against SiFli's limits (CL 7–12 pF, ESR ≤ 30 Ω). Leaving Y1 unmatched makes JLC ask about it at the BOM step. Do not let it go out as "do not place": the MCU has no 48 MHz reference without it. |
| J3 Molex 53261-0371 (LCSC C293630) | New part (MX1.25 / PicoBlade 1.25 mm, 3 positions, SMT right angle). Check stock at upload. It mates with a Molex 51021-0300 housing. **The harness must be pin 1 VCELL, pin 2 NTC (10k 103AT, B ≈ 3435), pin 3 GND.** MX1.25 does not fix polarity and nothing on the board protects against a reversed pack. Without an NTC the charger will not charge; fit R44 for such packs. |
| AE1 2450AT18B100E, SW3/SW4 PTS636SM25SMTRLFS, U5 SF32LB525UC6, R32 604k, U6 BQ24073RGTR, U7/U8 RT6150BGQW, MK1 MP34DT05TR-A | Same parts as r6. Check JLC stock when the BOM is matched; r6's stock notes are from 2026-09-23. |
| R37 3.6k RC0402FR-073K6L (LCSC C137990) | New value (2026-10-10): 0.5 C charge for the 500 mAh cell, 890 / 3.6k = 247 mA (r6 had 2.55k, 349 mA). About 590k in JLC stock on 2026-10-10. If it runs out, 3.57k (Yageo C137991 or UniOhm 0402WGF3571TCE C270609, 249 mA) or UniOhm 0402WGF3601TCE (C25891, 3.6k) also work; the charge current is 890 / R37. |
| C57 15 pF C0G (C1548), R40/R47 0R | The RF match ships as 15 pF series plus 0R / 0R, with the shunts open. Expect to tune S1/S2 (R40/R47) and the shunts (C52, C53, L6, C58) after a VNA measurement in the closed case. |

## After assembly (bring-up notes)

- **Firmware needs a Rev B variant.** The pin map changed:
  - The amplifier enable moved from PA20 to PA01.
  - The CAL test pad TP3 is now on PA00. Set the SiFli calibration/download
    tool's signal pin to PA00.
  - CHG is on PA33.

  See the README.
- First RF check: BLE RSSI and packet-error rate with the display streaming
  on and off. PANEL_D1/D3 run within 0.5 mm of the antenna's ground edge.
