# SPDX-License-Identifier: MIT
"""Rev B placement region module. apply(b, P, P2) places this region's parts.
P(ref, side, x, y, rot); P2(ref, side, x, y, toward[, rots]) -- see placelib.py."""


def apply(b, P, P2):
    # ================================================================ audio (lower tongue): amplifier on F next to J1, mic group on B

    P('J1', 'F', 33.0, 84.9, 90)          # fixed; mating face at u 35.6 facing +u -> plug zone u 35.6-40, v 82.4-87.4 kept free

    # ---- NS4150B, F side. rot 270: input row (4 3 2 1, west->east) faces north toward the DAC pair,
    # output row (5 6 7 8) faces south toward the ferrites and J1.
    UX, UY = 41.5, 72.5
    P('U1', 'F', UX, UY, 270)
    # Rin at pins 3/4 (summing nodes ~1 mm, mirror-symmetric), Cin right behind them
    P2('R4', 'F', UX - 0.10, UY - 4.10, 'U1.3', (90, 270))
    P2('R5', 'F', UX - 1.20, UY - 4.10, 'U1.4', (90, 270))
    P2('C3', 'F', UX + 0.15, UY - 6.65, 'R4.1', (90, 270))
    P2('C4', 'F', UX - 1.45, UY - 6.65, 'R5.1', (90, 270))
    # bypass cap over pins 1/2, CTRL pull-down east of pin 1 (AMP_ENABLE leaves pin 1 eastward)
    P2('C1', 'F', UX + 2.00, UY - 3.90, 'U1.2', (0, 180))
    P2('R3', 'F', UX + 2.45, UY - 2.10, 'U1.1', (90, 270))
    # VDD decoupling straddling pins 6 (VDD) / 7 (GND); bulk 10 uF on B under it (supply arrives on B)
    P2('C11', 'F', UX, UY + 3.90, 'U1.6', (0, 180))
    P2('C12', 'B', UX, UY + 6.90, 'U1.6', (0, 180))
    # ferrites straight below pins 5/8 (VON west, VOP east), shunt caps at their far side, C10 across
    P2('L2', 'F', UX - 1.80, UY + 7.15, 'U1.5', (90, 270))
    P2('L1', 'F', UX + 1.80, UY + 7.15, 'U1.8', (90, 270))
    P('C10', 'F', UX, UY + 8.70, 90)           # pad 2 (MINUS) north, pad 1 (PLUS) south
    P2('C9', 'F', UX - 3.95, UY + 8.70, 'L2.2', (0, 180))
    P2('C8', 'F', UX + 3.95, UY + 8.70, 'L1.2', (0, 180))

    # ---- microphone group, B side
    P('MK1', 'B', 48.0, 70.5, 0)
    # TPS22918 rot 270: inputs (1 IO_3V3, 3 ON) on the north row, output 6 at the south-west corner
    P('U2', 'B', 51.5, 63.5, 270)
    P2('C5', 'B', 51.0, 60.6, 'U2.1', (0, 180))
    P2('R9', 'B', 53.6, 62.4, 'U2.3', (90, 270))
    P2('C6', 'B', 53.6, 64.9, 'U2.4', (90, 270))
    P2('R8', 'B', 51.5, 66.6, 'U2.5', (90, 270))
    P2('R7', 'B', 48.0, 65.6, (49.5, 65.6), (0, 180))
    P2('C7', 'B', 48.0, 76.3, 'MK1.1', (0, 180))  # 0.9 mm clear of the sound-post ring (review)
