# SPDX-License-Identifier: MIT
"""U5 east/south escapes, U3 escapes and the touch/I2C links at U3 (hand-routed, locked).

East face: PANEL_RESET runs straight east to its via (44.95, 24.15) beside R30; the other
eight pins alternate between a near via column (u 43.35: v 24.6/25.4/26.2/27.0) and a far
column (u 44.2: v 25.0/25.8/26.6/27.4); far-column traces pass between near vias.
South face: twelve signal pins jog 45 deg west into two staggered via rows (S1 v 29.1 at
u 40.35 - 0.8k, S2 v 29.95 at u 39.95 - 0.8k); ADC_REF goes straight to C13; RESET's via
also feeds R39 (below the CHG via), CHG's via feeds R46 through the C13/R39 gap.
U3: even-row panel pins get vias above the pads (v 34.45), odd-row pins below (v 39.15,
in the cable lane where vias are allowed). Touch/I2C pins reach their pull-ups on B,
threading between the connector's pad rows.
"""
W = 0.12


def apply(R):
    P = R.pad
    pin = lambda n: P('U5', n)
    net = lambda n: R.padnet('U5', n)

    # ---------------------------------------------------------------- east face
    p42 = pin(42)
    R.path(net(42), 'B', [p42, (44.95, p42[1])], W)
    R.via(net(42), (44.95, 24.15))
    r301 = P('R30', 1)
    R.path(net(42), 'B', [(44.95, 24.15), (r301[0], 23.8), r301], W)
    E = {43: (43.35, 24.6), 44: (44.2, 25.0), 45: (43.35, 25.4), 46: (44.2, 25.8),
         47: (43.35, 26.2), 48: (44.2, 26.6), 49: (43.35, 27.0), 50: (44.2, 27.4)}
    for n, (vu, vv) in E.items():
        pu, pv = pin(n)
        x1 = 42.75 if n == 50 else 42.85          # CAL passes below the near 27.0 via
        pts = [(pu, pv), (x1, pv), (x1 + (vv - pv), vv), (vu, vv)]
        R.path(net(n), 'B', pts, W)
        R.via(net(n), (vu, vv))

    # ---------------------------------------------------------------- south face
    s1 = [round(40.35 - 0.8 * k, 3) for k in range(6)]               # v 29.1 (east end clear of Y1)
    s2 = [round(39.95 - 0.8 * k, 3) for k in range(6)]               # v 29.95
    slots = sorted([(u, 29.1) for u in s1] + [(u, 29.95) for u in s2])
    pins = [67, 66, 65, 64, 63, 62, 61, 60, 59, 58, 57, 56]          # west -> east
    svia = {}
    for n, (vu, vv) in zip(pins, slots):
        pu, pv = pin(n)
        d = pu - vu
        y0 = 28.15
        pts = [(pu, pv), (pu, y0), (vu, round(y0 + d, 3)), (vu, vv)]
        R.path(net(n), 'B', pts, W)
        R.via(net(n), (vu, vv))
        svia[net(n)] = (vu, vv)
    p68 = pin(68); c13 = P('C13', 1)
    R.path('ADC_REF', 'B', [p68, (p68[0], 28.1), (35.9, 28.4), (35.3, 28.4), c13], 0.15)
    vr = svia['RESET_ACTIVE_HIGH']; r39 = P('R39', 1)                # under the CHG via, up into R39.1
    R.path('RESET_ACTIVE_HIGH', 'B', [vr, (vr[0], 30.45), (r39[0], 30.45), r39], W)
    vc = svia['power::CHG_STATUS']; r46 = P('R46', 1)                # between C13 and R39
    R.path('power::CHG_STATUS', 'B', [vc, (vc[0] - 0.5, 29.45), (32.85, 29.45), r46], W)

    # ---------------------------------------------------------------- U3 panel pins
    north = {14: 35.85, 16: 36.55, 18: 37.25, 20: 37.95}
    for n, vu in north.items():
        pu, pv = P('U3', n)
        nn = R.padnet('U3', n)
        R.path(nn, 'B', [(pu, pv), (pu, 34.75), (vu, 34.75 - abs(pu - vu)), (vu, 34.45)], W)
        R.via(nn, (vu, 34.45))
    south = {15: 36.15, 17: 36.85, 19: 37.55, 21: 38.25}
    for n, vu in south.items():
        pu, pv = P('U3', n)
        nn = R.padnet('U3', n)
        R.path(nn, 'B', [(pu, pv), (pu, 38.85), (vu, 38.85 + abs(pu - vu)), (vu, 39.15)], W)
        R.via(nn, (vu, 39.15))

    # ---------------------------------------------------------------- touch / I2C pins -> pull-ups (B)
    t2 = P('U3', 2); r41 = P('R41', 2)
    R.path('TOUCH_IRQ', 'B', [t2, (t2[0], 34.7), (32.0, 34.35), (r41[0] + 0.2, 34.35), r41], W)
    t4 = P('U3', 4); r28 = P('R28', 2)
    R.path('I2C_SDA', 'B', [t4, (t4[0], 36.0), (31.2, 36.0), (r28[0] + 0.35, 35.75), (r28[0] + 0.2, r28[1])], W)
    R.path('I2C_SDA', 'B', [(r28[0] + 0.2, r28[1]), r28], W)
    t3 = P('U3', 3); r45 = P('R45', 1)
    R.path('TOUCH_RESET', 'B', [t3, (t3[0], 37.25), (31.0, 37.25), (r45[0] + 0.25, r45[1]), r45], W)
    t5 = P('U3', 5); r29 = P('R29', 2)
    R.path('I2C_SCL', 'B', [t5, (t5[0], 36.6), (31.0, 36.6), (r29[0] + 0.25, r29[1]), r29], W)
