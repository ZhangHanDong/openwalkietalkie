# SPDX-License-Identifier: MIT
"""Rev B placement region module. apply(b, P, P2) places this region's parts.
P(ref, side, x, y, rot); P2(ref, side, x, y, toward[, rots]) -- see placelib.py.

MCU cluster: U5 on B at (39.0, 24.5) rot 0 (pin faces: W 1-17, N 18-34, E 35-51, S 52-68).
Fan-out plan the placement leaves room for:
  W   supply pins 12-17 -> two cap columns NW; signal pins 5-11 -> via field SW of the caps
  N   18 LX -> L3; 19/20/22/23/24 (VSYS, CELL, LDO1, 1V8, VCC) -> via row north of the pads,
      their caps sit on F directly behind (v < 23.7, outside the battery reserve);
      25-27 (AMP_EN, UART) -> vias; 28-30 (IO3, CLK, IO0) -> B to U4's east row;
      31-33 (IO2, IO1, CS) -> vias NE, F.Cu to U4's west row
  E   35 PANEL_EN -> via; 36/37 DAC -> R2/R1 NE (B), C2 across, DAC_FILTER -> via -> F south;
      38/39 -> C14/C15; 42-50 panel bus + TE + CAL -> via field east (u 43.1-45.4, v 24.3-27.0)
  S   52 VSYS -> C16; 53 RF -> C57 (series 15 pF) -> line east at v 30.3 -> RF via (49.2, 30.6)
      -> F network; 54/55 -> Y1 below; 56-59 touch/I2C -> B south-west to U3;
      60-68 -> via field SW (u 34.5-38.5, v 29-31)
"""


def apply(b, P, P2):
    # ================================================================ MCU cluster (B side, under the display)
    UX, UY = 39.0, 24.5
    P('U5', 'B', UX, UY, 0)
    P('U4', 'B', 32.6, 14.2, 0)
    P2('C22', 'B', 36.45, 18.2, 'U4.8')

    # NW corner: buck inductor + W-face caps (two columns)
    P('L3', 'B', 34.0, 18.7, 180)
    P2('C17', 'B', 34.15, 20.4, 'U5.17')
    P2('C29', 'B', 34.15, 21.4, 'U5.16')
    P2('C30', 'B', 34.15, 22.4, 'U5.15')
    P2('C24', 'B', 32.2, 22.6, 'U5.14')
    P2('C25', 'B', 32.2, 23.6, 'U5.13')
    P2('C26', 'B', 32.2, 24.6, 'U5.12')
    P2('R6', 'B', 32.2, 26.6, 'U5.11')

    # N face supply caps on F directly behind the pad row (outside the battery reserve)
    P2('C31', 'F', 36.1, 17.55, 'U5.19', (90, 270))
    P2('C23', 'F', 37.2, 17.55, 'U5.22', (90, 270))
    P2('C27', 'F', 38.3, 17.55, 'U5.23', (90, 270))
    P2('C28', 'F', 39.4, 17.55, 'U5.24', (90, 270))

    # E face north: DAC RC filter NE of the corner, audio supply / reference caps at pins 38/39
    P2('R2', 'B', 44.6, 20.15, 'U5.36', (0, 180))
    P2('R1', 'B', 44.6, 21.15, 'U5.37', (0, 180))
    P2('C2', 'B', 46.6, 20.65, 'R1.2', (90, 270))
    P2('C14', 'B', 43.85, 22.95, 'U5.38', (0, 180))
    P('C15', 'F', 43.9, 23.15, 0)   # AUDIO_REF cap behind the E face (via from p39)

    # S face: AVDD_BRF cap, RF pi (C52 shunt DNP / C57 series / C53 shunt DNP), crystal
    P2('C16', 'B', 44.6, 28.05, 'U5.52', (0, 180))
    P2('C57', 'B', 43.4, 29.55, 'U5.53', (90, 270))
    P2('C52', 'B', 44.95, 29.05, 'C57.1', (0, 180))
    P2('C53', 'B', 43.4, 31.45, 'C57.2', (90, 270))
    P('Y1', 'B', 41.55, 31.0, 0)   # XI top-left (straight from p55), XO threads between the pads
    P2('C13', 'B', 34.5, 28.9, 'U5.68', (0, 180))
    P2('R39', 'B', 34.5, 30.0, 'U5.66', (0, 180))
    P2('R46', 'B', 32.5, 29.6, 'U5.67', (90, 270))

    # ================================================================ antenna corner (F)
    P('AE1', 'F', 52.2, 24.6, 90)
    # Johanson order from the antenna: feed stub, P2 shunt (C58), S2 series (R47), P1 shunt (L6), S1 series (R40)
    P2('R47', 'F', 52.2, 29.6, 'AE1.1', (90, 270))
    P2('C58', 'F', 50.75, 29.35, 'R47.2', (0, 180))
    P2('L6', 'F', 50.75, 30.6, 'R47.1', (0, 180))
    P2('R40', 'F', 52.2, 31.65, 'R47.1', (90, 270))
