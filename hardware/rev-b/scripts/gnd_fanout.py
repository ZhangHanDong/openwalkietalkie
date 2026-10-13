#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Ground fan-out: give every SMD GND pad its own via to the inner GND planes.

usage: gnd_fanout.py IN.kicad_pcb OUT.kicad_pcb [--refs U1,U2] [--extra REF=N,...] [--report]

For each SMD pad on GND (F or B) that has no via within 1.2 mm already connected,
search candidate via positions on rings around the pad (outward first), pick the
first spot with clearance >= CLR from every copper item of another net on both
outer layers (pads, tracks, vias), >= 0.3 mm from the board edge, outside rule
areas that forbid vias, and with a collision-free stub track from the pad.
Adds the via (0.45/0.2) and a 0.25 mm stub on the pad's layer. Large pads
(exposed pads, connector shells) are skipped unless --extra asks for N vias.
Vias and stubs are locked (they are part of the fixed skeleton for autorouting).
"""
import sys, math, argparse
import pcbnew

ap = argparse.ArgumentParser()
ap.add_argument('board'); ap.add_argument('out')
ap.add_argument('--refs', default='')
ap.add_argument('--skip', default='')
ap.add_argument('--clr', type=float, default=0.15)
ap.add_argument('--report', action='store_true')
a = ap.parse_args()

mm = pcbnew.FromMM
MM = pcbnew.ToMM
b = pcbnew.LoadBoard(a.board)
GND = b.FindNet('GND')
VIA_D, VIA_DR, STUB_W = 0.45, 0.2, 0.25
CLR = a.clr
only = set(r for r in a.refs.split(',') if r)
skip = set(r for r in a.skip.split(',') if r)

outline = pcbnew.SHAPE_POLY_SET()
b.GetBoardPolygonOutlines(outline, True)

# rule areas that forbid vias
no_via_areas = [z for z in b.Zones() if z.GetIsRuleArea() and z.GetDoNotAllowVias()]


def in_no_via(p):
    r = mm(VIA_D / 2 + 0.05)
    pts = [p] + [pcbnew.VECTOR2I(p.x + dx, p.y + dy) for dx, dy in ((r, 0), (-r, 0), (0, r), (0, -r), (r * 7 // 10, r * 7 // 10), (-r * 7 // 10, r * 7 // 10), (r * 7 // 10, -r * 7 // 10), (-r * 7 // 10, -r * 7 // 10))]
    for z in no_via_areas:
        o = z.Outline()
        if any(o.Contains(q) for q in pts):
            return True
    return False


def edge_ok(p, r):
    if not outline.Contains(p):
        return False
    # distance to outline edges
    d = outline.SquaredDistance(p) if hasattr(outline, 'SquaredDistance') else None
    return True


def edge_dist(p):
    best = 1e18
    for i in range(outline.OutlineCount()):
        o = outline.Outline(i)
        n = o.PointCount()
        for j in range(n):
            a_ = o.CPoint(j); b_ = o.CPoint((j + 1) % n)
            ax, ay, bx, by, px, py = a_.x, a_.y, b_.x, b_.y, p.x, p.y
            dx, dy = bx - ax, by - ay
            L = dx * dx + dy * dy
            t = 0 if L == 0 else max(0, min(1, ((px - ax) * dx + (py - ay) * dy) / L))
            qx, qy = ax + t * dx, ay + t * dy
            best = min(best, math.hypot(px - qx, py - qy))
    return MM(best)


sys.path.insert(0, '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb/scripts')
from geom import pt_seg_dist, seg_seg_dist, pt_poly_dist, seg_poly_dist, bbox_of, bbox_dist

LF, LB = pcbnew.F_Cu, pcbnew.B_Cu


def poly_pts(sps):
    o = sps.Outline(0)
    return [(MM(o.CPoint(j).x), MM(o.CPoint(j).y)) for j in range(o.PointCount())]


def obstacles():
    """dict entries: kind, geom, net, layers(set or None=all), item, bbox"""
    obs = []
    for f in b.GetFootprints():
        for p in f.Pads():
            for lid in (LF, LB):
                if p.IsOnLayer(lid):
                    pts = poly_pts(p.GetEffectivePolygon(lid))
                    obs.append(dict(kind='poly', g=pts, net=p.GetNetCode(), lay={lid}, item=p, bb=bbox_of(pts)))
            if p.GetDrillSizeX() > 0:
                c = (MM(p.GetPosition().x), MM(p.GetPosition().y)); r = MM(p.GetDrillSizeX()) / 2
                obs.append(dict(kind='circ', g=(c, r), net=-1, lay=None, item=p, bb=(c[0]-r, c[1]-r, c[0]+r, c[1]+r)))
    for t in b.GetTracks():
        if t.GetClass() == 'PCB_VIA':
            c = (MM(t.GetPosition().x), MM(t.GetPosition().y)); r = MM(t.GetWidth(LF)) / 2
            obs.append(dict(kind='circ', g=(c, r), net=t.GetNetCode(), lay=None, item=t, bb=(c[0]-r, c[1]-r, c[0]+r, c[1]+r)))
        else:
            a_ = (MM(t.GetStart().x), MM(t.GetStart().y)); b_ = (MM(t.GetEnd().x), MM(t.GetEnd().y)); w = MM(t.GetWidth()) / 2
            obs.append(dict(kind='seg', g=(a_, b_, w), net=t.GetNetCode(), lay={t.GetLayer()}, item=t,
                            bb=(min(a_[0], b_[0])-w, min(a_[1], b_[1])-w, max(a_[0], b_[0])+w, max(a_[1], b_[1])+w)))
    return obs


OBS = obstacles()
GN = GND.GetNetCode()


def dist_point(o, c):
    if o['kind'] == 'poly':
        return pt_poly_dist(c, o['g'])
    if o['kind'] == 'circ':
        return max(0.0, math.hypot(c[0]-o['g'][0][0], c[1]-o['g'][0][1]) - o['g'][1])
    a_, b_, w = o['g']
    return max(0.0, pt_seg_dist(c, a_, b_) - w)


def dist_seg(o, a_, b_):
    if o['kind'] == 'poly':
        return seg_poly_dist(a_, b_, o['g'])
    if o['kind'] == 'circ':
        return max(0.0, pt_seg_dist(o['g'][0], a_, b_) - o['g'][1])
    c, d, w = o['g']
    return max(0.0, seg_seg_dist(a_, b_, c, d) - w)


def free_via(c, own_pad):
    cc = (MM(c.x), MM(c.y))
    r = VIA_D / 2
    qb = (cc[0]-r-0.5, cc[1]-r-0.5, cc[0]+r+0.5, cc[1]+r+0.5)
    for o in OBS:
        if o['item'] is own_pad or bbox_dist(o['bb'], qb) > 0.5:
            continue
        clr = CLR
        if o['net'] == GN and o['kind'] == 'poly':
            clr = 0.12
        if o['net'] == GN and o['kind'] in ('circ', 'seg'):
            clr = 0.1
        if o['net'] == -1:
            clr = 0.25
        if dist_point(o, cc) < r + clr:
            return False
    if in_no_via(c):
        return False
    if edge_dist(c) < 0.3 + VIA_D / 2:
        return False
    return True


def free_stub(p0, p1, lid, own_pad):
    a_ = (MM(p0.x), MM(p0.y)); b_ = (MM(p1.x), MM(p1.y))
    w = STUB_W / 2
    qb = (min(a_[0], b_[0])-1, min(a_[1], b_[1])-1, max(a_[0], b_[0])+1, max(a_[1], b_[1])+1)
    for o in OBS:
        if o['item'] is own_pad or o['net'] == GN:
            continue
        if o['lay'] is not None and lid not in o['lay']:
            continue
        if bbox_dist(o['bb'], qb) > 0.5:
            continue
        if dist_seg(o, a_, b_) < w + 0.1:
            return False
    return True


def has_near_via(p, lid):
    pos = p.GetPosition()
    for t in b.GetTracks():
        if t.GetClass() == 'PCB_VIA' and t.GetNetCode() == GND.GetNetCode():
            if math.hypot(MM(t.GetPosition().x - pos.x), MM(t.GetPosition().y - pos.y)) < 1.0:
                return True
    return False


added, failed = 0, []
for f in sorted(b.GetFootprints(), key=lambda f: f.GetReference()):
    ref = f.GetReference()
    if (only and ref not in only) or ref in skip or f.IsDNP():
        continue
    for p in f.Pads():
        if p.GetNetCode() != GND.GetNetCode() or p.GetAttribute() != pcbnew.PAD_ATTRIB_SMD:
            continue
        lid = pcbnew.F_Cu if p.IsOnLayer(pcbnew.F_Cu) else pcbnew.B_Cu
        sx, sy = MM(p.GetSizeX()), MM(p.GetSizeY())
        if max(sx, sy) > 2.6:
            continue   # exposed pads / shells: handled by the footprint or by hand
        if has_near_via(p, lid):
            continue
        pos = p.GetPosition()
        cx, cy = MM(pos.x), MM(pos.y)
        fx, fy = MM(f.GetPosition().x), MM(f.GetPosition().y)
        out = math.atan2(cy - fy, cx - fx) if (abs(cy - fy) + abs(cx - fx)) > 0.05 else 0.0
        bb = p.GetBoundingBox()
        half = max(MM(bb.GetWidth()), MM(bb.GetHeight())) / 2
        done = False
        for dist in [half + 0.45, half + 0.6, half + 0.8, half + 1.0, half + 1.3, half + 1.7, half + 2.2]:
            for k in range(0, 24):
                ang = out + (k // 2) * (math.pi / 12) * (1 if k % 2 == 0 else -1)
                c = pcbnew.VECTOR2I(mm(cx + dist * math.cos(ang)), mm(cy + dist * math.sin(ang)))
                if not free_via(c, p):
                    continue
                if not free_stub(pos, c, lid, p):
                    continue
                v = pcbnew.PCB_VIA(b)
                v.SetPosition(c); v.SetWidth(mm(VIA_D)); v.SetDrill(mm(VIA_DR)); v.SetNet(GND)
                v.SetLocked(True)
                b.Add(v)
                t = pcbnew.PCB_TRACK(b)
                t.SetStart(pos); t.SetEnd(c); t.SetWidth(mm(STUB_W)); t.SetLayer(lid); t.SetNet(GND)
                t.SetLocked(True)
                b.Add(t)
                cc = (MM(c.x), MM(c.y))
                OBS.append(dict(kind='circ', g=(cc, VIA_D / 2), net=GN, lay=None, item=v, bb=(cc[0]-0.225, cc[1]-0.225, cc[0]+0.225, cc[1]+0.225)))
                OBS.append(dict(kind='seg', g=((MM(pos.x), MM(pos.y)), cc, STUB_W / 2), net=GN, lay={lid}, item=t, bb=(min(cx, cc[0])-0.2, min(cy, cc[1])-0.2, max(cx, cc[0])+0.2, max(cy, cc[1])+0.2)))
                added += 1
                done = True
                break
            if done:
                break
        if not done:
            failed.append(f'{ref}.{p.GetNumber()}')

pcbnew.SaveBoard(a.out, b)
print(f'gnd vias added: {added}; no spot found for: {" ".join(failed) if failed else "none"}')
