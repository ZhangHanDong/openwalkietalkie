# SPDX-License-Identifier: MIT
"""Post-route checks quoted in the README/review: antenna-clearance distances per
net, other-net vias near Y1, inner-layer tracks (and any under Y1/XO/XI copper).
usage: final_checks.py BOARD"""
import pcbnew, sys, math
b = pcbnew.LoadBoard(sys.argv[1])
U0, V1 = 48.0, 28.6                       # antenna clearance: u >= 48.0, v <= 28.6
def d_rect(x, y):
    dx = max(U0 - x, 0.0); dy = max(y - V1, 0.0); return math.hypot(dx, dy)
def seg_d(a, c, w):
    best = 9e9
    for k in range(41):
        t = k/40; x = a[0]+(c[0]-a[0])*t; y = a[1]+(c[1]-a[1])*t
        best = min(best, d_rect(x, y))
    return max(best - w/2, 0.0)
mm = lambda v: v/1e6
near = {}
for t in b.GetTracks():
    n = t.GetNetname()
    if t.GetClass() == 'PCB_VIA':
        p = t.GetPosition(); d = max(d_rect(mm(p.x), mm(p.y)) - mm(t.GetWidth())/2, 0)
    else:
        d = seg_d((mm(t.GetStart().x), mm(t.GetStart().y)), (mm(t.GetEnd().x), mm(t.GetEnd().y)), mm(t.GetWidth()))
    near[n] = min(near.get(n, 9e9), d)
for p in b.GetPads():
    bb = p.GetBoundingBox(); n = p.GetNetname()
    x = min(max(U0, mm(bb.GetX())), mm(bb.GetRight())); y = min(max(mm(bb.GetY()), V1), mm(bb.GetBottom()))
    d = d_rect(x, y) if not (mm(bb.GetRight()) >= U0 and mm(bb.GetY()) <= V1) else 0.0
    near[n + ' (pad ' + p.GetParentFootprint().GetReference() + ')'] = min(near.get(n + ' (pad ' + p.GetParentFootprint().GetReference() + ')', 9e9), d)
print('antenna clearance, nets within 3.2 mm:')
for n, d in sorted(near.items(), key=lambda kv: kv[1]):
    if d < 3.2 and n.split(' (')[0] not in ('GND', ''): print(f'  {d:5.2f}  {n}')
y1 = next(f for f in b.GetFootprints() if f.GetReference() == 'Y1')
pads = [(mm(p.GetPosition().x), mm(p.GetPosition().y), p.GetNetname()) for p in y1.Pads()]
vd = []
for t in b.GetTracks():
    if t.GetClass() != 'PCB_VIA': continue
    p = t.GetPosition(); x, y = mm(p.x), mm(p.y)
    for px, py, pn in pads:
        if t.GetNetname() != pn and t.GetNetname() != 'GND':
            vd.append((math.hypot(x-px, y-py), t.GetNetname(), round(x, 2), round(y, 2)))
vd.sort(); print('nearest non-GND other-net vias to a Y1 pad centre:', vd[:3])
# edge to edge: via annulus to Y1 pad polygon, any via whose net differs from the pad's
ve = {'signal': [], 'GND': []}
for t in b.GetTracks():
    if t.GetClass() != 'PCB_VIA': continue
    p = t.GetPosition()
    for pad in y1.Pads():
        if pad.GetNetname() == t.GetNetname(): continue
        d = mm(pad.GetEffectivePolygon(pcbnew.B_Cu).COutline(0).Distance(p)) - mm(t.GetWidth()) / 2
        ve['GND' if t.GetNetname() == 'GND' else 'signal'].append((round(d, 3), t.GetNetname(), round(mm(p.x), 3), round(mm(p.y), 3), 'Y1.' + pad.GetNumber(), pad.GetNetname()))
for k in ve:
    ve[k].sort(); print(f'nearest {k} via to a Y1 pad, edge to edge (mm, via net, via u, v, pad, pad net):', ve[k][:3])
ybb = y1.GetBoundingBox(False); X0, Y0, X1, Y1 = mm(ybb.GetX())-1.0, mm(ybb.GetY())-1.0, mm(ybb.GetRight())+1.0, mm(ybb.GetBottom())+1.5
L = {}
for t in b.GetTracks():
    if t.GetClass() == 'PCB_VIA' or t.GetLayer() not in (pcbnew.In1_Cu, pcbnew.In2_Cu): continue
    k = (b.GetLayerName(t.GetLayer()), t.GetNetname()); L[k] = L.get(k, 0) + mm(t.GetLength())
    for pt in (t.GetStart(), t.GetEnd()):
        if X0 <= mm(pt.x) <= X1 and Y0 <= mm(pt.y) <= Y1: print('  inner track near Y1:', k)
print('Y1 window', round(X0, 2), round(Y0, 2), round(X1, 2), round(Y1, 2))
print('inner-layer tracks:', {f'{k[0]} {k[1]}': round(v, 1) for k, v in sorted(L.items(), key=lambda kv: -kv[1])})
print('vias', sum(1 for t in b.GetTracks() if t.GetClass() == 'PCB_VIA'))
