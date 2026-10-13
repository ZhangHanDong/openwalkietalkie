#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Zoomed placement render (KiCad python + PIL), both sides overlaid.

usage: render_place.py BOARD OUT.png --window u0,v0,u1,v1 [--scale 40] [--nets] [--pins U5]
       [--tracks] [--zones] [--ratsnest]
B pads blue, F pads red/orange, courtyards outlined (B dashed teal, F solid green),
Edge.Cuts black. --pins REF labels that footprint's pad numbers; --nets labels pad nets
for 2-pad parts. View is from the FRONT of the KiCad frame (F side), i.e. NOT mirrored.
"""
import argparse, math
import pcbnew
from PIL import Image, ImageDraw, ImageFont

ap = argparse.ArgumentParser()
ap.add_argument('board'); ap.add_argument('out')
ap.add_argument('--window', default='0,0,56,95')
ap.add_argument('--scale', type=float, default=30)
ap.add_argument('--pins', default='')
ap.add_argument('--nets', action='store_true')
ap.add_argument('--tracks', action='store_true')
ap.add_argument('--zones', action='store_true')
ap.add_argument('--ratsnest', action='store_true')
ap.add_argument('--hl', default='')
a = ap.parse_args()
b = pcbnew.LoadBoard(a.board)
u0, v0, u1, v1 = map(float, a.window.split(','))
S = a.scale
W, H = int((u1 - u0) * S), int((v1 - v0) * S)
img = Image.new('RGB', (W, H), (255, 255, 255))
d = ImageDraw.Draw(img, 'RGBA')
F = lambda s: ImageFont.truetype('/System/Library/Fonts/Supplemental/Arial.ttf', max(8, int(s)))
f_small, f_ref, f_big = F(S * 0.22), F(S * 0.32), F(S * 0.45)
MM = pcbnew.ToMM
P = lambda x, y: ((MM(x) - u0) * S, (MM(y) - v0) * S)
hl = set(n for n in a.hl.split(',') if n)


def poly(sps):
    out = []
    for i in range(sps.OutlineCount()):
        o = sps.Outline(i)
        out.append([P(o.CPoint(j).x, o.CPoint(j).y) for j in range(o.PointCount())])
    return out

# grid
for g in range(int(u0), int(u1) + 1):
    x = (g - u0) * S
    d.line([(x, 0), (x, H)], fill=(235, 235, 235) if g % 5 else (200, 200, 200), width=1)
    if g % 5 == 0:
        d.text((x + 2, 2), str(g), fill=(120, 120, 120), font=f_small)
for g in range(int(v0), int(v1) + 1):
    y = (g - v0) * S
    d.line([(0, y), (W, y)], fill=(235, 235, 235) if g % 5 else (200, 200, 200), width=1)
    if g % 5 == 0:
        d.text((2, y + 2), str(g), fill=(120, 120, 120), font=f_small)

if a.zones:
    for z in b.Zones():
        if z.GetIsRuleArea():
            for pl in poly(z.Outline()):
                d.line(pl + [pl[0]], fill=(200, 0, 200, 160), width=1)
            continue
        for lid, col in ((pcbnew.F_Cu, (220, 60, 60, 25)), (pcbnew.B_Cu, (60, 60, 220, 25))):
            if z.IsOnLayer(lid):
                for pl in poly(z.GetFilledPolysList(lid)):
                    if len(pl) > 2:
                        d.polygon(pl, fill=col)

if a.tracks:
    LC = {pcbnew.F_Cu: (210, 50, 50, 200), pcbnew.B_Cu: (50, 80, 210, 200), pcbnew.In1_Cu: (200, 160, 0, 160), pcbnew.In2_Cu: (0, 160, 60, 160)}
    for t in b.GetTracks():
        if t.GetClass() == 'PCB_VIA':
            continue
        c = LC.get(t.GetLayer(), (100, 100, 100, 200))
        if hl and t.GetNetname() in hl:
            c = (255, 0, 255, 255)
        d.line([P(t.GetStart().x, t.GetStart().y), P(t.GetEnd().x, t.GetEnd().y)], fill=c, width=max(1, int(MM(t.GetWidth()) * S)))
    for t in b.GetTracks():
        if t.GetClass() != 'PCB_VIA':
            continue
        x, y = P(t.GetPosition().x, t.GetPosition().y)
        r = MM(t.GetWidth(pcbnew.F_Cu)) * S / 2
        d.ellipse([x - r, y - r, x + r, y + r], fill=(90, 90, 90, 230) if not (hl and t.GetNetname() in hl) else (255, 0, 255, 255))
        rd = MM(t.GetDrillValue()) * S / 2
        d.ellipse([x - rd, y - rd, x + rd, y + rd], fill=(255, 255, 255, 255))

# pads: B first so F draws on top (semi-transparent)
for side_layer, col in ((pcbnew.B_Cu, (40, 80, 220, 150)), (pcbnew.F_Cu, (230, 90, 40, 150))):
    for f in b.GetFootprints():
        for p in f.Pads():
            if not p.IsOnLayer(side_layer):
                continue
            c = col
            if hl and p.GetNetname() in hl:
                c = (255, 0, 255, 230)
            for pl in poly(p.GetEffectivePolygon(side_layer)):
                if len(pl) > 2:
                    d.polygon(pl, fill=c)

if a.ratsnest:
    b.BuildConnectivity()
    conn = b.GetConnectivity()
    for f in b.GetFootprints():
        for p in f.Pads():
            pass
    # simple ratsnest: MST per net over pad centres (non-GND)
    nets = {}
    for f in b.GetFootprints():
        for p in f.Pads():
            n = p.GetNetname()
            if n and n != 'GND':
                nets.setdefault(n, []).append((MM(p.GetPosition().x), MM(p.GetPosition().y)))
    for n, pts in nets.items():
        if len(pts) < 2:
            continue
        inT = [False] * len(pts); dist = [1e9] * len(pts); par = [-1] * len(pts); dist[0] = 0
        for _ in range(len(pts)):
            u = min((i for i in range(len(pts)) if not inT[i]), key=lambda i: dist[i]); inT[u] = True
            if par[u] >= 0:
                x0, y0 = pts[par[u]]; x1, y1 = pts[u]
                d.line([((x0 - u0) * S, (y0 - v0) * S), ((x1 - u0) * S, (y1 - v0) * S)], fill=(0, 150, 0, 120) if n not in hl else (255, 0, 255, 255), width=1)
            for v in range(len(pts)):
                if not inT[v]:
                    dd = math.hypot(pts[u][0] - pts[v][0], pts[u][1] - pts[v][1])
                    if dd < dist[v]:
                        dist[v] = dd; par[v] = u

# courtyards + refs
for f in b.GetFootprints():
    onb = f.GetLayer() == pcbnew.B_Cu
    cy = f.GetCourtyard(pcbnew.B_CrtYd if onb else pcbnew.F_CrtYd)
    for pl in poly(cy):
        if len(pl) > 1:
            if onb:
                for i in range(len(pl)):
                    x0, y0 = pl[i]; x1, y1 = pl[(i + 1) % len(pl)]
                    n = max(1, int(math.hypot(x1 - x0, y1 - y0) / 6))
                    for k in range(0, n, 2):
                        d.line([(x0 + (x1 - x0) * k / n, y0 + (y1 - y0) * k / n), (x0 + (x1 - x0) * (k + 1) / n, y0 + (y1 - y0) * (k + 1) / n)], fill=(0, 140, 160, 255), width=2)
            else:
                d.line(pl + [pl[0]], fill=(0, 150, 0, 255), width=2)
    x, y = P(f.GetPosition().x, f.GetPosition().y)
    ref = f.GetReference()
    if -20 < x < W + 20 and -20 < y < H + 20:
        d.text((x, y), ref + ('·B' if onb else ''), fill=(0, 0, 0) if onb else (120, 30, 0), font=f_big if ref[0] in 'UJ' else f_ref, anchor='mm')
    if a.nets and len(list(f.Pads())) == 2:
        for p in f.Pads():
            px, py = P(p.GetPosition().x, p.GetPosition().y)
            d.text((px, py + S * 0.25), p.GetNetname().split('::')[-1][:10], fill=(80, 0, 120), font=f_small, anchor='mm')
for ref in [r for r in a.pins.split(',') if r]:
    f = b.FindFootprintByReference(ref)
    for p in f.Pads():
        px, py = P(p.GetPosition().x, p.GetPosition().y)
        d.text((px, py), p.GetNumber(), fill=(0, 0, 0), font=f_small, anchor='mm')

# edges
for dr in b.GetDrawings():
    if dr.GetLayer() != pcbnew.Edge_Cuts:
        continue
    sh = dr.GetShape()
    if sh == pcbnew.SHAPE_T_SEGMENT:
        d.line([P(dr.GetStart().x, dr.GetStart().y), P(dr.GetEnd().x, dr.GetEnd().y)], fill=(0, 0, 0), width=2)
    elif sh == pcbnew.SHAPE_T_ARC:
        c = dr.GetCenter(); r = dr.GetRadius()
        sa = dr.GetArcAngleStart().AsDegrees(); ang = dr.GetArcAngle().AsDegrees()
        n = max(4, int(abs(ang) / 4))
        pts = [P(c.x + r * math.cos(math.radians(sa + ang * k / n)), c.y + r * math.sin(math.radians(sa + ang * k / n))) for k in range(n + 1)]
        d.line(pts, fill=(0, 0, 0), width=2)
    elif sh == pcbnew.SHAPE_T_CIRCLE:
        c = dr.GetCenter(); r = MM(dr.GetRadius()) * S
        x, y = P(c.x, c.y)
        d.ellipse([x - r, y - r, x + r, y + r], outline=(0, 0, 0), width=2)
img.save(a.out)
print('wrote', a.out, W, H)
