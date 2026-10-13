# SPDX-License-Identifier: MIT
"""Hand-routed MCU-critical copper (B side unless noted). apply(R) with R = routelib.Router.

U5 at (39.0, 24.5) B rot 0. Pins: W face u 35.7 (p17 v 21.7 ... p1 v 27.3, step 0.35),
N face v 21.2 (p18 u 36.2 ... p34 u 41.8), E face u 42.3 (p35 v 21.7 ... p51 v 27.3),
S face v 27.8 (p52 u 41.8 ... p68 u 36.2). Pad outer ends are 0.275 beyond the pin centre.
"""
UX, UY = 39.0, 24.5


def apply(R):
    P = R.pad
    # ---------------------------------------------------------------- exposed-pad thermal vias (replace r6's PTH pads)
    for dx in (-2.5, -0.83, 0.83, 2.5):
        for dy in (-2.5, -0.83, 0.83, 2.5):
            R.via('GND', (round(UX + dx, 3), round(UY + dy, 3)))

    # ---------------------------------------------------------------- RF: p53 -> C57.1 ; crystal p54/p55 -> Y1
    p53 = P('U5', 53); c571 = P('C57', 1)
    R.path('BT_RF', 'B', [p53, (p53[0], 28.6), (c571[0] - 0.35, 28.6), c571], 0.16)
    c521 = P('C52', 1)                            # shunt (DNP) on the BT_RF node
    R.path('BT_RF', 'B', [c571, (c521[0], c571[1])], 0.16)
    c572 = P('C57', 2); c531 = P('C53', 1)
    R.path('rf::BT_RF_LINE', 'B', [c572, c531], 0.16)
    p54 = P('U5', 54); p55 = P('U5', 55)
    xo = P('Y1', 3); xi = P('Y1', 1)             # rot 0 on B: XI top-left, XO bottom-right
    R.path('XTAL48_XI', 'B', [p55, (p55[0], xi[1] - 0.3), (xi[0], xi[1])], 0.12)
    R.path('XTAL48_XO', 'B', [p54, (p54[0], 29.4), (41.55, 29.85), (41.55, 31.1), (41.8, 31.35), xo], 0.12)

    # ---------------------------------------------------------------- p52 AVDD_BRF (VSYS) -> C16
    p52 = P('U5', 52); c16 = P('C16', 1)
    R.path('MCU_VSYS', 'B', [p52, (p52[0], 28.15), (43.82, 28.15), c16], 0.2)

    # ---------------------------------------------------------------- DAC pins -> R2/R1 (NE), filter cap C2, vias to F
    p36 = P('U5', 36); p37 = P('U5', 37)
    r21 = P('R2', 1); r11 = P('R1', 1)
    R.path('DAC_N', 'B', [p36, (42.85, p36[1]), (43.4, 21.5), (43.4, 20.4), (r21[0], r21[1])], 0.15)
    R.path('DAC_P', 'B', [p37, (43.0, p37[1]), (43.75, 21.65), (r11[0], r11[1])], 0.15)
    r22 = P('R2', 2); r12 = P('R1', 2); c2n = P('C2', 2); c2p = P('C2', 1)
    R.path('audio::DAC_FILTER_N', 'B', [r22, (45.8, r22[1]), (46.1, c2n[1]), c2n], 0.15)
    R.path('audio::DAC_FILTER_P', 'B', [r12, (45.8, r12[1]), (46.1, c2p[1]), c2p], 0.15)
    # vias to F just west of C2; the pair then runs south on F (audio route module)
    R.via('audio::DAC_FILTER_N', (45.75, 19.2))
    R.path('audio::DAC_FILTER_N', 'B', [c2n, (46.6, 19.2), (45.75, 19.2)], 0.15)
    R.via('audio::DAC_FILTER_P', (46.6, 22.45))
    R.path('audio::DAC_FILTER_P', 'B', [c2p, (46.6, 22.45)], 0.15)

    # ---------------------------------------------------------------- audio supply/reference caps at p38/p39
    p38 = P('U5', 38); p39 = P('U5', 39)
    R.path('MCU_LDO1', 'B', [p38, (43.0, p38[1]), (43.3, P('C14', 1)[1]), P('C14', 1)], 0.2)
    R.path('AUDIO_REF', 'B', [p39, (42.75, p39[1]), (42.75, 23.35), (43.0, 23.6), (43.45, 23.6)], 0.15)
    R.via('AUDIO_REF', (43.45, 23.6))                       # C15 sits on F right behind
    R.path('AUDIO_REF', 'F', [(43.45, 23.6), P('C15', 1)], 0.2)


    # ---------------------------------------------------------------- crystal ground: each GND pad gets its own via
    g2 = P('Y1', 2); g4 = P('Y1', 4)              # top-right / bottom-left
    R.via('GND', (g2[0], 29.6)); R.path('GND', 'B', [g2, (g2[0], 29.6)], 0.25)
    R.via('GND', (g4[0], 32.45)); R.path('GND', 'B', [g4, (g4[0], 32.45)], 0.25)
