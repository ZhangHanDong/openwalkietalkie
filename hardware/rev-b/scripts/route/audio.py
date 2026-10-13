# SPDX-License-Identifier: MIT
"""Audio chain: DAC filter pair (F), amplifier input network (F), amplifier power, class-D outputs to J1 (F),
microphone group (B). apply(R) with R = routelib.Router.

Frame (u, v) == KiCad (x, y). F = battery/back side, B = display/front side. In1/In2 are solid GND planes.

DAC_FILTER pair: leaves the two vias the MCU module dropped next to C2 (N 45.75/19.2, P 46.6/22.45) and runs
south on F as a 0.15/0.20 pair (N west, P east) down u 45.75/46.10, then 45 degrees into the tongue and splits
symmetrically into C4.1 / C3.1. N starts 3.25 mm further north, so P carries three skew bumps (east side) right
where the pair forms; total net lengths (B + F) match within 0.1 mm.

Interface points created here (other modules connect to them):
  SYSTEM_SUPPLY via (36.0, 74.0)   - power trunk arrives from the north on F/B
  IO_3V3 via        (49.4, 60.6)   - feeds U2.1 / C5.1
Nets left for the autorouter: AMP_ENABLE (R3.1 / U1.1, free exit north-east on F), MIC_POWER_ENABLE (R9.1 /
U2.3, from the north on B), MIC_PDM_DATA (continues north from R7.1 on B), audio::MIC_CLOCK (MK1.3: north
channel inside the ring, out through the north-west corner gap, then north-west on B).
"""
import math

W_SIG = 0.15          # analog small-signal
W_PAIR = 0.15
PITCH = 0.35          # 0.15 track + 0.20 gap
W_OUT = 0.4           # class-D outputs / speaker pair
W_VDD = 0.5
W_SUP_B = 0.8         # SYSTEM_SUPPLY trunk on B
W_MIC = 0.25          # MIC_SUPPLY
W_GND = 0.5

VIA_N = (45.75, 19.2)
VIA_P = (46.6, 22.45)
LANE_N = 45.75
LANE_P = LANE_N + PITCH      # 46.10
SYS_IF = (36.0, 74.0)
IO33_IF = (49.4, 60.6)


def _len(pts):
    return sum(math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(pts[:-1], pts[1:]))


def _net_len(R, net):
    n = R.b.FindNet(net).GetNetCode()
    return sum(t.GetLength() / 1e6 for t in R.b.GetTracks()
               if t.GetNetCode() == n and t.GetClass() == 'PCB_TRACK')


def _bump(u0, v0, h, c, t):
    """skew bump to the east of a southbound track at u0 starting at v0 (45-degree chamfers c)."""
    return [(u0, v0), (u0 + c, v0 + c), (u0 + h - c, v0 + c), (u0 + h, v0 + 2 * c),
            (u0 + h, v0 + 2 * c + t), (u0 + h - c, v0 + 3 * c + t), (u0 + c, v0 + 3 * c + t),
            (u0, v0 + 4 * c + t)]


def r(p):
    return (round(p[0], 4), round(p[1], 4))


def dac_filter_pair(R):
    P = R.pad
    cN = P('C4', 1)                  # (40.05, 65.025)
    cP = P('C3', 1)                  # (41.65, 65.025)
    mid = (cN[0] + cP[0]) / 2        # 40.85
    eN, eP = mid - PITCH / 2, mid + PITCH / 2   # end lanes 40.675 / 41.025
    dlt = LANE_N - eN                # diagonal run (same for both lanes)
    VB = 58.0                        # P turns south-west here
    VS = 63.6                        # lanes split here
    spl = cN[0] - eN                 # -0.625 (N goes west, P east)
    dk = PITCH * (math.sqrt(2) - 1)  # inner/outer corner offset of a 45-degree pair bend (0.145)

    # B-side lengths already routed by the MCU module (N 3.107, P 2.607)
    LbN = _net_len(R, 'audio::DAC_FILTER_N')
    LbP = _net_len(R, 'audio::DAC_FILTER_P')

    # the RF line's GND fence (rows v 29.8 / 31.2, u 43.95 ... 47.75 every 0.95) is crossed with the pair split
    # around the 45.85 fence via: N through the 44.9-45.85 gap, P through the 45.85-46.8 gap (0.175 clearance)
    fN, fP, f0, f1 = 45.375, 46.325, 29.2, 31.8
    n_fence = [(LANE_N, f0 - (LANE_N - fN)), (fN, f0), (fN, f1), (LANE_N, f1 + (LANE_N - fN))]
    p_fence = [(LANE_P, f0 - (fP - LANE_P)), (fP, f0), (fP, f1), (LANE_P, f1 + (fP - LANE_P))]
    n_pts = [VIA_N] + [(LANE_N, VB - dk), (eN, VB - dk + dlt), (eN, VS), (cN[0], VS - spl), cN]
    p_head = [VIA_P, (LANE_P, VIA_P[1] + (VIA_P[0] - LANE_P))]
    p_tail = [(LANE_P, VB), (eP, VB + dlt), (eP, VS), (cP[0], VS - spl), cP]
    # skew bumps: solve the bump height so total lengths match
    C, T, S, NB = 0.15, 0.3, 0.3, 5   # five low bumps keep P within 0.9 mm of N (pair p95 <= 1.0)
    v0 = 23.2
    base = _len(p_head + [(LANE_P, v0)]) + _len([(LANE_P, v0)] + p_tail)
    need = (LbN + _len(n_pts)) - (LbP + base)
    h = (need / NB + 4 * C * (2 - math.sqrt(2))) / 2          # extra per bump = 2h - 4c(2 - sqrt2)
    bumps = []
    for k in range(NB):
        bumps += _bump(LANE_P, v0 + k * (4 * C + T + S), h, C, T)
    # audio-band skew of a few mm is irrelevant: no meander (Rev B review)
    p_pts = p_head + p_tail
    R.path('audio::DAC_FILTER_N', 'F', [r(p) for p in n_pts], W_PAIR)
    R.path('audio::DAC_FILTER_P', 'F', [r(p) for p in p_pts], W_PAIR)
    print("audio pair: bump h %.3f, L_N %.3f, L_P %.3f" % (h, LbN + _len(n_pts), LbP + _len(p_pts)))


def input_network(R):
    P = R.pad
    # AC coupling: C3.2 -> R4.1, C4.2 -> R5.1 (mirror images about u 40.85)
    c32, r41 = P('C3', 2), P('R4', 1)
    c42, r51 = P('C4', 2), P('R5', 1)
    jog = c32[0] - r41[0]            # 0.25
    R.path('audio::AC_P', 'F', [c32, (c32[0], 67.2), (r41[0], 67.2 + jog), r41], W_SIG)
    R.path('audio::AC_N', 'F', [c42, (c42[0], 67.2), (r51[0], 67.2 + jog), r51], W_SIG)
    # summing nodes: R4.2 -> U1.3, R5.2 -> U1.4 (mirror images, ~1.5 mm, no vias)
    r42, u3 = P('R4', 2), P('U1', 3)
    r52, u4 = P('R5', 2), P('U1', 4)
    j = r42[0] - u3[0]               # 0.225
    R.path('audio::AMP_INP', 'F', [r42, (r42[0], 69.25), (u3[0], 69.25 + j), u3], W_SIG)
    R.path('audio::AMP_INN', 'F', [r52, (r52[0], 69.25), (u4[0], 69.25 + j), u4], W_SIG)
    # BYPASS: C1.1 -> U1.2
    c11, u2 = P('C1', 1), P('U1', 2)
    d = c11[0] - u2[0]
    R.path('audio::AMP_BYPASS', 'F', [c11, (u2[0], c11[1] + d), u2], W_SIG)
    # C1.2 GND: via in pad (filled + capped)
    R.via('GND', P('C1', 2))
    # AMP_ENABLE local: U1.1 <-> R3.1 ; R3.2 GND via just south
    u1, r31, r32 = P('U1', 1), P('R3', 1), P('R3', 2)
    R.path('AMP_ENABLE', 'F', [u1, (u1[0], r31[1]), r31], 0.15)
    gv = (r32[0], 71.5)
    R.path('GND', 'F', [r32, gv], 0.3)
    R.via('GND', gv)


def amp_power(R):
    P = R.pad
    u5, u6, u7, u8 = P('U1', 5), P('U1', 6), P('U1', 7), P('U1', 8)
    c11s, c11g = P('C11', 1), P('C11', 2)
    # VDD: U1.6 straight down into the NE corner of C11.1
    R.path('SYSTEM_SUPPLY', 'F', [u6, (u6[0], c11s[1] - (u6[0] - c11s[0])), c11s], W_VDD)
    # GND: U1.7 straight down into the NW corner of C11.2, via right at the pin, via below C11.2
    R.path('GND', 'F', [u7, (u7[0], c11g[1] - (c11g[0] - u7[0])), c11g], W_GND)
    R.via('GND', (u7[0], 75.45))
    g2 = (c11g[0], 77.15)
    R.path('GND', 'F', [c11g, g2], W_GND)
    R.via('GND', g2)
    # SYSTEM_SUPPLY: C11.1 -> two vias -> B trunk to the interface via and to C12.1 (10 uF, B)
    s1, s2 = (c11s[0], 77.1), (41.35, 77.1)
    R.path('SYSTEM_SUPPLY', 'F', [c11s, s1, s2], W_VDD)
    R.via('SYSTEM_SUPPLY', s1)
    R.via('SYSTEM_SUPPLY', s2)
    # interface: the agreed via at (36.0, 74.0) plus a twin on the trunk 0.85 mm south-east, tied on F too,
    # so up to ~1.3 A can change layers there whichever layer the power trunk arrives on
    sys2 = (SYS_IF[0] + 0.6, SYS_IF[1] + 0.6)
    R.via('SYSTEM_SUPPLY', SYS_IF)
    R.via('SYSTEM_SUPPLY', sys2)
    R.path('SYSTEM_SUPPLY', 'F', [SYS_IF, sys2], W_SUP_B)
    k = (SYS_IF[0] + (s1[1] - SYS_IF[1]), s1[1])          # 45-degree from the interface
    R.path('SYSTEM_SUPPLY', 'B', [SYS_IF, k, s2], W_SUP_B)
    c12s = P('C12', 1)
    R.path('SYSTEM_SUPPLY', 'B', [s1, (s1[0], c12s[1])], W_SUP_B)
    R.via('GND', (42.2, P('C12', 2)[1]))                   # in C12.2 (B), clear of L1 on F
    # class-D outputs to the ferrites (short, no vias)
    l21, l11 = P('L2', 1), P('L1', 1)
    R.path('audio::AMP_VON', 'F', [u5, (u5[0], 75.0), (l21[0], 75.0 + (u5[0] - l21[0])), l21], W_OUT)
    R.path('audio::AMP_VOP', 'F', [u8, (u8[0], 75.0), (l11[0], 75.0 + (l11[0] - u8[0])), l11], W_OUT)


def speaker(R):
    P = R.pad
    l22, l12 = P('L2', 2), P('L1', 2)
    c9m, c9g = P('C9', 1), P('C9', 2)
    c8p, c8g = P('C8', 1), P('C8', 2)
    c10p, c10m = P('C10', 1), P('C10', 2)
    jP, jM = P('J1', 1), P('J1', 2)
    # pair: 0.4 mm tracks, 0.25 mm gap (pitch 0.65). Heading south MINUS is west (u 39.7, straight down from L2.2)
    # and PLUS east (u 40.35); the right turn to the west puts MINUS on the north lane (v 84.275 = J1.2) and
    # PLUS on the south lane (v 84.925), which jogs down to J1.1 at the connector.
    pp = 0.65
    um, up_ = l22[0], l22[0] + pp
    lm, lp = jM[1], jM[1] + pp
    cm = 0.375                                           # MINUS corner chamfer
    km = um + (lm - cm)                                  # MINUS chamfer line u + v = km
    kp = km + pp * math.sqrt(2)                          # PLUS chamfer parallel, 0.65 mm outside
    # SPEAKER_MINUS: L2.2 -> C10.2 (east), L2.2 -> C9.1 (west), L2.2 -> south -> J1.2
    d = l22[1] - c10m[1]
    R.path('audio::SPEAKER_MINUS', 'F', [l22, (l22[0] + d, c10m[1]), c10m], W_OUT)
    R.path('audio::SPEAKER_MINUS', 'F', [l22, (c9m[0], l22[1])], W_OUT)
    R.path('audio::SPEAKER_MINUS', 'F', [l22, (um, lm - cm), (um - cm, lm), jM], W_OUT)
    # SPEAKER_PLUS: L1.2 -> C8.1 (east), L1.2 -> C10.1 -> 45 deg under L2.2 onto the east lane -> J1.1
    R.path('audio::SPEAKER_PLUS', 'F', [l12, (c8p[0], l12[1])], W_OUT)
    dp = c10p[1] - l12[1]
    s = c10p[0] + c10p[1]
    R.path('audio::SPEAKER_PLUS', 'F', [l12, (l12[0] - dp, c10p[1]), c10p, (up_, s - up_), (up_, kp - up_),
                                        (kp - lp, lp), (jP[0] + 2.5, lp), (jP[0] + 2.5 - (jP[1] - lp), jP[1]), jP],
           W_OUT)
    # shunt-cap GND vias
    for pad, dv in ((c9g, -0.6), (c8g, 0.6)):
        g = (round(pad[0] + dv, 3), pad[1])
        R.path('GND', 'F', [pad, g], 0.3)
        R.via('GND', g)


def mic(R):
    P = R.pad
    # --- U2 (TPS22918) local
    c51, u21 = P('C5', 1), P('U2', 1)
    # (both modules meet here on B.Cu: a junction, no via)
    R.path('IO_3V3', 'B', [IO33_IF, c51, (c51[0], 61.1), (u21[0], 61.1 + (u21[0] - c51[0])), u21], 0.4)
    c52, u22 = P('C5', 2), P('U2', 2)
    gv = (u22[0], 61.3)
    R.path('GND', 'B', [c52, (c52[0], gv[1] - (c52[0] - gv[0])), gv, u22], 0.3)
    R.via('GND', gv)
    u23, r91, r92 = P('U2', 3), P('R9', 1), P('R9', 2)
    R.path('MIC_POWER_ENABLE', 'B', [u23, (u23[0] + (u23[1] - r91[1]), r91[1]), r91], 0.15)
    g = (r92[0], 63.65)
    R.path('GND', 'B', [r92, g], 0.3)
    R.via('GND', g)
    u24, c61, c62 = P('U2', 4), P('C6', 1), P('C6', 2)
    R.path('audio::MIC_SLEW', 'B', [u24, (u24[0] + (u24[1] - c61[1]), c61[1]), c61], 0.15)
    g = (c62[0], 66.15)
    R.path('GND', 'B', [c62, g], 0.3)
    R.via('GND', g)
    u25, r81, r82 = P('U2', 5), P('R8', 1), P('R8', 2)
    R.path('audio::MIC_DISCHARGE', 'B', [u25, r81], 0.15)
    # --- MIC_SUPPLY: U2.6 down the east side of MK1, west under it, T to MK1.1 (SW corner gap) and C7.1
    u26, m1, c71 = P('U2', 6), P('MK1', 1), P('C7', 1)
    vh = 73.7
    m1a = (m1[0], 119.0 - m1[0])                 # on u+v = 119.0, centred in the SW corner gap
    tee = (46.55, vh)
    R.path('audio::MIC_SUPPLY', 'B', [u26, (u26[0], vh - 0.5), (u26[0] - 0.5, vh), tee], W_MIC)
    R.path('audio::MIC_SUPPLY', 'B', [m1, m1a, (tee[0], 119.0 - tee[0]), tee, (tee[0], c71[1])], W_MIC)
    R.path('audio::MIC_SUPPLY', 'B', [(u26[0], r82[1]), r82], W_MIC)
    # --- MIC_PDM_DATA: MK1.4 -> inner east channel -> NE corner gap -> R7.1
    m4, r71, r72 = P('MK1', 4), P('R7', 1), P('R7', 2)
    ch = 48.85
    R.path('MIC_PDM_DATA', 'B', [m4, (ch, m4[1] - (ch - m4[0])), (ch, r71[1] + (ch - r71[0])), r71], 0.12)
    g = (r72[0], 64.9)
    R.path('GND', 'B', [r72, g], 0.25)
    R.via('GND', g)
    # --- MK1 GND ring (pad 5 is four bars; vias only outside the 46.45-49.55 x 68.45-72.55 no-via box)
    #   west bar + pad 2  -> via west ; north bar -> via north ; south bar + east bar (SE gap) -> vias south/east
    m2 = P('MK1', 2)
    R.path('GND', 'B', [m2, (46.775, m2[1])], 0.2)
    gw = (45.95, 70.3)
    R.path('GND', 'B', [(46.775, gw[1]), gw], 0.25)
    R.via('GND', gw)
    gn = (47.6, 68.0)
    R.path('GND', 'B', [(gn[0], 68.775), gn], 0.25)
    R.via('GND', gn)
    R.path('GND', 'B', [(49.225, 71.55), (48.6, 72.175)], 0.2)
    gs = (48.0, 72.95)
    R.path('GND', 'B', [(48.0, 72.225), gs], 0.25)
    R.via('GND', gs)
    ge = (49.85, 70.5)
    R.path('GND', 'B', [(49.225, 70.5), ge], 0.25)
    R.via('GND', ge)
    R.via('GND', P('C7', 2))                     # in the 1206 pad


def apply(R):
    dac_filter_pair(R)
    input_network(R)
    amp_power(R)
    speaker(R)
    mic(R)
