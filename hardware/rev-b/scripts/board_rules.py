#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Board-level Rev B setup after placement (KiCad python): layer types, rule areas, net classes.

usage: board_rules.py IN.kicad_pcb OUT.kicad_pcb

* In2.Cu becomes a 'power' (plane) layer like In1 (both carry the full GND zone).
* Rule areas (replacing any earlier ones with the same names):
    - "ANT clearance: no vias/pours (all layers)"            u>=48, v<=28.6 inside the board
    - "ANT clearance: no copper (In1/In2/B)"                 same polygon, no tracks/pads too
    - "ANT clearance: F tracks only in the feed corridor"    same polygon minus u 51.7-52.7, v 26.3-28.6
    - "FPC lane: no parts (B)"                               u 31.3-41.9, v 38.65-51.6
    - "BATTERY_USB_FACE_COMPONENT_RESERVE" (F, no parts)     kept from r6
    - "J3 plug withdrawal (F, no parts)"                     u 11.65-22.35, v 19.65-23.7
    - B-side case contact areas (no parts on B)
* Net classes: RF50 (0.16 mm, 0.20 mm clearance) for the 50-ohm feed; AudioAnalog (0.15 mm)
  for DAC/DAC_FILTER/AC/AMP_IN*/BYPASS/AUDIO_REF; existing power classes kept; patterns added.
"""
import sys, json, os
import pcbnew

mm = pcbnew.FromMM
IN, OUT = sys.argv[1], sys.argv[2]
b = pcbnew.LoadBoard(IN)
b.SetLayerType(pcbnew.In2_Cu, pcbnew.LT_POWER)

outline = pcbnew.SHAPE_POLY_SET()
b.GetBoardPolygonOutlines(outline, True)


def rect(u0, v0, u1, v1):
    s = pcbnew.SHAPE_POLY_SET()
    s.NewOutline()
    for x, y in ((u0, v0), (u1, v0), (u1, v1), (u0, v1)):
        s.Append(mm(x), mm(y))
    return s


def circle_ring(cx, cy, r0, r1, n=48):
    import math
    s = pcbnew.SHAPE_POLY_SET()
    s.NewOutline()
    for k in range(n):
        t = 2 * math.pi * k / n
        s.Append(mm(cx + r1 * math.cos(t)), mm(cy + r1 * math.sin(t)))
    s.NewHole()
    for k in range(n):
        t = -2 * math.pi * k / n
        s.Append(mm(cx + r0 * math.cos(t)), mm(cy + r0 * math.sin(t)), 0, 0)
    return s


def add_rule(name, poly, layers, tracks=False, vias=False, pads=False, fps=False, fills=False):
    for z in list(b.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName() == name:
            b.Delete(z)
    z = pcbnew.ZONE(b)
    z.SetIsRuleArea(True)
    ls = pcbnew.LSET()
    for l in layers:
        ls.AddLayer(l)
    z.SetLayerSet(ls)
    z.SetZoneName(name)
    z.SetDoNotAllowTracks(tracks); z.SetDoNotAllowVias(vias); z.SetDoNotAllowPads(pads)
    z.SetDoNotAllowFootprints(fps); z.SetDoNotAllowZoneFills(fills)
    o = z.Outline()
    o.RemoveAllContours()
    for i in range(poly.OutlineCount()):
        o.AddOutline(poly.Outline(i))
        for h in range(poly.HoleCount(i)):
            o.AddHole(poly.Hole(i, h), i)
    b.Add(z)
    return z


ALL = (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu)
# drop r6-era rule areas we replace
for z in list(b.Zones()):
    if z.GetIsRuleArea() and z.GetZoneName() in ('J3_USB_FACE_PLUG_WITHDRAWAL_RESERVE',):
        b.Delete(z)

ant = rect(48.0, 10.0, 56.0, 28.6)
ant.BooleanIntersection(outline)
add_rule('ANT clearance: no vias/pours (all layers)', ant, ALL, vias=True, fills=True)
add_rule('ANT clearance: no copper (In1/In2/B)', ant, (pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu), tracks=True, vias=True, pads=True, fills=True)
antf = ant.CloneDropTriangulation() if hasattr(ant, 'CloneDropTriangulation') else pcbnew.SHAPE_POLY_SET(ant)
antf.BooleanSubtract(rect(51.7, 25.8, 52.7, 28.7))
add_rule('ANT clearance: F tracks only in the feed corridor', antf, (pcbnew.F_Cu,), tracks=True, vias=True, fills=True)

add_rule('FPC lane: no parts (B)', rect(31.3, 39.8, 41.9, 51.6), (pcbnew.B_Cu,), fps=True)
add_rule('J3 plug withdrawal (F, no parts)', rect(11.65, 19.65, 22.35, 23.7), (pcbnew.F_Cu,), fps=True, pads=True)

B_CASE = {
    'case: TL pillar block (B)': rect(1.3, 1.3, 7.7, 7.7),
    'case: left button wall (B)': rect(1.3, 11.6, 4.8, 48.7),
    'case: MH2 boss (B)': rect(1.3, 49.1, 5.7, 52.9),
    'case: right rib (B)': rect(51.2, 17.0, 51.8, 54.4),
    'case: wheel-pocket wall (B)': rect(49.4, 17.0, 53.6, 22.25),
    'case: right screw boss (B)': rect(49.9, 49.75, 53.6, 54.4),
    'case: BR pillar block (B)': rect(48.3, 87.3, 54.7, 93.7),
    'case: MH4 boss (B)': rect(32.35, 89.3, 36.1, 93.7),
    'case: sound-post ring (B)': circle_ring(48.0, 70.5, 3.0, 4.0),
}
for name, poly in B_CASE.items():
    poly.BooleanIntersection(outline)
    add_rule(name, poly, (pcbnew.B_Cu,), fps=True)

pcbnew.SaveBoard(OUT, b)

# ---------------------------------------------------------------- net classes in the .kicad_pro
pro = os.path.splitext(OUT)[0] + '.kicad_pro'
d = json.load(open(pro))
ns = d['net_settings']
base = dict(next(c for c in ns['classes'] if c['name'] == 'Default'))
def cls(name, w, clr, prio):
    c = dict(base); c.update(name=name, track_width=w, clearance=clr, priority=prio)
    return c
ns['classes'] = [c for c in ns['classes'] if c['name'] not in ('RF50', 'AudioAnalog', 'McuPower')]
ns['classes'] += [cls('RF50', 0.16, 0.2, 7), cls('AudioAnalog', 0.15, 0.1, 8), cls('McuPower', 0.25, 0.1, 9)]
pats = [p for p in ns.get('netclass_patterns', []) if p['netclass'] not in ('RF50', 'AudioAnalog', 'McuPower')]
for n in ('BUCK_LX', 'BUCK_FB', 'MCU_VSYS', 'VOUT1', 'VOUT2', 'SIP_1V8', 'MCU_LDO2'):
    pats.append({'netclass': 'McuPower', 'pattern': n})
for n in ('BT_RF', 'rf::BT_RF_LINE', 'rf::ANT_MATCH', 'rf::BT_ANT'):
    pats.append({'netclass': 'RF50', 'pattern': n})
for n in ('DAC_P', 'DAC_N', 'audio::DAC_FILTER_P', 'audio::DAC_FILTER_N', 'audio::AC_P', 'audio::AC_N',
          'audio::AMP_INP', 'audio::AMP_INN', 'audio::AMP_BYPASS', 'AUDIO_REF'):
    pats.append({'netclass': 'AudioAnalog', 'pattern': n})
ns['netclass_patterns'] = pats
json.dump(d, open(pro, 'w'), indent=2)
print('saved', OUT, 'and', pro)

# custom DRC rules (KiCad reads <board>.kicad_dru)
dru = os.path.splitext(OUT)[0] + '.kicad_dru'
open(dru, 'w').write("""(version 1)
# RF50 keeps 0.2 mm to other copper (coplanar gap) except between tracks/pads inside U5's
# 0.35 mm pin field; zones are excluded so the GND pours keep the designed 0.2 mm CPWG gap
(rule "RF50 at the U5 pins"
  (condition "(A.NetClass == 'RF50' || B.NetClass == 'RF50') && (A.intersectsCourtyard('U5') || B.intersectsCourtyard('U5')) && A.Type != 'Zone' && B.Type != 'Zone'")
  (constraint clearance (min 0.1mm)))
# other analog audio nets: >= 0.3 mm to non-audio, non-GND copper (pads of their own parts excepted)
(rule "audio analog spacing"
  (condition "A.NetClass == 'AudioAnalog' && B.NetClass == 'Default' && B.Type != 'Pad' && !A.intersectsCourtyard('U5') && !B.intersectsCourtyard('U5')")
  (constraint clearance (min 0.3mm)))
# audio summing nodes: >= 0.5 mm to any non-audio, non-GND copper that is not a pad of U1.
# Placed last so it wins over the 0.3 mm rule above (KiCad applies the last matching rule).
(rule "audio summing nodes"
  (condition "(A.NetName == 'audio::AMP_INP' || A.NetName == 'audio::AMP_INN') && B.NetClass != 'AudioAnalog' && B.NetClass != 'GroundLocal' && B.NetName != 'GND' && !B.memberOfFootprint('U1')")
  (constraint clearance (min 0.5mm)))
""")
print('wrote', dru)
