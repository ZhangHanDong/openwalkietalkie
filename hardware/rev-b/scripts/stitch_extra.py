#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Targeted GND stitching / RF fixes from the Rev B review (KiCad python).

usage: stitch_extra.py IN.kicad_pcb OUT.kicad_pcb
Adds candidate GND vias (0.45/0.2): antenna-clearance edges at <= 1.2 mm pitch, CPWG fence
gaps, the NE side of the jog, N/E returns at the RF layer-change via, one via beside each RF
shunt GND pad, stitching beside the DAC pair, and a second via at U1 pin 7. Every candidate is
checked geometrically against other-net copper (0.1 mm + via radius, 0.25 mm to holes) and
via-forbidding rule areas, and dropped if it does not fit. Shunt GND pads get a solid zone
connection; In1 (under F) and In2 (under B) get small fill-keepouts under the RF network pads.
"""
import math, sys
import pcbnew

MM = pcbnew.ToMM; FM = pcbnew.FromMM
b = pcbnew.LoadBoard(sys.argv[1])
GND = b.FindNet('GND')
R_VIA = 0.225


def other_copper():
    items = []
    for t in b.GetTracks():
        if t.GetClass() == 'PCB_VIA':
            items.append((t.GetNetname(), 'c', MM(t.GetPosition().x), MM(t.GetPosition().y), MM(t.GetWidth(pcbnew.F_Cu)) / 2, None))
        else:
            items.append((t.GetNetname(), 's', MM(t.GetStart().x), MM(t.GetStart().y), MM(t.GetEnd().x), MM(t.GetEnd().y), MM(t.GetWidth()) / 2))
    for f in b.GetFootprints():
        for p in f.Pads():
            bb = p.GetBoundingBox()
            items.append((p.GetNetname(), 'p', p, MM(bb.GetLeft()), MM(bb.GetTop()), MM(bb.GetRight()), MM(bb.GetBottom())))
    return items


def seg_d(px, py, x0, y0, x1, y1):
    dx, dy = x1 - x0, y1 - y0; L = dx * dx + dy * dy
    t = 0 if L == 0 else max(0, min(1, ((px - x0) * dx + (py - y0) * dy) / L))
    return math.hypot(px - x0 - t * dx, py - y0 - t * dy)


def clr(net):
    return 0.2 if (net.startswith('rf::') or net.startswith('BT_RF')) else 0.1


def fits(u, v, items, areas):
    # board edge: >= 0.3 mm copper-to-edge
    if min(seg_d(u, v, *e) for e in EDGES) < 0.3 + R_VIA:
        return False
    for (L, nov, poly) in areas:
        if nov and poly.Contains(pcbnew.VECTOR2I(FM(u), FM(v))):
            return False
    if not outline.Contains(pcbnew.VECTOR2I(FM(u), FM(v))):
        return False
    for it in items:
        net = it[0]
        if it[1] == 'c':
            d = math.hypot(u - it[2], v - it[3]) - it[4]
            if net == 'GND':
                if d < 0.25 + R_VIA - it[4]: return False      # keep hole spacing to other GND vias
                continue
            if d < clr(net) + R_VIA: return False
        elif it[1] == 's':
            if net == 'GND': continue
            if seg_d(u, v, *it[2:6]) - it[6] < clr(net) + R_VIA: return False
        else:
            p = it[2]
            if not (it[3] - 0.6 < u < it[5] + 0.6 and it[4] - 0.6 < v < it[6] + 0.6): continue
            if p.GetDrillSize().x > 0:
                if math.hypot(u - MM(p.GetPosition().x), v - MM(p.GetPosition().y)) < MM(p.GetDrillSize().x) / 2 + 0.25 + R_VIA:
                    return False
            if net == 'GND': continue
            for lid in (pcbnew.F_Cu, pcbnew.B_Cu):
                if p.IsOnLayer(lid) and p.HitTest(pcbnew.VECTOR2I(FM(u), FM(v)), FM(clr(net) + R_VIA)):
                    return False
    return True


outline = pcbnew.SHAPE_POLY_SET(); b.GetBoardPolygonOutlines(outline, True)
EDGES = []
for i in range(outline.OutlineCount()):
    o = outline.Outline(i); n = o.PointCount()
    pts = [(MM(o.CPoint(j).x), MM(o.CPoint(j).y)) for j in range(n)]
    EDGES += [(pts[j][0], pts[j][1], pts[(j + 1) % n][0], pts[(j + 1) % n][1]) for j in range(n)]
areas = []
for z in b.Zones():
    if z.GetIsRuleArea():
        areas.append((None, z.GetDoNotAllowVias(), z.Outline()))

cands = []
cands += [(round(48.4 + 1.2 * k, 3), 29.1) for k in range(5)]                 # below the clearance
cands += [(47.5, round(19.8 + 1.2 * k, 3)) for k in range(8)]                 # left of the clearance
cands += [(45.3, 29.8), (45.3, 31.2), (46.7, 29.8), (46.7, 31.2)]             # CPWG fence gaps
cands += [(50.75, 31.05), (50.9, 31.8), (51.6, 32.5)]                         # jog NE side, RF via N/E
for v in [round(31.5 + 2.5 * k, 2) for k in range(11)]:                       # DAC pair stitching
    cands += [(45.15, v), (46.75, v)]
cands += [(42.1, 75.45), (42.2, 75.3), (42.0, 75.6)]                          # U1 pin 7
shunts = []
for ref in ('C52', 'C53', 'L6', 'C58'):
    f = b.FindFootprintByReference(ref)
    for p in f.Pads():
        if p.GetNetname() == 'GND':
            p.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
            pu, pv = MM(p.GetPosition().x), MM(p.GetPosition().y)
            # candidates on a ring 0.55-0.65 mm from the pad centre (edge within ~0.3 mm)
            shunts.append([(round(pu + r * math.cos(a), 3), round(pv + r * math.sin(a), 3))
                           for r in (0.6, 0.7) for a in [k * math.pi / 8 for k in range(16)]])
added = []
items = other_copper()
def add(u, v):
    via = pcbnew.PCB_VIA(b)
    via.SetPosition(pcbnew.VECTOR2I(FM(u), FM(v))); via.SetWidth(FM(0.45)); via.SetDrill(FM(0.2))
    via.SetNet(GND); via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); via.SetLocked(True)
    b.Add(via); items.append(('GND', 'c', u, v, R_VIA, None)); added.append((u, v))
for (u, v) in cands:
    if fits(u, v, items, areas):
        add(u, v)
for ring in shunts:
    for (u, v) in ring:
        if fits(u, v, items, areas):
            add(u, v); break
# plane voids under the RF network pads (pad + 0.2 mm): In1 under F pads, In2 under B pads
nv = 0
for ref, lay in (('R40', pcbnew.In1_Cu), ('L6', pcbnew.In1_Cu), ('R47', pcbnew.In1_Cu), ('C58', pcbnew.In1_Cu),
                 ('C57', pcbnew.In2_Cu), ('C52', pcbnew.In2_Cu), ('C53', pcbnew.In2_Cu)):
    f = b.FindFootprintByReference(ref)
    for p in f.Pads():
        bb = p.GetBoundingBox()
        z = pcbnew.ZONE(b); z.SetIsRuleArea(True)
        ls = pcbnew.LSET(); ls.AddLayer(lay); z.SetLayerSet(ls)
        z.SetZoneName(f'RF pad void {ref}.{p.GetNumber()}')
        z.SetDoNotAllowTracks(False); z.SetDoNotAllowVias(False); z.SetDoNotAllowPads(False)
        z.SetDoNotAllowFootprints(False); z.SetDoNotAllowZoneFills(True)
        o = z.Outline(); o.RemoveAllContours(); o.NewOutline()
        x0, y0, x1, y1 = MM(bb.GetLeft()) - 0.2, MM(bb.GetTop()) - 0.2, MM(bb.GetRight()) + 0.2, MM(bb.GetBottom()) + 0.2
        for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
            o.Append(FM(x), FM(y))
        b.Add(z); nv += 1
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
pcbnew.SaveBoard(sys.argv[2], b)
print(f'added {len(added)} GND vias of {len(cands) + len(shunts)} candidates; {nv} plane voids')
print('  ', added)
