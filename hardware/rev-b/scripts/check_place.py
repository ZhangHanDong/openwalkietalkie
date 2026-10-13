#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Rev B placement checker (run with ~/.cohdl-dev-runtime/cam-venv/bin/python).

usage: check_place.py PLACE.json [--md OUT.md] [--json OUT.json] [--png OUT.png]

PLACE.json comes from extract_place.py (KiCad python). Encodes every Rev B
placement constraint: board outline, case contact areas, B-side height zones
under the display, display-cable lane, battery reserve, antenna clearance,
fixed parts (switches centred on caps, connectors, U3, AE1, MK1), decoupling /
proximity rules, flash-vs-audio separation, and reports metrics (ratsnest).
Exit code 0 when there are no ERRORs.
"""
import sys, json, math, argparse
from collections import defaultdict
from shapely.geometry import Polygon, Point, box, LineString, MultiPoint
from shapely.ops import unary_union

S = '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad'
HEIGHTS = json.load(open(S + '/revb/model_heights.json'))

ap = argparse.ArgumentParser()
ap.add_argument('place')
ap.add_argument('--md')
ap.add_argument('--json')
ap.add_argument('--png')
ap.add_argument('--quiet', action='store_true')
ap.add_argument('--refs', default='', help='only report findings that mention one of these refs (comma list)')
a = ap.parse_args()
D = json.load(open(a.place))
FP = D['footprints']

errors, warns, infos = [], [], []
def E(code, msg): errors.append((code, msg))
def W(code, msg): warns.append((code, msg))


def to_poly(pl):
    ps = []
    for p in pl:
        if len(p['outer']) >= 3:
            ps.append(Polygon(p['outer'], [h for h in p['holes'] if len(h) >= 3]).buffer(0))
    return unary_union(ps) if ps else Polygon()


BOARD = to_poly(D['outline'])
CY = {r: to_poly(f['courtyard']) for r, f in FP.items()}


def pad(ref, num):
    for p in FP[ref]['pads']:
        if p['num'] == str(num):
            return p
    raise KeyError(f'{ref}.{num}')


def padpt(ref, num):
    p = pad(ref, num)
    return Point(p['x'], p['y'])


def height(ref):
    f = FP[ref]
    hs = [HEIGHTS.get(m, {}).get('zmax', None) for m in f['models']]
    hs = [h for h in hs if h is not None]
    if hs:
        return max(hs)
    v = f['value'].lower(); fpid = f['fpid'].lower()
    if 'test_point' in fpid or 'mounting' in v:
        return 0.1
    if '0402' in fpid:
        return 0.55
    if '0603' in fpid:
        return 0.95
    if '0805' in fpid:
        return 1.45
    if '1206' in fpid:
        return 1.8
    return 1.0

# ---------------------------------------------------------------- regions (board u,v mm)
R = lambda u0, v0, u1, v1: box(u0, v0, u1, v1)
U3_V = 36.75
U3_MOUTH_V = U3_V + 1.9
REG = {
    'battery_reserve_F': R(7.325, 23.7, 48.925, 44.3),
    'j3_withdrawal_F': R(11.65, 19.65, 22.35, 23.7),
    'screw_keepout_FB': Point(52.2, 52.0).buffer(2.5).union(R(51.1, 50.9, 54.0, 53.1)),
    'antenna_clear': R(48.0, 15.0, 56.0, 28.6).intersection(BOARD),
}
B_CASE_NOPART = {
    'TL pillar block': R(1.3, 1.3, 7.7, 7.7),
    'left button wall': R(1.3, 11.6, 4.8, 48.7),
    'MH2 boss': R(1.3, 49.1, 5.7, 52.9),
    'right rib (u51.25-51.75)': R(51.2, 17.0, 51.8, 54.4),
    'wheel-pocket wall (v17-22.25)': R(49.4, 17.0, 53.6, 22.25),
    'right screw boss (B)': R(49.9, 49.75, 53.6, 54.4),
    'sound-post ring': Point(48, 70.5).buffer(4.0).difference(Point(48, 70.5).buffer(3.0)),
    'BR pillar block': R(48.3, 87.3, 54.7, 93.7),
    'MH4 boss': R(32.35, 89.3, 36.1, 93.7),
    'display cable lane (no parts)': R(31.3, U3_MOUTH_V, 41.9, 51.6),
}
# B-side height limits: list of (name, region, limit). The effective limit at a
# point is the minimum of all regions containing it; outside every listed region
# the B face sees the front wall (4.4 mm).
ring_outer = R(4.755, 3.3, 51.245, 51.09)
ring_inner = R(7.355, 6.38, 48.645, 48.47)
H_ZONES = [
    ('front wall Y14', BOARD, 4.4),
    ('ring band', ring_outer.difference(ring_inner), 3.4),
    ('panel rear', ring_inner, 3.3),
    ('folded flex', R(8.05, 14.75, 43.81, 33.13), 2.9),
    ('flex neck', R(20.35, 6.08, 35.65, 16.85), 2.9),
    ('driver IC strip', R(23.89, 10.03, 32.11, 11.58), 2.5),
    ('flex component zone', R(25.72, 19.68, 43.16, 32.63), 1.8),
    ('module connector', R(29.77, 28.64, 42.58, 32.25), 1.8),
    ('over display cable', R(31.3, 32.25, 41.9, U3_V - 2.4), 2.5),
    ('macro cap 1 flange', R(4.755, 52.09, 27.8, 62.09), 2.0),
    ('macro cap 2 flange', R(28.2, 52.09, 51.245, 62.09), 2.0),
]
H_EXEMPT = {'SW3', 'SW4', 'U3'}   # parts the zone is designed around
F_ENCODER = Polygon([(33.2, 4.8), (52.9, 4.8), (52.9, 21.8), (33.2, 21.8)])

# ---------------------------------------------------------------- fixed placements
FIXED = {
    'SW1': ('F', 3.76, 18.5, -90), 'SW2': ('F', 3.76, 30.5, -90), 'SW6': ('F', 3.76, 42.5, -90),
    'SW5': ('F', 52.24, 35.743, 90),
    'SW3': ('B', 39.7225, 57.09, 0), 'SW4': ('B', 16.2775, 57.09, 0),
    'MK1': ('B', 48.0, 70.5, 0),
    'J5': ('F', 28.0, 4.0, 180), 'J2': ('F', 32.5, 16.8, -90), 'J1': ('F', 33.0, 84.9, 90), 'J3': ('F', 17.0, 16.5, 0),
    'U3': ('B', 36.6, U3_V, 0),
    'AE1': ('F', 52.2, 24.6, 90),
    'MH2': ('F', 3.8, 51.0, 0), 'MH4': ('F', 34.229, 91.2, 0),
}
# ---------------------------------------------------------------- side rules
MUST_B = ['U5', 'U4', 'Y1', 'L3', 'C13', 'C14', 'C15', 'C16', 'C17', 'C22', 'C24', 'C25', 'C26',
          'C29', 'C30', 'C52', 'C57', 'C53', 'R1', 'R2', 'C2', 'R39']
MUST_B.remove('C15')
# C23/C27/C28/C31 (N-face decoupling) may be on either side: on F they sit directly behind
# the MCU's north edge, outside the battery reserve (checked by KEEPOUT-F).
MUST_F = ['AE1', 'R40', 'L6', 'R47', 'C58', 'TP3', 'TP4', 'TP5', 'TP6', 'TP8', 'TP10']

# ---------------------------------------------------------------- proximity rules: (ref, ref_pad or None, target, target_pad, max mm, severity)
# distance = nearest pad-centre distance between the two parts restricted to the
# given pads (None = any pad sharing a net with the other part, else any pad)
PROX = [
    # MCU supply decoupling: cap pad on the pin's net to the pin
    # low-current always-on / LDO outputs: <= 4 mm of 0.2 mm trace is fine; core and buck tighter
    ('C26', None, 'U5', 12, 4.0, 'E'), ('C25', None, 'U5', 13, 4.0, 'E'), ('C24', None, 'U5', 14, 4.0, 'E'),
    ('C30', None, 'U5', 15, 3.0, 'E'), ('C29', None, 'U5', 16, 3.0, 'E'), ('C17', None, 'U5', 17, 3.0, 'E'),
    ('L3', 1, 'U5', 18, 3.5, 'E'), ('L3', 2, 'C17', None, 3.0, 'E'),
    ('C31', None, 'U5', 19, 3.5, 'E'), ('C23', None, 'U5', 22, 3.5, 'E'), ('C27', None, 'U5', 23, 3.5, 'E'),
    ('C28', None, 'U5', 24, 4.0, 'E'), ('C14', None, 'U5', 38, 2.5, 'E'), ('C15', None, 'U5', 39, 2.5, 'E'),
    ('C16', None, 'U5', 52, 2.5, 'E'), ('C13', None, 'U5', 68, 3.0, 'E'),
    ('Y1', 1, 'U5', 55, 4.5, 'E'), ('Y1', 3, 'U5', 54, 4.5, 'E'),
    ('R39', None, 'U5', 66, 4.0, 'W'),
    # RF: chip-side pi at pin 53, antenna-side network at the feed
    ('C57', 1, 'U5', 53, 2.5, 'E'), ('C52', 1, 'C57', 1, 1.6, 'E'), ('C53', 1, 'C57', 2, 1.6, 'E'),
    ('R47', 2, 'AE1', 1, 4.5, 'E'), ('C58', 1, 'AE1', 1, 4.5, 'E'), ('L6', 1, 'R47', 1, 2.0, 'E'), ('R40', 2, 'R47', 1, 2.0, 'E'),
    # flash
    ('C22', None, 'U4', 8, 2.5, 'E'),
    # DAC RC filter at the DAC pins
    ('R1', 1, 'U5', 37, 3.0, 'E'), ('R2', 1, 'U5', 36, 3.0, 'E'), ('C2', 1, 'R1', 2, 3.0, 'E'), ('C2', 2, 'R2', 2, 3.0, 'E'),
    # amplifier: R4/R5 at the input pins, C3/C4 behind them, power and bypass close, ferrites close to the outputs
    ('R4', 2, 'U1', 3, 1.8, 'E'), ('R5', 2, 'U1', 4, 1.8, 'E'), ('C3', 2, 'R4', 1, 2.2, 'E'), ('C4', 2, 'R5', 1, 2.2, 'E'),
    ('C1', 1, 'U1', 2, 2.5, 'E'), ('C11', 1, 'U1', 6, 2.5, 'E'), ('C12', 1, 'U1', 6, 6.0, 'W'),
    ('L1', 1, 'U1', 8, 4.0, 'E'), ('L2', 1, 'U1', 5, 4.0, 'E'),
    ('C8', 1, 'L1', 2, 3.0, 'W'), ('C9', 1, 'L2', 2, 3.0, 'W'), ('C10', None, 'L1', 2, 4.0, 'W'),
    ('R3', 1, 'U1', 1, 6.0, 'W'),
    # microphone
    ('C7', 1, 'MK1', 1, 6.0, 'E'), ('U2', 6, 'MK1', 1, 9.0, 'W'), ('C6', 1, 'U2', 4, 2.5, 'E'), ('R8', 1, 'U2', 5, 3.0, 'E'),
    ('C5', 1, 'U2', 1, 3.0, 'E'), ('R9', 1, 'U2', 3, 4.0, 'W'), ('R7', 1, 'MK1', 4, 8.0, 'W'), ('R6', 1, 'U5', 11, 4.0, 'W'),
    # regulators
    ('L4', 1, 'U7', 4, 2.8, 'E'), ('L4', 2, 'U7', 2, 2.8, 'E'), ('C42', 1, 'U7', 5, 3.5, 'E'), ('C43', 1, 'U7', 1, 3.5, 'E'), ('C44', 1, 'U7', 1, 5.0, 'W'),
    ('R42', 2, 'U7', 10, 3.0, 'E'), ('R43', 1, 'U7', 10, 3.0, 'E'),
    ('L5', 1, 'U8', 4, 2.8, 'E'), ('L5', 2, 'U8', 2, 2.8, 'E'), ('C45', 1, 'U8', 5, 3.5, 'E'), ('C46', 1, 'U8', 1, 3.5, 'E'), ('C47', 1, 'U8', 1, 5.0, 'W'),
    ('R31', 1, 'U8', 10, 3.0, 'E'), ('R32', 2, 'U8', 10, 3.0, 'E'), ('R34', 1, 'U8', 6, 4.0, 'W'),
    ('C49', 1, 'U10', 1, 3.0, 'E'), ('C50', 1, 'U10', 4, 2.5, 'E'), ('R35', 2, 'U10', 5, 2.5, 'E'), ('R36', 1, 'U10', 3, 4.0, 'W'),
    # display connector decoupling
    ('C18', 1, 'U3', 10, 4.0, 'E'), ('C19', 1, 'U3', 10, 3.0, 'E'), ('C21', 1, 'U3', 9, 4.5, 'E'), ('C20', 1, 'U3', 8, 3.0, 'E'), ('R48', 2, 'U3', 6, 3.0, 'E'),
    # charger
    ('C40', 1, 'U6', 2, 3.0, 'E'), ('C41', 1, 'U6', 13, 3.0, 'E'), ('C48', 1, 'U6', 10, 3.0, 'E'),
    ('R37', 1, 'U6', 16, 3.0, 'E'), ('R38', 1, 'U6', 12, 3.0, 'E'), ('R44', 1, 'U6', 1, 8.0, 'W'),
    # USB
    ('U20', 1, 'J5', 'A6', 9.0, 'W'), ('R10', 1, 'J5', 'A5', 7.0, 'W'), ('R11', 1, 'J5', 'B5', 7.0, 'W'),
    # encoder / buttons: series R + C at the MCU end or switch end is fine; keep them close to each other
    ('C38', 1, 'R26', 2, 2.5, 'W'), ('C39', 1, 'R27', 2, 2.5, 'W'),
]

# ---------------------------------------------------------------- checks
def near(ref, rpad, tgt, tpad):
    A = [p for p in FP[ref]['pads'] if (rpad is None or p['num'] == str(rpad))]
    B = [p for p in FP[tgt]['pads'] if (tpad is None or p['num'] == str(tpad))]
    if rpad is None and tpad is not None:
        tn = pad(tgt, tpad)['net']
        A2 = [p for p in A if p['net'] == tn]
        A = A2 or A
    if tpad is None and rpad is not None:
        rn = pad(ref, rpad)['net']
        B2 = [p for p in B if p['net'] == rn]
        B = B2 or B
    return min(math.hypot(p['x'] - q['x'], p['y'] - q['y']) for p in A for q in B)


fitted = [r for r, f in FP.items() if not r.startswith('MH')]

# outline / edge
for r in fitted:
    f = FP[r]
    for p in f['pads']:
        if not p['layers']:
            continue
        pb = box(*p['bbox'])
        if not BOARD.buffer(-0.3 + 1e-6).contains(pb):
            if r in ('J5',) and not BOARD.buffer(0.05).contains(pb):
                continue
            E('EDGE', f'{r}.{p["num"]} pad within 0.3 mm of / outside the board edge')
    cy = CY[r]
    exempt_edge = {'J5', 'SW1', 'SW2', 'SW5', 'SW6', 'AE1', 'J1', 'J3', 'J2'}
    if r not in exempt_edge and not BOARD.buffer(0.05).contains(cy):
        E('CY-EDGE', f'{r} courtyard leaves the board ({cy.difference(BOARD).area:.2f} mm2)')

# courtyard overlaps
items = [(r, FP[r]['side'], CY[r]) for r in fitted]
tht = {r for r in fitted if any(p['tht'] and p['drill'] >= 0.5 for p in FP[r]['pads'])}
for i in range(len(items)):
    for j in range(i + 1, len(items)):
        r1, s1, c1 = items[i]; r2, s2, c2 = items[j]
        if s1 != s2 and r1 not in tht and r2 not in tht:
            continue
        if c1.intersects(c2):
            ar = c1.intersection(c2).area
            if ar > 0.01:
                E('CY-OVERLAP', f'{r1}/{r2} courtyards overlap {ar:.2f} mm2')


# pad clash: copper pads of different footprints on the same layer closer than 0.12 mm
from shapely.strtree import STRtree
for layer in ('F.Cu', 'B.Cu'):
    geoms, owners = [], []
    for r in fitted:
        for p in FP[r]['pads']:
            if layer in p['layers']:
                geoms.append(box(*p['bbox'])); owners.append((r, p['num'], p['net']))
    tree = STRtree(geoms)
    seen = set()
    for i, g in enumerate(geoms):
        for j in tree.query(g.buffer(0.12)):
            j = int(j)
            if j <= i or owners[i][0] == owners[j][0]:
                continue
            if owners[i][2] and owners[i][2] == owners[j][2]:
                continue
            dd = g.distance(geoms[j])
            if dd < 0.12:
                key = (owners[i][0], owners[j][0], layer)
                if key in seen:
                    continue
                seen.add(key)
                E('PAD-CLASH', f'{owners[i][0]}.{owners[i][1]} / {owners[j][0]}.{owners[j][1]} on {layer}: {dd:.2f} mm apart')

# fixed parts
for r, (side, x, y, rot) in FIXED.items():
    f = FP[r]
    if f['side'] != side or abs(f['x'] - x) > 0.002 or abs(f['y'] - y) > 0.002 or abs(((f['rot'] - rot + 180) % 360) - 180) > 0.01:
        E('FIXED', f'{r} must be {side} ({x}, {y}) rot {rot}; is {f["side"]} ({f["x"]}, {f["y"]}) rot {f["rot"]}')

for r in MUST_B:
    if FP[r]['side'] != 'B':
        E('SIDE', f'{r} must be on B (MCU cluster under the display)')
for r in MUST_F:
    if FP[r]['side'] != 'F':
        E('SIDE', f'{r} must be on F')

# any copper pad on F.Cu inside the battery reserve (catches PTH via pads of B parts)
for r in fitted:
    for p in FP[r]['pads']:
        if 'F.Cu' in p['layers'] and box(*p['bbox']).intersects(REG['battery_reserve_F']) and r not in ('J3',):
            E('PAD-BATTERY', f'{r}.{p["num"]} copper on F inside the battery reserve')
            break

# F-side keep-outs
for r in fitted:
    f = FP[r]; cy = CY[r]
    if f['side'] == 'F' or r in tht:
        for name in ('battery_reserve_F', 'j3_withdrawal_F'):
            ar = cy.intersection(REG[name]).area
            if ar > 0.01 and r not in ('J3',):
                E('KEEPOUT-F', f'{r} courtyard in {name} ({ar:.2f} mm2)')
        if cy.intersects(F_ENCODER) and height(r) > 4.4 and r != 'J5':
            E('HEIGHT-F', f'{r} {height(r):.2f} mm tall over the encoder board (max 4.4)')
    if f['side'] == 'B' or r in tht:
        for name, reg in B_CASE_NOPART.items():
            reg = reg if (name == 'display cable lane (no parts)' or (r == 'MK1')) else reg.buffer(0.3)
            ar = cy.intersection(reg).area
            if ar > 0.01:
                if r in ('SW1', 'SW2', 'SW6', 'SW5') and name in ('left button wall', 'right rib (u51.25-51.75)'):
                    continue  # SKTD side switches: only their NPTH pegs reach the B face (r6-proven)
                if r == 'U3' and name == 'display cable lane (no parts)':
                    continue  # the lane starts at U3's own mouth
                E('KEEPOUT-B', f'{r} courtyard in case area "{name}" ({ar:.2f} mm2)')
    ar = cy.intersection(REG['screw_keepout_FB']).area
    if ar > 0.01:
        E('KEEPOUT', f'{r} courtyard in the right-screw keep-out ({ar:.2f} mm2)')
    if r != 'AE1':
        ar = cy.intersection(REG['antenna_clear']).area
        if ar > 0.01:
            E('ANT-CLEAR', f'{r} courtyard in the antenna clearance ({ar:.2f} mm2)')

# B-side heights
for r in fitted:
    f = FP[r]
    if f['side'] != 'B' or r in H_EXEMPT:
        continue
    h = height(r); cy = CY[r]
    lim, zname = 99, ''
    for name, reg, l in H_ZONES:
        if cy.intersects(reg) and cy.intersection(reg).area > 0.02 and l < lim:
            lim, zname = l, name
    if h > lim:
        E('HEIGHT-B', f'{r} is {h:.2f} mm tall, limit {lim} mm in "{zname}"')

# antenna surroundings (soft)
ant = REG['antenna_clear']
for r in fitted:
    if r in ('AE1', 'R40', 'L6', 'R47', 'C58'):
        continue
    d = CY[r].distance(ant)
    metal = r.startswith(('SW', 'J', 'U', 'Y', 'L'))
    if metal and d < 2.0:
        W('ANT-METAL', f'{r} is {d:.2f} mm from the antenna clearance (want >= 2)')

# proximity
for (r, rp, t, tp, dmax, sev) in PROX:
    try:
        d = near(r, rp, t, tp)
    except KeyError as e:
        E('PROX', f'missing pad {e}')
        continue
    if d > dmax:
        (E if sev == 'E' else W)('PROX', f'{r}{"."+str(rp) if rp else ""} -> {t}{"."+str(tp) if tp else ""}: {d:.2f} mm > {dmax}')

# flash vs audio separation
u5c = Point(FP['U5']['x'], FP['U5']['y'])
def cen(refs):
    return MultiPoint([(FP[r]['x'], FP[r]['y']) for r in refs]).centroid
fl = cen(['U4']); au = cen(['R1', 'R2', 'C2'])
v1 = (fl.x - u5c.x, fl.y - u5c.y); v2 = (au.x - u5c.x, au.y - u5c.y)
ang = math.degrees(math.acos(max(-1, min(1, (v1[0]*v2[0] + v1[1]*v2[1]) / (math.hypot(*v1) * math.hypot(*v2) + 1e-9)))))
infos.append(f'flash vs DAC-filter direction from U5: {ang:.0f} deg')
if ang < 60:
    E('AUDIO-FLASH', f'flash (U4) and DAC filter directions from U5 differ by only {ang:.0f} deg (need >= 60)')
flash_zone = unary_union([CY['U4'], CY['C22']] + [padpt('U5', n).buffer(0.6) for n in range(28, 34)])
audio_parts = ['R1', 'R2', 'C2', 'C3', 'C4', 'R4', 'R5', 'C1', 'U1']
for r in audio_parts:
    d = CY[r].distance(unary_union([CY['U4'], CY['C22']]))
    lim = 4.0 if r in ('R1', 'R2', 'C2') else 10.0
    if d < lim:
        E('AUDIO-FLASH', f'{r} is {d:.2f} mm from the flash (need >= {lim})')
aggressors = ['Y1', 'L3', 'L4', 'L5', 'U7', 'U8', 'U6', 'U20']
for r in ['U1', 'R4', 'R5', 'C3', 'C4']:
    for g in aggressors:
        d = CY[r].distance(CY[g])
        if d < 5.0:
            E('AUDIO-AGGR', f'{r} is {d:.2f} mm from {g} (need >= 5)')
d = CY['U1'].distance(CY['AE1'])
if d < 8.0:
    E('AUDIO-ANT', f'U1 is {d:.2f} mm from AE1 (need >= 8)')
for g in ['L3', 'U6', 'L4', 'L5']:
    d = CY['Y1'].distance(CY[g])
    if d < 5.0:
        W('XTAL', f'Y1 is {d:.2f} mm from {g} (SiFli: >= 5 from heat/switching sources)')

# RF feed length estimate
rf = padpt('U5', 53).distance(padpt('C57', 1)) + padpt('C57', 2).distance(padpt('R40', 1)) + padpt('R40', 2).distance(padpt('R47', 1)) + padpt('R47', 2).distance(padpt('AE1', 1))
infos.append(f'RF straight-line chain pin53 -> AE1 feed: {rf:.1f} mm')
if rf > 22:
    E('RF-LEN', f'RF chain {rf:.1f} mm (> 22)')
elif rf > 16:
    W('RF-LEN', f'RF chain {rf:.1f} mm (target <= 16)')

# debug pads pattern: keep r6's 2x3 relative arrangement
# r6 pattern on B; on F it must appear mirrored in v (rows swapped)
TPP = {'TP6': (31.3, 83.0), 'TP4': (31.3, 85.0), 'TP3': (34.15, 83.0), 'TP10': (34.15, 85.0), 'TP5': (35.7, 83.0), 'TP8': (35.7, 85.0)}
ox = FP['TP6']['x'] - 31.3; oy = FP['TP6']['y'] - 83.0
for r, (x, y) in TPP.items():
    if abs(FP[r]['x'] - (x + ox)) > 0.01 or abs(FP[r]['y'] - (y + oy)) > 0.01:
        # mirrored pattern is acceptable only as an exact mirror in u
        W('TP-PATTERN', f'{r} breaks the r6 2x3 debug-pad pattern (jig)')
        break

# ---------------------------------------------------------------- metrics: ratsnest (MST per net)
nets = defaultdict(list)
for r, f in FP.items():
    for p in f['pads']:
        if p['net'] and p['layers']:
            nets[p['net']].append((p['x'], p['y'], r, f['side']))
def mst(pts):
    if len(pts) < 2:
        return 0.0
    n = len(pts); inT = [False]*n; dist = [1e18]*n; dist[0] = 0; tot = 0
    for _ in range(n):
        u = min((i for i in range(n) if not inT[i]), key=lambda i: dist[i]); inT[u] = True; tot += dist[u]
        for v in range(n):
            if not inT[v]:
                dd = math.hypot(pts[u][0]-pts[v][0], pts[u][1]-pts[v][1])
                if dd < dist[v]: dist[v] = dd
    return tot
rat = {}
for n, pts in nets.items():
    if n == 'GND':
        continue
    # collapse pads of the same footprint on the same net to one point each (internal connections)
    uniq = {}
    for x, y, r, s in pts:
        uniq.setdefault(r, (x, y))
    rat[n] = mst(list(uniq.values()))
total = sum(rat.values())
infos.append(f'ratsnest total (non-GND, MST): {total:.0f} mm')
top = sorted(rat.items(), key=lambda kv: -kv[1])[:12]
infos.append('longest nets: ' + ', '.join(f'{k} {v:.0f}' for k, v in top))
sideB = sorted(r for r in fitted if FP[r]['side'] == 'B')
infos.append(f'B side parts ({len(sideB)}): ' + ' '.join(sideB))

if a.refs:
    import re as _re
    want = set(a.refs.split(','))
    keep = lambda m: any(_re.search(r'(?<![A-Z0-9])' + _re.escape(w) + r'(?![0-9])', m) for w in want)
    errors[:] = [e for e in errors if keep(e[1])]
    warns[:] = [w for w in warns if keep(w[1])]
res = {'errors': errors, 'warnings': warns, 'info': infos, 'ratsnest_total': total, 'ratsnest': rat}
if a.json:
    json.dump(res, open(a.json, 'w'), indent=1)
lines = [f'# placement check: {len(errors)} errors, {len(warns)} warnings', '']
lines += [f'- ERROR {c}: {m}' for c, m in errors]
lines += [f'- WARN {c}: {m}' for c, m in warns]
lines += ['', *[f'- {i}' for i in infos]]
txt = '\n'.join(lines)
if a.md:
    open(a.md, 'w').write(txt)
if not a.quiet:
    print(txt)

# ---------------------------------------------------------------- optional overlay PNG
if a.png:
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.patches import Polygon as MPoly
    fig, axs = plt.subplots(1, 2, figsize=(17, 15))
    for ax, side in zip(axs, ('B', 'F')):
        ax.set_title(f'{side} side (board u,v; NOT mirrored)')
        ax.set_aspect('equal'); ax.invert_yaxis(); ax.set_xlim(-1, 57); ax.set_ylim(96, -1)
        def draw(g, **kw):
            gs = getattr(g, 'geoms', [g])
            for gg in gs:
                if gg.is_empty or gg.geom_type != 'Polygon':
                    continue
                ax.add_patch(MPoly(list(gg.exterior.coords), closed=True, **kw))
        draw(BOARD, fill=False, ec='k', lw=1.5)
        if side == 'B':
            for name, reg, l in H_ZONES[1:]:
                draw(reg, fc=(0.2, 0.4, 1.0, 0.06), ec=(0.2, 0.4, 1.0, 0.4), lw=0.5)
                c = reg.representative_point(); ax.text(c.x, c.y, f'{l}', fontsize=6, color='b', alpha=0.6)
            for name, reg in B_CASE_NOPART.items():
                draw(reg, fc=(0.8, 0.2, 0.2, 0.25), ec='none')
        else:
            draw(REG['battery_reserve_F'], fc=(0.8, 0.6, 0.3, 0.2), ec='none')
            draw(REG['j3_withdrawal_F'], fc=(0.8, 0.6, 0.3, 0.2), ec='none')
            draw(F_ENCODER, fc='none', ec=(0.5, 0, 0.5, 0.5), ls='--')
        draw(REG['antenna_clear'], fc=(1, 1, 0, 0.35), ec='none')
        for r in fitted:
            f = FP[r]
            if f['side'] != side and r not in tht:
                continue
            col = 'tab:red' if r.startswith('U') else 'tab:green' if r.startswith(('J', 'SW', 'AE', 'MK', 'Y')) else 'tab:gray'
            draw(CY[r], fc='none', ec=col, lw=0.8)
            for p in f['pads']:
                if p['layers']:
                    x0, y0, x1, y1 = p['bbox']
                    ax.add_patch(MPoly([(x0, y0), (x1, y0), (x1, y1), (x0, y1)], closed=True, fc=(0.9, 0.5, 0.1, 0.6) if side == 'F' else (0.2, 0.3, 0.9, 0.6), ec='none'))
            ax.text(f['x'], f['y'], r, fontsize=5 if not r.startswith('U') else 7, ha='center', va='center', weight='bold' if r[0] in 'UJ' else 'normal')
        ax.grid(alpha=0.2)
    plt.tight_layout(); plt.savefig(a.png, dpi=110)
sys.exit(1 if errors else 0)
