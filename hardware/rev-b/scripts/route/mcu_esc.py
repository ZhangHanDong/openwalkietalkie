# SPDX-License-Identifier: MIT
"""U5 escapes for the north and east faces (hand-routed, locked).

North face (pads v 20.925-21.475, pitch 0.35): every pin except LX drops to a via in two
staggered rows (A v 20.45 at u 36.45 + 0.8k, B v 19.6 at u 36.85 + 0.8k); B-row traces pass
between A-row vias (0.35 mm channel). LX goes on B straight to L3; the buck output (p17)
reaches C17/L3.2 around C17's GND pad. The six flash lines continue on F.Cu to vias beside
U4's pads; the N-face supply vias get short F traces to their decoupling caps (F, v 17.55).
East face (pads u 42.025-42.575): panel bus + CAL + PANEL_ENABLE drop to a staggered via field
(near column u 43.35, far column u 44.2) between C15 and C16.
"""
W = 0.12


def apply(R):
    P = R.pad
    def pin(n):
        return P('U5', n)

    # ---------------------------------------------------------------- LX and the buck output
    p18 = pin(18)
    R.path('BUCK_LX', 'B', [p18, (p18[0], 21.0), (35.55, 20.35), (35.0, 19.8), (35.0, 19.4)], 0.25)
    p17 = pin(17); c17 = P('C17', 1)
    R.path('BUCK_FB', 'B', [p17, (35.45, p17[1]), (35.2, 21.45), (35.2, 20.65), (34.95, 20.4), c17], 0.15)
    l32 = P('L3', 2)
    R.path('BUCK_FB', 'B', [(l32[0], 19.4), (33.55, 19.8), (34.4, 19.8), (c17[0], 20.06), c17], 0.25)

    # ---------------------------------------------------------------- north-face via fan-out
    rowA = [round(36.45 + 0.8 * k, 3) for k in range(8)]       # v 20.45
    rowB = [round(36.85 + 0.8 * k, 3) for k in range(7)]       # v 19.6
    slots = []
    for k in range(8):
        slots.append((rowA[k], 20.45))
        if k < 7:
            slots.append((rowB[k], 19.6))
    pins = [19, 20, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34]
    vias = {}
    for n, (vu, vv) in zip(pins, slots):
        if n == 25:                                   # PA20 is unused since AMP_ENABLE moved to PA01
            continue
        pu, pv = pin(n)
        net = R.padnet('U5', n)
        d = pu - vu                                   # jog left (>0) or right (<0)
        y_j = 20.86                                   # just above the pad end (20.925)
        y_k = y_j - abs(d)                            # 45-degree jog
        pts = [(pu, pv), (pu, y_j), (vu, round(y_k, 3)), (vu, vv)]
        if net == 'SYSTEM_SUPPLY':               # clear of the 1V8 jog next door
            vv = 20.35
            pts[-1] = (vu, vv)
        R.path(net, 'B', pts, W)
        R.via(net, (vu, vv))
        vias[net] = (vu, vv)

    # supply vias -> F caps (supply pad is the lower one, v 18.06). A-row vias sit on the centres of
    # the B-row gaps, so their F links go straight up through those gaps (0.12 mm there).
    def cap_pad(cap, net):
        return [P(cap, k) for k in ('1', '2') if R.padnet(cap, k) == net][0]
    vs = vias['MCU_VSYS']; c = cap_pad('C31', 'MCU_VSYS')
    R.path('MCU_VSYS', 'F', [vs, (vs[0], 18.55), (c[0] + 0.1, 18.33)], W)
    vl = vias['MCU_LDO1']; c = cap_pad('C23', 'MCU_LDO1')
    R.path('MCU_LDO1', 'F', [vl, (vl[0], 18.45), (c[0], 18.33)], W)
    v8 = vias['SIP_1V8']; c = cap_pad('C27', 'SIP_1V8')
    R.path('SIP_1V8', 'F', [v8, (v8[0], 18.85), (v8[0] + 0.3, 18.55), (c[0] - 0.1, 18.33)], W)
    vc = vias['SYSTEM_SUPPLY']; c = cap_pad('C28', 'SYSTEM_SUPPLY')
    R.path('SYSTEM_SUPPLY', 'F', [vc, (vc[0], 19.15), (c[0] - 0.2, 19.15), (c[0], 18.95), (c[0], 18.33)], W)

    # ---------------------------------------------------------------- flash: vias beside U4's pads (autorouter joins them
    # to the N-face vias on F.Cu); short B stubs from the pads to the vias
    for pn, vpt in ((5, (37.75, 12.3)), (6, (37.75, 13.57)), (7, (37.75, 14.84)),
                    (3, (27.45, 13.57)), (2, (27.45, 14.84)), (1, (27.45, 16.1))):
        pp = P('U4', pn)
        net = R.padnet('U4', pn)
        R.via(net, vpt)
        R.path(net, 'B', [pp, vpt], W)

    # PANEL_ENABLE (p35) north-east to a via clear of the DAC traces
    p35 = pin(35)
    R.path('PANEL_ENABLE', 'B', [p35, (42.7, p35[1]), (42.7, 20.45)], W)
    R.via('PANEL_ENABLE', (42.7, 20.45))
