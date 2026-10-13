# SPDX-License-Identifier: MIT
"""Rev B placement region module. apply(b, P, P2) places this region's parts.
P(ref, side, x, y, rot); P2(ref, side, x, y, toward[, rots]) -- see placelib.py.

Region "topleft": charger / USB / battery (F, top-left), encoder RC (F, south of J2),
side-button RC (F, u 6.8), PTT RC (F, under SW5), macro-switch RC (F, behind SW3/SW4),
UART/debug pads (F, below the battery, under the MCU).

Routing plan the placement leaves room for:
  VBUS   J5 A9/B4 pad -> down to v~9.3 -> west -> north along u~22.4 -> west along v~3.3 (TP9)
         -> C41 west pad -> U6.13 (straight down). A4/B9 tied on B.
  SYS    U6.10/11 -> C48 top pad -> east -> TP7 -> SE into the corridor between J3
         (u<=22.34) and U20 (u>=23.85) -> south into the battery reserve (F tracks allowed).
  CELL   U6.2/3 -> C40 east pad -> south along u~12.2 -> under J3's label strip -> J3.1 (F);
         TP2 west of the trunk.
  TS     U6.1 -> R44 east pad -> via -> B -> via (17.0, 12.5) -> J3.2 (crosses CELL on B).
  CHG    U6.9 -> SE under C48 -> TP1 -> via -> B toward R46 / U5.67.
  EN1    U6.6 (SYS) -> via -> B -> SYS (or around C48 on F if CHG drops to B at the pin).
  USB    J5 A7/A6 (N west of P) -> U20.3/U20.1 (flow-through, rot -90) -> U20.4/6 south past
         the FFC loop (traces only) -> battery reserve -> MCU S-face via field.
  ENC    J2.2/J2.3 exit east, turn south at u~34.4/34.75; A runs west at v~19.35 above the B
         parts; outputs leave C38/C39 southward to the MCU S-face via field.
"""


def apply(b, P, P2):
    # ================================================================ switches (centred on their caps)

    P('SW5', 'F', 52.24, 35.743, 90)
    P('SW3', 'B', 39.7225, 57.09, 0)
    P('SW4', 'B', 16.2775, 57.09, 0)

    # PTT RC under SW5 (F), right of the battery reserve, >= 10 mm below the antenna clearance.
    # PTT_CONTACT comes down SW5's west side (u~49.6) into the two top pads; PTT leaves west
    # between R20 and C32 (F under the battery); R12's 3V3 pad gets a via to B.
    P2('R20', 'F', 49.5, 39.65, 'SW5.2', (90, 270))
    P2('R12', 'F', 50.75, 39.65, 'SW5.2', (90, 270))
    P2('C32', 'F', 49.5, 41.75, 'R20.2', (90, 270))

    # Macro switches: RC on F directly behind the contact pad (pad 1, B side); the contact
    # reaches the B pad through one via between the two resistor bottoms.
    #   SW3: C36 | R24 | R16  -> MACRO_LEFT exits north on the west (left of the debug pads),
    #        3V3 on the east.
    P2('R24', 'F', 35.85, 53.6, 'SW3.1', (90, 270))
    P2('R16', 'F', 37.1, 53.6, 'SW3.1', (90, 270))
    P2('C36', 'F', 34.6, 53.6, 'R24.2', (90, 270))
    #   SW4: R17 | R25 | C37  -> MACRO_RIGHT exits north-east, 3V3 west (via to B, C44/U7).
    P2('R17', 'F', 11.15, 53.6, 'SW4.1', (90, 270))
    P2('R25', 'F', 12.4, 53.6, 'SW4.1', (90, 270))
    P2('C37', 'F', 13.65, 53.6, 'R25.2', (90, 270))

    # Side buttons (F, r6 column at u 6.81, snapped to the 0.05 grid): C | Rseries | Rpullup
    for (c, rs, rp), v0 in ((('C33', 'R21', 'R13'), 17.05), (('C34', 'R22', 'R14'), 29.05), (('C35', 'R23', 'R15'), 41.05)):
        P(c, 'F', 6.8, v0, 90)
        P(rs, 'F', 6.8, v0 + 1.9, 90)
        P(rp, 'F', 6.8, v0 + 3.8, 90)

    # ================================================================ debug pads (F, below the battery, under the MCU)
    # r6 2x3 pattern for the DAPLink jig; columns under the MCU so TX/RX (N face), RESET (S face)
    # and CAL (E face) drop straight down; pads 2.2 mm below the battery reserve (room for a
    # pogo block beside the cell), MACRO_LEFT passes west of the group.
    u0, v0 = 37.5, 47.0
    # r6 had these on B; seen from F the 2x3 jig pattern is mirrored, so the rows swap (review)
    for r, (du, dv) in {'TP6': (0, 0), 'TP4': (0, 2), 'TP3': (2.85, 0), 'TP10': (2.85, 2), 'TP5': (4.4, 0), 'TP8': (4.4, 2)}.items():
        P(r, 'F', u0 + du, v0 + dv, 0)

    # ================================================================ charger / USB / battery (F, top-left)

    P('J3', 'F', 17.0, 16.5, 0)
    P('U6', 'F', 15.0, 8.2, 0)                      # W: TS BAT BAT CE  N: ISET TD TMR IN  E: ILIM OUT OUT CHG
    P2('C41', 'F', 16.6, 5.1, 'U6.13', (0, 180))    # IN cap: VBUS pad straight above pin 13, GND east
    P2('R37', 'F', 14.25, 4.9, 'U6.16', (90, 270))  # ISET above pin 16
    P2('R38', 'F', 18.4, 7.1, 'U6.12', (0, 180))    # ILIM east of pin 12
    P2('C48', 'F', 18.5, 9.35, 'U6.10', (90, 270))  # OUT cap: SYS pad facing pins 10/11, GND below
    P2('C40', 'F', 11.1, 8.55, 'U6.3', (0, 180))    # BAT cap: CELL pad facing pins 2/3, GND west
    P2('R44', 'F', 11.5, 6.95, 'U6.1', (0, 180))    # TS bench 10k (DNP) at pin 1
    P('TP9', 'F', 19.6, 3.1, 0)                     # VBUS, on the north VBUS run
    P('TP1', 'F', 20.4, 11.6, 0)                    # CHG, SE of C48 (CHG drops to B here)
    P('TP2', 'F', 13.4, 12.7, 0)                    # CELL, on the trunk between C40 and J3.1
    P('TP7', 'F', 21.0, 9.6, 0)                     # SYS, on the trunk between C48 and the corridor

    # USB: ESD array flow-through between J5's middle D-/D+ pads and the MCU, west of the
    # F flash-routing strip (u >= 27.5, v 11.6-14.6) and north of the encoder FFC loop.
    P('U20', 'F', 25.55, 12.8, -90)                 # pins 3/2/1 (C_N, GND, C_P) face J5
    P2('R10', 'B', 29.6, 10.1, 'J5.A5', (0, 180))   # CC1 5.1k, B side under J5's CC1 pad
    P2('R11', 'B', 25.35, 10.0, 'J5.B5', (0, 180))  # CC2 5.1k

    # ================================================================ encoder RC (F, south of J2)
    # Columns (west -> east): R26 series A | R18 pull-up A | R19 pull-up B | R27 series B,
    # contact pads on top (A's contact runs west at v~19.35 above the B parts, B's at v~20);
    # the two pull-up 3V3 pads share one via between them, C38/C39 sit under the series
    # resistors with their GND pads facing each other (one shared GND via) and the ENC pads
    # in line, so ENC_A/ENC_B continue straight south into the battery reserve (F tracks).
    # Placed over B-free board (west of the MCU's B cap columns at u >= 31.27) so neither side
    # loses via room; clear of the F flash strip (v <= 14.6), the MCU cap zone (u >= 34.8,
    # v 18.5-21.7), the encoder FFC loop (u 25.3-30.3, v 15.5-18.1), the battery reserve
    # (v >= 23.7), and leaving the u 22.4-25.9 corridor for SYSTEM_SUPPLY and the USB pair.
    P2('R26', 'F', 26.35, 20.5, 'J2.2', (90, 270))
    P2('R18', 'F', 27.6, 20.5, 'J2.2', (90, 270))
    P2('R19', 'F', 28.85, 20.5, 'J2.3', (90, 270))
    P2('R27', 'F', 30.1, 20.5, 'J2.3', (90, 270))
    P2('C38', 'F', 26.85, 22.25, 'R26.2', (0, 180))
    P2('C39', 'F', 29.6, 22.25, 'R27.2', (0, 180))
