# SPDX-License-Identifier: MIT
"""RF feed: chip-side pi (B) -> 50-ohm grounded CPW on B.Cu over In2 GND -> RF via -> F network -> AE1.

CPWG (JLC04101H-3313, 0.0994 mm prepreg): 0.16 mm trace, 0.20 mm gap to the B.Cu GND pour
(RF50 net-class clearance), fence vias 0.70 mm from the centre line at <= 1.0 mm pitch.
Network on F along u 52.2 (Johanson order from the antenna): AE1.1 <- feed stub <- R47 (S2, 0R)
<- ANT_MATCH node (L6 P1 shunt, NF) <- R40 (S1, 0R) <- line; C58 (P2 shunt, NF) on the feed node.
"""
W = 0.16
LINE_V = 30.5
VIA_RF = (50.9, 32.5)
KNEE = (49.4, LINE_V)


def apply(R):
    P = R.pad
    c572 = P('C57', 2)
    d = VIA_RF[0] - KNEE[0]                      # 45-degree jog
    R.path('rf::BT_RF_LINE', 'B', [(c572[0], LINE_V), KNEE, (VIA_RF[0], LINE_V + d), VIA_RF], W)
    R.via('rf::BT_RF_LINE', VIA_RF)
    r401 = P('R40', 1); r402 = P('R40', 2); r471 = P('R47', 1); r472 = P('R47', 2)
    R.path('rf::BT_RF_LINE', 'F', [VIA_RF, (51.6, VIA_RF[1]), (51.9, r401[1]), r401], W)
    R.path('rf::ANT_MATCH', 'F', [r402, r471], 0.3)
    l61 = P('L6', 1)
    R.path('rf::ANT_MATCH', 'F', [l61, (51.75, l61[1]), (r471[0], 30.6)], 0.2)
    c581 = P('C58', 1)
    R.path('rf::BT_ANT', 'F', [c581, (51.75, c581[1]), (r472[0], c581[1])], 0.2)
    ae = P('AE1', 1)
    R.path('rf::BT_ANT', 'F', [r472, (ae[0], 28.6)], W)
    R.path('rf::BT_ANT', 'F', [(ae[0], 28.6), ae], 0.3)
    # GND stitching around the RF via (west / south side only; the F shunts sit north-east)
    for dx, dy in ((-0.75, 0.0), (0.0, 0.75), (-0.75, 0.75)):
        R.via('GND', (round(VIA_RF[0] + dx, 3), round(VIA_RF[1] + dy, 3)))
    # fence along the straight run, both sides
    u = c572[0] + 1.0
    while u <= KNEE[0] - 0.4:
        if not (45.2 <= u <= 46.6):          # gap where the DAC_FILTER pair crosses underneath on F.Cu
            R.via('GND', (round(u, 3), round(LINE_V - 0.7, 3)))
            R.via('GND', (round(u, 3), round(LINE_V + 0.7, 3)))
        u += 0.95
    # fence along the jog, south-west side only
    for t in (0.35, 0.95):
        R.via('GND', (round(KNEE[0] + t - 0.495, 3), round(LINE_V + t + 0.495, 3)))
