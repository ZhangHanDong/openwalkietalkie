#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Remove exact duplicate tracks (same net, layer, width, endpoints in either order) and
duplicate vias (same net and position) (KiCad python).  usage: dedupe_tracks.py IN OUT"""
import sys
import pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
seen = set(); n = 0
for t in list(b.GetTracks()):
    if t.GetClass() == 'PCB_VIA':
        k = ('V', t.GetNetCode(), t.GetPosition().x, t.GetPosition().y)
    else:
        a = (t.GetStart().x, t.GetStart().y); c = (t.GetEnd().x, t.GetEnd().y)
        k = ('T', t.GetNetCode(), t.GetLayer(), t.GetWidth()) + tuple(sorted([a, c]))
    if k in seen:
        b.Delete(t); n += 1
    else:
        seen.add(k)
# segments lying entirely inside another collinear same-net, same-layer segment of >= width
def inside(t, o):
    ax, ay, bx, by = o.GetStart().x, o.GetStart().y, o.GetEnd().x, o.GetEnd().y
    L2 = (bx - ax) ** 2 + (by - ay) ** 2
    if L2 == 0:
        return False
    for P in (t.GetStart(), t.GetEnd()):
        cross = (bx - ax) * (P.y - ay) - (by - ay) * (P.x - ax)
        if abs(cross) / L2 ** 0.5 > 1000:              # > 1 um off the line
            return False
        dot = (P.x - ax) * (bx - ax) + (P.y - ay) * (by - ay)
        if dot < -1000 * L2 ** 0.5 or dot > L2 + 1000 * L2 ** 0.5:
            return False
    return True
segs = [t for t in b.GetTracks() if t.GetClass() != 'PCB_VIA']
gone = set()
for t in segs:
    for o in segs:
        if o is t or id(o) in gone or t.GetNetCode() != o.GetNetCode() or t.GetLayer() != o.GetLayer():
            continue
        if o.GetWidth() >= t.GetWidth() and o.GetLength() > t.GetLength() and inside(t, o):
            gone.add(id(t)); b.Delete(t); n += 1; break
pcbnew.SaveBoard(sys.argv[2], b)
print('removed', n, 'duplicate / contained segments')
