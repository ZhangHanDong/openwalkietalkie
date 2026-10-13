# SPDX-License-Identifier: MIT
"""Hand-routed power copper: USB-C input, BQ24073 charger, SYSTEM_SUPPLY trunk, the two RT6150B
buck-boosts (U8 PANEL_4V, U7 IO_3V3), the TPS22918 panel-logic switch (U10) and the rails to the
display connector U3 and the tongue interface vias. apply(R) with R = routelib.Router.

Layer plan (F = battery side, B = display side; In1/In2 are solid GND):
  USB/charger   F around J5/U6; the second VBUS pad joins over B (via-in-pad, both VBUS pads);
                D+ crosses D- through a B jumper, D- stays on F; CC1/CC2 drop to R10/R11 on B.
  CELL          F only, 0.8 mm: J3.1 -> TP2 -> west of U6 -> C40 + BAT pins.
  TS / EN1      the two strap/sense nets that would cross CELL/CHG hop over B (thin).
  SYSTEM_SUPPLY F trunk 1.0 mm: OUT pins -> C48 -> TP7 -> between J3 and U20 -> south-west under the
                battery -> u 17 past the regulators (vias into C45/C42 and the VIN-A/EN pins) ->
                south-east diagonal -> down the tongue west edge -> (36, 74) [audio module's via].
  regulators    B, power row south, CIN west / COUT east, CIN/COUT GND pads tied straight into the
                exposed pad, EP + cap GND vias, FB sense up the slot between the two output caps.
  PANEL_4V      B C46/C47 -> via -> F loop north of U3's cap row -> via -> C18 -> C19 -> U3.10.
  IO_3V3        B U7 out -> south of U10 -> C49/U10.1 -> via; F north-east to the U3 cap row
                (R48.1/C20/U3.8, R48.2 -> U3.6) and F south-east to (43.6, 52.8), then B across the
                DAC corridor (south of v 52) to the tongue interface via (49.4, 60.6).
  PANEL_LOGIC   B U10.6 -> R35 -> C21 -> under U3's west half (v 40.35) -> U3.9 / U3.11.
"""
W_SYS = 1.0     # SYSTEM_SUPPLY trunk
W_CELL = 0.8    # battery
W_VBUS = 0.5
W_USB = 0.2     # USB data
W_SIG = 0.15    # straps, FB, status
W_LX = 0.4

# trunk geometry (F)
SYS_U = 17.0          # trunk column west of the regulator caps
SYS_V_U8 = 32.3       # branch to U8 pin 8 (VIN-A) via
SYS_V_U7 = 40.2       # branch to U7 pins 6/8 via
SYS_V_C42 = 46.05     # branch to C42 vias; trunk turns south-east here
TONGUE_U = 30.3       # trunk column down the tongue west edge
TONGUE_VIA = (36.0, 74.0)
IO_VIA = (49.4, 60.6)


def apply(R):
    P = R.pad
    F, B = 'F', 'B'

    # ================================================================ USB-C J5 (F) + ESD U20 (F)
    # VBUS: west pad (25.6) runs on F to TP9 / C41 / U6.13; the east pad (30.4) joins over B
    vb_w = P('J5', 'A9'); vb_e = P('J5', 'A4')
    R.via('USB_VBUS', (vb_w[0], 8.3))                 # via-in-pad (filled + capped)
    R.via('USB_VBUS', (vb_e[0], 8.3))
    R.path('USB_VBUS', B, [(vb_w[0], 8.3), (vb_e[0], 8.3)], W_VBUS)
    tp9 = P('TP9', 1); c41v = P('C41', 1); u6in = P('U6', 13)
    R.path('USB_VBUS', F, [vb_w, (vb_w[0], 9.5), (23.0, 9.5), (22.3, 8.8), (22.3, 4.4), (21.0, tp9[1]), tp9,
                           (19.1, 3.6), (16.3, 3.6), (u6in[0], 4.15), (u6in[0], c41v[1])], W_VBUS)
    R.path('USB_VBUS', F, [(u6in[0], c41v[1]), u6in], 0.3)
    # ESD VBUS reference: via-in-pad at U20.5, B branch west of R11 back to the VBUS via
    u205 = P('U20', 5)
    R.via('USB_VBUS', (u205[0], 14.2))
    R.path('USB_VBUS', B, [(vb_w[0], 8.3), (25.0, 8.9), (24.3, 8.9), (24.0, 9.2), (24.0, 13.6), (24.6, 14.2),
                           (u205[0], 14.2)], 0.3)

    # D+ (USB_C_P: B6 27.25, A6 28.25) -> U20.1 (26.5): both pads drop to vias, joined on B, back up
    # beside U20.1. D- (USB_C_N: A7 27.75, B7 28.75) -> U20.3 (24.6) stays on F under the D+ vias.
    b6 = P('J5', 'B6'); a6 = P('J5', 'A6'); a7 = P('J5', 'A7'); b7 = P('J5', 'B7')
    u201 = P('U20', 1); u203 = P('U20', 3)
    vp1 = (b6[0], 9.45); vp2 = (a6[0], 9.45); vp3 = (26.85, 10.8)
    for v in (vp1, vp2, vp3):
        R.via('USB_C_P', v)
    R.path('USB_C_P', F, [b6, vp1], W_USB)
    R.path('USB_C_P', F, [a6, vp2], W_USB)
    R.path('USB_C_P', B, [vp2, vp1, (vp1[0], 10.4), vp3], W_USB)
    R.path('USB_C_P', F, [vp3, (u201[0], 11.15), u201], W_USB)
    R.path('USB_C_N', F, [a7, (a7[0], 10.2)], W_USB)
    R.path('USB_C_N', F, [b7, (b7[0], 10.2), (u203[0], 10.2), u203], W_USB)

    # CC1 / CC2 -> pull-downs R10 / R11 on B
    a5 = P('J5', 'A5'); b5 = P('J5', 'B5')
    r101 = P('R10', 1); r111 = P('R11', 1)
    cc1v = (a5[0], 9.45); cc2v = (b5[0], 9.3)
    R.via('CC1', cc1v); R.via('CC2', cc2v)
    R.path('CC1', F, [a5, cc1v], W_SIG)
    R.path('CC1', B, [cc1v, (cc1v[0], r101[1])], 0.2)
    R.path('CC2', F, [b5, cc2v], W_SIG)
    R.path('CC2', B, [cc2v, (r111[0], cc2v[1] + (cc2v[0] - r111[0])), r111], 0.2)

    # CC2 pull-down GND + ESD GND share one via between R11 and U20.2 (the D- run boxes R11.2 in)
    r112 = P('R11', 2); u202 = P('U20', 2)
    gv = (25.4, 10.75)
    R.via('GND', gv)
    R.path('GND', B, [r112, (r112[0], round(gv[1] - (gv[0] - r112[0]), 3)), gv], 0.25)
    R.path('GND', F, [gv, (u202[0], round(gv[1] + (u202[0] - gv[0]), 3)), u202], 0.3)

    # receptacle GND pads into the shell legs (PTH -> inner planes)
    for gp, sh in (('A12', 'SH2'), ('A1', 'SH1')):
        g = P('J5', gp); s = P('J5', sh)
        R.path('GND', F, [(g[0], 7.6), (s[0], 7.6)], 0.4)

    # ================================================================ charger U6 (F)
    # pins: W 1 TS, 2/3 BAT, 4 GND | S 5 GND, 6 EN1(SYS), 7 nc, 8 GND | E 9 CHG, 10/11 OUT, 12 ILIM
    #       N 13 IN, 14 nc, 15 GND, 16 ISET | 17 EP
    UX, UY = 15.0, 8.2
    for dx, dy in ((-0.55, -0.55), (0.55, -0.55), (-0.55, 0.55), (0.55, 0.55), (0.0, 0.0)):
        R.via('GND', (round(UX + dx, 3), round(UY + dy, 3)))      # via-in-pad, EP (<= 1 mm from each GND pin)
    p4 = P('U6', 4); p5 = P('U6', 5); p8 = P('U6', 8); p15 = P('U6', 15)
    R.path('GND', F, [p4, (14.3, p4[1])], 0.25)
    R.path('GND', F, [p5, (p5[0], 8.9)], 0.25)
    R.path('GND', F, [p8, (p8[0], 8.9)], 0.25)
    R.path('GND', F, [p15, (p15[0], 7.5)], 0.25)

    # CELL: BAT pins 2/3 -> C40 -> down the west side -> TP2 -> J3.1
    bat_v = (P('U6', 2)[1] + P('U6', 3)[1]) / 2
    p2u = P('U6', 2)[0]
    c401 = P('C40', 1); tp2 = P('TP2', 1); j31 = P('J3', 1)
    cu = 12.4
    R.path('CELL', F, [(p2u, bat_v), (cu, bat_v), (cu, tp2[1] - 1.0), tp2, (j31[0] - 0.9, tp2[1]),
                       (j31[0], tp2[1] + 0.9), j31], W_CELL)
    R.path('CELL', F, [(cu, bat_v), (c401[0], bat_v)], W_CELL)
    # BAT cap GND
    c402 = P('C40', 2)
    R.via('GND', (c402[0], 9.65)); R.via('GND', (9.4, c402[1]))
    R.path('GND', F, [c402, (c402[0], 9.65)], 0.4)
    R.path('GND', F, [c402, (9.4, c402[1])], 0.4)

    # TS: U6.1 -> R44.1 (DNP) on F, via -> B -> J3.2
    p1 = P('U6', 1); r441 = P('R44', 1); j32 = P('J3', 2)
    tsv1 = (12.8, 6.55); tsv2 = (j32[0], 12.75)
    R.path('power::CELL_TEMPERATURE', F, [p1, (tsv1[0], p1[1]), tsv1], W_SIG)
    R.path('power::CELL_TEMPERATURE', F, [(tsv1[0], r441[1]), r441], W_SIG)
    R.via('power::CELL_TEMPERATURE', tsv1)
    R.path('power::CELL_TEMPERATURE', B, [tsv1, (12.8, 12.0), (13.55, tsv2[1]), tsv2], W_SIG)
    R.via('power::CELL_TEMPERATURE', tsv2)
    R.path('power::CELL_TEMPERATURE', F, [tsv2, j32], 0.2)

    # J3.3 battery return: two GND vias right at the pad
    j33 = P('J3', 3)
    R.via('GND', (j33[0], 12.7)); R.via('GND', (19.1, 13.75))
    R.path('GND', F, [j33, (j33[0], 12.7)], 0.5)
    R.path('GND', F, [(j33[0], 13.75), (19.1, 13.75)], 0.4)

    # ISET / ILIM
    p16 = P('U6', 16); r371 = P('R37', 1)
    R.path('power::CHG_ISET', F, [p16, r371], 0.2)
    p12 = P('U6', 12); r381 = P('R38', 1)
    R.path('power::CHG_ILIM', F, [p12, (r381[0] - 0.59, p12[1]), (r381[0] - 0.24, r381[1]), r381], W_SIG)

    # IN cap GND
    c412 = P('C41', 2)
    R.via('GND', (18.35, c412[1]))
    R.path('GND', F, [c412, (18.35, c412[1])], 0.5)

    # OUT -> SYSTEM_SUPPLY bar -> C48 -> TP7 ; C48 GND via
    p10 = P('U6', 10); p11 = P('U6', 11); c481 = P('C48', 1); c482 = P('C48', 2); tp7 = P('TP7', 1)
    out_v = (p10[1] + p11[1]) / 2
    R.path('SYSTEM_SUPPLY', F, [(p10[0], out_v), (19.9, out_v), (tp7[0], out_v + 1.1), tp7],
           W_CELL)
    R.via('GND', (19.45, c482[1]))
    R.path('GND', F, [c482, (19.45, c482[1])], 0.4)

    # CHG -> TP1 (around the west/south of C48)
    p9 = P('U6', 9); tp1 = P('TP1', 1)
    R.path('power::CHG_STATUS', F, [p9, (17.0, p9[1]), (17.45, 9.4), (17.45, 10.9), (18.15, tp1[1]), tp1], W_SIG)

    # EN1 (U6.6, tied to SYSTEM_SUPPLY): via south of the pin, B under the charger, back up on the trunk
    p6 = P('U6', 6)
    env1 = (p6[0], 10.65); env2 = (22.9, 12.6)
    R.path('SYSTEM_SUPPLY', F, [p6, env1], 0.2)
    R.via('SYSTEM_SUPPLY', env1)
    R.path('SYSTEM_SUPPLY', B, [env1, (15.25, 11.15), (22.4, 11.15), (22.9, 11.65), env2], 0.2)
    R.via('SYSTEM_SUPPLY', env2)

    # ================================================================ SYSTEM_SUPPLY trunk (F)
    R.path('SYSTEM_SUPPLY', F, [tp7, (22.9, 11.5), (22.9, 20.2), (SYS_U, 20.2 + (22.9 - SYS_U)),
                                (SYS_U, SYS_V_C42), (TONGUE_U, SYS_V_C42 + (TONGUE_U - SYS_U)),
                                (TONGUE_U, TONGUE_VIA[1] - 2.0), (TONGUE_U + 2.0, TONGUE_VIA[1]), TONGUE_VIA], W_SYS)

    # ================================================================ U8: PANEL_4V buck-boost (B)
    # pins S row (v 35.725): 1 VOUT 23.0, 2 LX2, 3 PGND, 4 LX1, 5 VIN 21.0 ; N row (v 33.275): 6 EN 21.0,
    # 7 GND, 8 VIN-A 22.0, 9 GND, 10 FB 23.0 ; EP 11
    def buck(U, L, CIN, COUT1, COUT2, RTOP, RBOT, vout, lx1, lx2, fb):
        q = {n: P(U, n) for n in range(1, 12)}
        ex, ey = q[11]
        # EP GND vias + GND pins into the EP
        for dx, dy in ((-0.5, -0.3), (0.5, -0.3), (-0.3, 0.3), (0.3, 0.3)):
            R.via('GND', (round(ex + dx, 3), round(ey + dy, 3)))
        R.path('GND', B, [q[7], (q[7][0], ey - 0.5)], 0.25)
        R.path('GND', B, [q[9], (q[9][0], ey - 0.5)], 0.25)
        R.path('GND', B, [q[3], (q[3][0], ey + 0.5)], 0.25)
        # CIN / COUT GND pads straight into the EP
        cin2 = P(CIN, 2); co12 = P(COUT1, 2); co22 = P(COUT2, 2)
        R.path('GND', B, [(cin2[0], ey - 0.2), (ex - 1.0, ey - 0.2)], 0.6)
        R.path('GND', B, [(ex + 1.0, ey - 0.2), (co12[0], ey - 0.2)], 0.6)
        # cap GND vias above each GND pad
        for c2 in (cin2, co12):
            R.via('GND', (c2[0], round(c2[1] - 0.9, 3)))
            R.path('GND', B, [c2, (c2[0], round(c2[1] - 0.9, 3))], 0.5)
        for du in (-0.35, 0.35):
            R.via('GND', (round(co22[0] + du, 3), round(co22[1] - 0.9, 3)))
            R.path('GND', B, [(round(co22[0] + du, 3), co22[1]), (round(co22[0] + du, 3), round(co22[1] - 0.9, 3))], 0.4)
        # VIN: pin 5 west, 45 deg into CIN
        cin1 = P(CIN, 1)
        R.path('SYSTEM_SUPPLY', B, [q[5], (q[5][0] - 0.55, q[5][1])], 0.3)
        R.path('SYSTEM_SUPPLY', B, [(q[5][0] - 0.55, q[5][1]), (q[5][0] - 1.3, q[5][1] + 0.75)], 0.5)
        # LX1 / LX2 straight down into the inductor
        R.path(lx1, B, [q[4], (q[4][0], q[4][1] + 0.725), (q[4][0] - 0.25, q[4][1] + 0.975)], W_LX)
        R.path(lx2, B, [q[2], (q[2][0], q[2][1] + 0.725), (q[2][0] + 0.25, q[2][1] + 0.975)], W_LX)
        # VOUT: pin 1 east, 45 deg into COUT1, COUT1 <-> COUT2
        co11 = P(COUT1, 1); co21 = P(COUT2, 1)
        R.path(vout, B, [q[1], (q[1][0] + 0.6, q[1][1])], 0.3)
        R.path(vout, B, [(q[1][0] + 0.6, q[1][1]), (q[1][0] + 1.5, q[1][1] + 0.9)], 0.5)
        R.path(vout, B, [co11, co21], 0.6)
        # FB sense from between the output caps up to the divider top
        su = round((co11[0] + co21[0]) / 2, 3)
        rt1 = P(RTOP, 1); rt2 = P(RTOP, 2); rb1 = P(RBOT, 1)
        R.path(vout, B, [(su, co11[1]), (su, rt1[1] + 0.15)], W_SIG)
        # FB: pin 10 up and 45 deg into the divider midpoint
        R.path(fb, B, [q[10], (q[10][0], rb1[1] + 0.9), (q[10][0] + 0.9, rb1[1])], W_SIG)
        R.path(fb, B, [(q[10][0] + 0.9, rb1[1]), rt2], W_SIG)
        return q

    q8 = buck('U8', 'L5', 'C45', 'C46', 'C47', 'R32', 'R31', 'PANEL_4V', 'power::PANEL_LX1',
              'power::PANEL_LX2', 'power::PANEL_FB')
    # U8 VIN-A (pin 8) -> via -> F branch from the trunk ; EN (pin 6) -> R34 pull-down (MCU exit north)
    v8 = (q8[8][0], SYS_V_U8)
    R.path('SYSTEM_SUPPLY', B, [q8[8], v8], 0.2)
    R.via('SYSTEM_SUPPLY', v8)
    R.path('SYSTEM_SUPPLY', F, [(SYS_U, SYS_V_U8), v8], 0.3)
    r341 = P('R34', 1)
    R.path('PANEL_ENABLE', B, [q8[6], (q8[6][0], r341[1])], W_SIG)
    # U8 input: two vias from the trunk into C45.1
    c451 = P('C45', 1)
    for dv in (-0.4, 0.4):
        vv = (SYS_U, round(c451[1] + dv, 3))
        R.via('SYSTEM_SUPPLY', vv)
        R.path('SYSTEM_SUPPLY', B, [vv, (c451[0] - 0.4, vv[1])], 0.6)

    # ================================================================ U7: IO_3V3 buck-boost (B)
    q7 = buck('U7', 'L4', 'C42', 'C43', 'C44', 'R42', 'R43', 'IO_3V3', 'power::IO_LX1',
              'power::IO_LX2', 'power::IO_FB')
    # U7 EN (pin 6) + VIN-A (pin 8) -> one via north of pin 7 -> F branch from the trunk
    v7 = (q7[7][0], SYS_V_U7)
    R.path('SYSTEM_SUPPLY', B, [q7[6], (q7[6][0], v7[1] + 0.5), v7], 0.2)
    R.path('SYSTEM_SUPPLY', B, [q7[8], (q7[8][0], v7[1] + 0.5), v7], 0.2)
    R.via('SYSTEM_SUPPLY', v7)
    R.path('SYSTEM_SUPPLY', F, [(SYS_U, SYS_V_U7), v7], 0.3)
    # U7 input: two vias south of C42.1, fed by an F branch off the trunk corner
    c421 = P('C42', 1)
    for du in (-0.45, 0.45):
        vv = (round(c421[0] + du, 3), SYS_V_C42)
        R.via('SYSTEM_SUPPLY', vv)
        R.path('SYSTEM_SUPPLY', B, [vv, (vv[0], c421[1] + 0.3)], 0.4)
    R.path('SYSTEM_SUPPLY', F, [(SYS_U, SYS_V_C42), (round(c421[0] - 0.45, 3), SYS_V_C42)], W_CELL)

    # ================================================================ U10 load switch (B): IO_3V3 -> PANEL_LOGIC
    u = {n: P('U10', n) for n in range(1, 7)}
    c501 = P('C50', 1); r352 = P('R35', 2); r351 = P('R35', 1); r361 = P('R36', 1)
    R.path('power::VCI_SLEW', B, [u[4], (u[4][0], c501[1])], 0.2)
    R.path('power::VCI_DISCHARGE', B, [u[5], (u[5][0], r352[1])], 0.2)
    R.path('PANEL_VCI_ENABLE', B, [u[3], r361], W_SIG)
    c491 = P('C49', 1); c492 = P('C49', 2)
    R.path('GND', B, [u[2], (u[2][0], c492[1] - 0.2)], 0.3)
    R.via('GND', (u[2][0], 45.2))
    # IO_3V3 into the switch: C44 -> south of U10 -> C49.1 <-> U10.1 ; via to F east of C49
    c441 = P('C44', 1)
    R.path('IO_3V3', B, [c441, (21.6, c441[1]), (23.55, 46.912), (26.85, 46.912), (c491[0], 46.437)], 0.4)
    R.path('IO_3V3', B, [u[1], (u[1][0], 45.2), (c491[0], 45.575)], 0.4)
    vi1 = (28.4, c491[1])
    R.path('IO_3V3', B, [c491, vi1], 0.4)
    R.via('IO_3V3', vi1)

    # PANEL_LOGIC: U10.6 -> R35.1 -> east under U3's west half -> C21.1 branch -> U3.9 / U3.11
    c211 = P('C21', 1); p9_ = P('U3', 9); p11_ = P('U3', 11)
    pl_v = r351[1]
    R.path('PANEL_LOGIC', B, [u[6], (u[6][0], pl_v), (34.8, pl_v), (34.8, 39.3), (p9_[0], 39.0), p9_], 0.3)
    R.path('PANEL_LOGIC', B, [(34.8, 39.3), (p11_[0], 39.0), p11_], 0.3)
    R.path('PANEL_LOGIC', B, [(c211[0], pl_v), c211], 0.3)
    c212 = P('C21', 2)
    R.via('GND', (27.65, c212[1]))
    R.path('GND', B, [c212, (27.65, c212[1])], 0.4)

    # ================================================================ PANEL_4V to U3 (B -> F -> B)
    c471 = P('C47', 1); c181 = P('C18', 1); c191 = P('C19', 1); u310 = P('U3', 10)
    va = (28.7, c471[1]); vb = (35.6, 32.5)
    R.path('PANEL_4V', B, [c471, va], 0.5)
    R.via('PANEL_4V', va)
    R.path('PANEL_4V', F, [va, (va[0], 31.9), (va[0] + 0.7, 31.2), (vb[0] - 0.7, 31.2), (vb[0], 31.9), vb], 0.5)
    R.via('PANEL_4V', vb)
    R.path('PANEL_4V', B, [vb, (vb[0], c181[1] - 0.25)], 0.4)
    R.path('PANEL_4V', B, [(u310[0], c191[1]), (c181[0], c191[1])], 0.4)
    R.path('PANEL_4V', B, [(u310[0], c191[1]), u310], 0.4)

    # ================================================================ IO_3V3 to U3 (F -> B), R48, C20
    r481 = P('R48', 1); r482 = P('R48', 2); c201 = P('C20', 1); u38 = P('U3', 8); u36 = P('U3', 6)
    vi2 = (34.1, 32.3)
    R.path('IO_3V3', F, [vi1, (29.6, vi1[1] - 1.2), (29.6, 36.8), vi2], 0.3)
    R.via('IO_3V3', vi2)
    R.path('IO_3V3', B, [vi2, (vi2[0], 34.75), (u38[0], 34.85), u38], 0.2)
    R.path('IO_3V3', B, [vi2, (r481[0], vi2[1]), r481], 0.2)
    d = r481[0] - c201[0] - 0.03
    R.path('IO_3V3', B, [c201, (round(c201[0] + d, 3), round(c201[1] - d, 3))], 0.2)
    R.path('TP_VCC', B, [r482, (r482[0], 34.75), (u36[0], 34.85), u36], 0.2)
    # cap GND vias above the U3 cap row
    for ref, dv in (('C20', -0.79), ('C19', -0.84), ('C18', -0.95)):
        g = P(ref, 2)
        R.via('GND', (g[0], round(g[1] + dv, 3)))
        R.path('GND', B, [g, (g[0], round(g[1] + dv, 3))], 0.3)

    # ================================================================ IO_3V3 long run to the tongue (F, then B)
    vi3 = (43.6, 52.8)
    R.path('IO_3V3', F, [vi1, (vi1[0], 46.6), (33.5, 51.7), (42.5, 51.7), vi3], 0.4)
    R.via('IO_3V3', vi3)
    R.path('IO_3V3', B, [vi3, (45.0, 54.2), (45.0, 56.2), IO_VIA], 0.4)
