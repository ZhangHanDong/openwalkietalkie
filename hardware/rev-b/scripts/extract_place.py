#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Dump placement-relevant board data to JSON (run with KiCad's python).

usage: extract_place.py BOARD.kicad_pcb OUT.json
Footprints: ref, value, side, pos, rot, dnp, courtyard polygons (board mm),
pad centres/sizes/nets/layers, 3D model file names. Board outline polygons
(outer + holes) from Edge.Cuts. Zones / rule areas with their flags.
"""
import sys, json, os
import pcbnew

b = pcbnew.LoadBoard(sys.argv[1])
MM = pcbnew.ToMM


def poly_list(shape_poly_set):
    out = []
    for i in range(shape_poly_set.OutlineCount()):
        o = shape_poly_set.Outline(i)
        outer = [(round(MM(o.CPoint(j).x), 4), round(MM(o.CPoint(j).y), 4)) for j in range(o.PointCount())]
        holes = []
        for h in range(shape_poly_set.HoleCount(i)):
            hh = shape_poly_set.Hole(i, h)
            holes.append([(round(MM(hh.CPoint(j).x), 4), round(MM(hh.CPoint(j).y), 4)) for j in range(hh.PointCount())])
        out.append({'outer': outer, 'holes': holes})
    return out


outline = pcbnew.SHAPE_POLY_SET()
b.GetBoardPolygonOutlines(outline, True)
data = {'board': os.path.abspath(sys.argv[1]), 'outline': poly_list(outline), 'footprints': {}, 'zones': []}

for f in b.GetFootprints():
    side = 'B' if f.IsFlipped() else 'F'
    cy = f.GetCourtyard(pcbnew.B_CrtYd if side == 'B' else pcbnew.F_CrtYd)
    if cy.OutlineCount() == 0:
        # fall back to the pad bounding box + 0.25
        bb = f.GetBoundingBox(False)
        x0, y0, x1, y1 = MM(bb.GetLeft()), MM(bb.GetTop()), MM(bb.GetRight()), MM(bb.GetBottom())
        cyl = [{'outer': [(x0, y0), (x1, y0), (x1, y1), (x0, y1)], 'holes': []}]
    else:
        cyl = poly_list(cy)
    pads = []
    for p in f.Pads():
        pos = p.GetPosition()
        bb = p.GetBoundingBox()
        layers = [b.GetLayerName(l) for l in p.GetLayerSet().Seq() if l in (pcbnew.F_Cu, pcbnew.B_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu)]
        pads.append({'num': p.GetNumber(), 'x': round(MM(pos.x), 4), 'y': round(MM(pos.y), 4), 'net': p.GetNetname(),
                     'bbox': [round(MM(bb.GetLeft()), 4), round(MM(bb.GetTop()), 4), round(MM(bb.GetRight()), 4), round(MM(bb.GetBottom()), 4)],
                     'layers': layers, 'tht': p.GetAttribute() in (pcbnew.PAD_ATTRIB_PTH, pcbnew.PAD_ATTRIB_NPTH),
                     'drill': round(MM(p.GetDrillSizeX()), 3)})
    models = [os.path.basename(m.m_Filename) for m in f.Models()]
    data['footprints'][f.GetReference()] = {
        'value': f.GetValue(), 'fpid': str(f.GetFPID().GetUniStringLibId()), 'side': side,
        'x': round(MM(f.GetPosition().x), 4), 'y': round(MM(f.GetPosition().y), 4), 'rot': round(f.GetOrientationDegrees(), 3),
        'dnp': bool(f.IsDNP()), 'locked': bool(f.IsLocked()), 'courtyard': cyl, 'pads': pads, 'models': models}

for z in b.Zones():
    zd = {'name': z.GetZoneName(), 'net': z.GetNetname(), 'rule': bool(z.GetIsRuleArea()),
          'layers': [b.GetLayerName(l) for l in z.GetLayerSet().Seq()], 'outline': poly_list(z.Outline())}
    if z.GetIsRuleArea():
        zd.update(no_tracks=z.GetDoNotAllowTracks(), no_vias=z.GetDoNotAllowVias(), no_fp=z.GetDoNotAllowFootprints(),
                  no_pads=z.GetDoNotAllowPads(), no_fill=z.GetDoNotAllowZoneFills())
    data['zones'].append(zd)

json.dump(data, open(sys.argv[2], 'w'), indent=0)
print('wrote', sys.argv[2], len(data['footprints']), 'footprints')
