# JLCPCB order package — OpenMicroDial r6 (BodyV6.13)

Prepared 2026-09-23 from the KiCad-saved r6 boards:
main `main-45deg.kicad_pcb` sha256 54822c39…, encoder `encoder-45deg.kicad_pcb` sha256 384a992f…, encoder-wheel `encoder-wheel-45deg.kicad_pcb` sha256 e1437f77….

**Updated later on 2026-09-23 for the microphone change:** MK1 is now the ST MP34DT05TR-A (LCSC C2688664) on the **bottom** side (the out-of-stock SPM1423HM4H-B was on top). The main Gerber zip, BOM, CPL and orientation list were regenerated from the new board; the encoder files did not change.

**Updated again on 2026-09-23 for the charger change:** U6 is now the TI BQ24073RGTR (LCSC C15220), because the SGM41562SA is out of stock. R37 and R38 changed value, C51 and R33 are gone, and C40, C41 and C48 moved. The main Gerber zip, BOM, CPL and orientation list were regenerated from the new board; the encoder files did not change.

**Updated on 2026-09-24 for the 3.3 V regulator:** U7 is now the adjustable RT6150BGQW (LCSC C147972, the same part as U8) with a new feedback divider R42 1M / R43 180k on the bottom, because the fixed RT6150B-33GQW is out of stock. The main Gerber zip, BOM, CPL and orientation list were regenerated again; the encoder files did not change.

**Updated again on 2026-09-24 for pin-1 marks:** a 0.3 mm silkscreen dot now marks pin 1 of U1, U2, U3, U4, U5, U7, U8, U10, U20 and Y1, which had none. Only the two silkscreen layers changed, so only `main/main-jlcpcb-gerber.zip` was rebuilt; the main BOM, CPL and orientation list are unchanged (the board's parts, positions and copper are the same).

**Added on 2026-09-24: an alternative encoder board, `encoder-wheel/`.** Its encoder ENC1 is on the bottom, inside the scroll wheel, and J1 stays on top. Order either `encoder/` or `encoder-wheel/`, not both. The encoder-wheel board needs the modified wheel described in `../encoder-wheel/README.md`. See [Encoder-wheel board](#encoder-wheel-board-alternative-enc1-on-the-bottom).

| Upload this | Main board | Encoder board | or: Encoder-wheel board |
|---|---|---|---|
| Gerber + drill (PCB order) | `main/main-jlcpcb-gerber.zip` | `encoder/encoder-jlcpcb-gerber.zip` | `encoder-wheel/encoder-wheel-jlcpcb-gerber.zip` |
| BOM (PCBA) | `main/main-bom-jlcpcb.csv` | `encoder/encoder-bom-jlcpcb.csv` | `encoder-wheel/encoder-wheel-bom-jlcpcb.csv` |
| CPL / pick-and-place (PCBA) | `main/main-cpl-jlcpcb.csv` | `encoder/encoder-cpl-jlcpcb.csv` | `encoder-wheel/encoder-wheel-cpl-jlcpcb.csv` |
| Placement check list | `main/main-orientation-check.csv` | `encoder/encoder-orientation-check.csv` | `encoder-wheel/encoder-wheel-orientation-check.csv` |

The Gerber zips hold the copper, mask, silkscreen, paste (for the stencil) and outline layers plus the PTH and NPTH Excellon drill files, all plotted by KiCad 10.0.4 from the hashed boards. They are the same plots as `main/`, `encoder/` and `encoder-wheel/` in the release, with the paste layers added.

## PCB order options

| Option | Main | Encoder |
|---|---|---|
| Layers | 4 | 2 |
| Size | 53.43 × 92.43 mm | 21.35 × 20.39 mm |
| Thickness | 1.0 mm | 1.0 mm |
| Stackup | **JLC04101H-3313** (the KiCad stackup is 3313 prepreg 0.0994 mm + 0.7 mm core, 1 oz outer / 0.5 oz inner) | standard |
| Surface finish | ENIG | ENIG |
| Smallest via | 0.20 mm hole / 0.45 mm pad (299 vias) | 0.20 mm / 0.45 mm |
| Smallest track / gap | 0.10 / 0.10 mm | 0.20 / 0.10 mm |
| Via covering | **Epoxy filled & capped** (charged on 4-layer). 31 vias sit in or touch SMD pads, including 5 in the U6 charger thermal pad and 2 each in the U7 and U8 thermal pads; open or tented vias there will wick some solder. Five more vias sit inside the case's microphone sound post and three touch its edge; open vias would leak sound past the mic to the back of the board. | Tented |
| Order-number mark | "Specify location" or remove (the encoder is small) | same |

## PCBA settings

- Main: assemble **both sides**. 120 parts on 43 BOM lines: 70 on top (F), 50 on bottom (B). Encoder: top only, 2 parts. Encoder-wheel: both sides, ENC1 on the bottom and J1 on top.
- Not assembled (in the Gerbers only): C52 and C53 (DNP), J4 cell/NTC wire pads, TP1–TP10 and the mounting holes.
- CPL format: millimetres, KiCad's absolute origin (the same origin as the Gerbers), Y negated, X not mirrored for bottom parts. Bottom rotation is 180 − KiCad angle, which is the convention used by JLCPCB's KiCad Fabrication Toolkit and by kicad-jlcpcb-tools. Top rotation is the KiCad angle.
- Positions are footprint origins. Five footprints have their origin away from the pad centre, so check that their bodies line up in JLC's placement preview. J5 USB-C is 1.48 mm off, ENC1 1.75 mm, J1/J3 PicoBlade 0.60 mm, SW1/SW2/SW5/SW6 0.55 mm, and the FH34 connectors 0.10 mm.
- JLC's library parts don't all use KiCad's zero orientation. In JLC's preview, check every part listed in `*-orientation-check.csv`. The file gives the board location where pin 1 must land, as seen from the top. On the main board every IC and the crystal also has a silkscreen dot beside pin 1.
- The older `main/assembly/positions.csv` in this release uses raw KiCad angles for bottom parts. Use the new CPL for JLC. The two differ for 27 bottom parts. Six of those are polarized: U2, U3, U10, SW3, SW4 and MK1.

## Parts that need a decision before ordering

| Line | Status on 2026-09-23 | What to do |
|---|---|---|
| Y1 E1SB48E001G00E (48 MHz crystal) | Not at LCSC or JLC | Global sourcing, or review C54561124 (TKD SX20Y048000B81T001: 48 MHz, 8 pF, ±10 ppm, ESR not listed) against SiFli's limits (CL 7–12 pF, ESR ≤ 30 Ω) |
| AE1 2450AT18B100E, SW3/SW4 PTS636SM25SMTRLFS, U5 SF32LB525UC6 | Out of stock at LCSC. JLC keeps separate stock. | Check JLC stock when the BOM is matched, then pre-order or use global sourcing. U5 also has a JLC listing, C9900153074. |
| U7/U8 RT6150BGQW (C147972) | About 1,350 in stock at LCSC on 2026-09-24; the board now uses 2 | Check stock at upload |
| R32 604k | Out of stock at LCSC; JLC showed some stock | Check at upload |
| R37 2.55k RC0402FR-072K55L (C185431) | About 700 in stock at LCSC on 2026-09-23 | Check at upload. If it runs out, 2.49k (357 mA) or 2.7k (330 mA) also work; the charge current is 890 / R37. |
| U6 BQ24073RGTR (C15220) | Extended part; about 16,900 in stock at LCSC on 2026-09-23 | Check stock at upload |
| MK1 MP34DT05TR-A (C2688664) | Extended part; about 577 in stock at LCSC on 2026-09-23 (JLC's page does not show its own stock) | Check stock at upload |

Leaving Y1 unmatched makes JLC ask about it at the BOM step.

## Microphone MK1 (bottom side)

- MK1 is placed on the bottom at (48.000, −70.500), JLC rotation 180 (KiCad angle 0 on the bottom layer). Its sound port must face away from the board, toward the case's front sound hole; that is the part's normal top-port orientation, so nothing special is needed beyond placing it on the bottom.
- Seen from the top of the board, pin 1 (Vdd) is at (47.550, −71.175), lower left of the part centre. In JLC's preview, check that pin 1 lands there and that the ground ring is centred on the four bars.
- It is an HCLGA (land grid) part with a ground ring; JLC may recommend X-ray inspection for it.

## Charger U6 (BQ24073RGTR, top side)

- U6 is a 3 × 3 mm VQFN-16 with an exposed thermal pad, placed on the top at (15.000, −10.000), JLC rotation 0. Seen from the top, pin 1 (TS) is at (13.537, −9.250), upper left of the part centre. Check that in JLC's preview.
- The thermal pad has five 0.2 mm vias to the ground planes. They need the epoxy filled & capped option above; open vias would pull solder out of the thermal pad.
- R37 (2.55k, ISET) sets the charge current to 349 mA. R38 (3.3k, ILIM) only matters in the BQ24073's resistor-limit mode; this board runs it in USB500 mode (500 mA input limit), but ILIM must still be connected or the chip won't charge.
- C40 (4.7 µF, battery), C41 (4.7 µF, USB input) and C48 (4.7 µF, system output) are the charger's capacitors and were moved next to it.

## 3.3 V regulator U7 (RT6150BGQW, top side)

- U7 makes the IO_3V3 rail (display logic via U10, touch controller, microphone via U2, pull-ups) from the battery / charger output. It is now the same adjustable RT6150BGQW as U8, in the same place and orientation; only its FB pin (pin 10) changed, from a direct link to the output to the divider.
- R42 (1M, IO_3V3 to FB) and R43 (180k, FB to GND) sit on the **bottom**, under U7: R42 at (35.500, −63.400), JLC rotation 180; R43 at (36.400, −64.900), JLC rotation 270. Output: 0.5 V × (1 + 1000/180) = 3.28 V.
- The new FB via (36.575, −62.975) and IO_3V3 via (35.200, −62.800) sit close to R42's pads (0.05–0.09 mm, same nets); filled & capped vias keep solder on the pads.

## Encoder-wheel board (alternative; ENC1 on the bottom)

- The PCB order options are the same as for the encoder board: 2 layers, 1.0 mm, ENIG, 0.20/0.45 mm vias (5), tented. Same outline and size.
- **Assemble both sides:** ENC1 on the **bottom** and J1 on the top, 2 parts. JLC charges for the second side, or J1 can be hand-soldered.
- ENC1 CPL: bottom, (47.500, −8.500), JLC rotation 0 (KiCad angle 180 on the bottom → 180 − 180). Seen from the top, terminal A is at (49.500, −12.650), lower right of the part centre, the mirror of the top-side board. In JLC's bottom-side preview, check that the three terminals sit on the pad row nearer J1, and the side tabs on the two big pads.
- The board has a "OMD ENC-W r6" label on the top silkscreen, to tell it apart from the r6 encoder board.

## Substitutions already in the BOM

Every change is listed in the BOM's "Part placed (if substituted)" column. To revert a line to the exact MPN, copy the "Exact-MPN C# (alt)" value into "LCSC Part #".

- Basic parts with identical specs, which avoid JLC's extended-part fee: all 0402 resistors except 86.6k, 604k, 2.55k, 3.3k and 180k (UniOhm 0402WGF…), C19/C20/C22 100 nF X7R, and C12 10 µF 25 V 0805.
- Basic parts with the same value, dielectric and size but a higher voltage rating: 1 µF 0603 (50 V, 9 pcs), 4.7 µF 0603 (16 V, 9 pcs) and 2.2 µF 0603 (16 V, 6 pcs).
- R38 3.3k: UniOhm 0402WGF3301TCE (C25890) and R43 180k: UniOhm 0402WGF1803TCE (C25760), the same series as the other substituted 0402 resistors. JLC shows at upload whether they count as Basic parts.
- C42–C47: TDK C3216X7R1C106KT000N, which has the same specs from the same maker; the exact TDK MPN is out of stock.
- C7: Murata GRM31C5C1H104JA01L, 100 nF C0G 1206 rated 50 V. The design MPN CC1206JRNPO8BN104 doesn't exist at any distributor.
- J2 and the encoder's J1: FH34SRJ-4S-0.5SH(50), the reel packaging of the same Hirose connector. The (99) small-lot pack isn't stocked.
