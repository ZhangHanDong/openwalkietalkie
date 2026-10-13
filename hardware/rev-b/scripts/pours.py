#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Outer-layer GND pours + stitching vias (KiCad python).

usage: pours.py IN.kicad_pcb OUT.kicad_pcb [--pitch 2.5] [--no-stitch]
* adds 'F GND pour' / 'B GND pour' zones over the whole outline (net GND, clearance 0.2 mm,
  min width 0.2, thermal reliefs 0.2 gap / 0.25 spokes, islands removed), fills all zones;
* drops GND stitching vias on a grid wherever a 0.45 mm via fits with >= 0.25 mm margin inside
  BOTH outer pours' filled copper (so it ties F, In1, In2 and B together and never touches
  another net), skipping rule areas that forbid vias; refills.
"""
import sys, argparse
import pcbnew

ap = argparse.ArgumentParser()
ap.add_argument('inp'); ap.add_argument('out')
ap.add_argument('--pitch', type=float, default=2.5)
ap.add_argument('--no-stitch', action='store_true')
a = ap.parse_args()
mm = pcbnew.FromMM
MM = pcbnew.ToMM
b = pcbnew.LoadBoard(a.inp)
GND = b.FindNet('GND')

outline = pcbnew.SHAPE_POLY_SET()
b.GetBoardPolygonOutlines(outline, True)

for z in list(b.Zones()):
    if not z.GetIsRuleArea() and z.GetZoneName() in ('F GND pour', 'B GND pour'):
        b.Delete(z)

for lid, name in ((pcbnew.F_Cu, 'F GND pour'), (pcbnew.B_Cu, 'B GND pour')):
    z = pcbnew.ZONE(b)
    z.SetLayer(lid)
    z.SetNet(GND)
    z.SetZoneName(name)
    z.SetLocalClearance(mm(0.2))
    z.SetMinThickness(mm(0.2))
    z.SetThermalReliefGap(mm(0.2))
    z.SetThermalReliefSpokeWidth(mm(0.25))
    z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
    z.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
    z.SetAssignedPriority(0)
    o = z.Outline()
    o.RemoveAllContours()
    o.AddOutline(outline.Outline(0))
    b.Add(z)

filler = pcbnew.ZONE_FILLER(b)
filler.Fill(b.Zones())

added = 0
if not a.no_stitch:
    no_via = [z for z in b.Zones() if z.GetIsRuleArea() and z.GetDoNotAllowVias()]
    pf = [z for z in b.Zones() if z.GetZoneName() == 'F GND pour'][0].GetFilledPolysList(pcbnew.F_Cu)
    pb = [z for z in b.Zones() if z.GetZoneName() == 'B GND pour'][0].GetFilledPolysList(pcbnew.B_Cu)
    pf2 = pcbnew.SHAPE_POLY_SET(pf); pb2 = pcbnew.SHAPE_POLY_SET(pb)
    margin = mm(0.225 + 0.25)
    pf2.Deflate(margin, pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, mm(0.01))
    pb2.Deflate(margin, pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, mm(0.01))
    # existing vias / holes: keep 0.8 mm apart
    holes = [(MM(t.GetPosition().x), MM(t.GetPosition().y)) for t in b.GetTracks() if t.GetClass() == 'PCB_VIA']
    for f in b.GetFootprints():
        for p in f.Pads():
            if p.GetDrillSizeX() > 0:
                holes.append((MM(p.GetPosition().x), MM(p.GetPosition().y)))
    bb = b.GetBoardEdgesBoundingBox()
    u = MM(bb.GetLeft()) + 1.0
    while u < MM(bb.GetRight()) - 1.0:
        v = MM(bb.GetTop()) + 1.0
        while v < MM(bb.GetBottom()) - 1.0:
            p = pcbnew.VECTOR2I(mm(u), mm(v))
            if pf2.Contains(p) and pb2.Contains(p) and not any(z.Outline().Contains(p) for z in no_via):
                if all((u - x) ** 2 + (v - y) ** 2 > 0.8 ** 2 for x, y in holes):
                    via = pcbnew.PCB_VIA(b)
                    via.SetPosition(p); via.SetWidth(mm(0.45)); via.SetDrill(mm(0.2)); via.SetNet(GND)
                    b.Add(via)
                    holes.append((u, v))
                    added += 1
            v += a.pitch
        u += a.pitch
    filler.Fill(b.Zones())

pcbnew.SaveBoard(a.out, b)
print(f'pours added and filled; stitching vias added: {added}')
