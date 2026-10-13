#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Grid maze router for the last open connections (KiCad python + numpy).

usage: maze.py IN.kicad_pcb OUT.kicad_pcb --nets N1,N2,... [--grid 0.05] [--w 0.12]
               [--via-cost 25] [--margin 6] [--layers F,B] [--no-via]

For every net, the islands of its copper are found on a raster (F/B/In1/In2 + vias and
through-hole pads); while more than one island is left, the cheapest path from one island to
any other is found with Dijkstra over F/B cells (8-connected moves, vias between layers), then
simplified into 0/45/90-degree segments and added as locked tracks and 0.45/0.2 vias.
Obstacles: every other net's copper on that layer grown by its clearance + half the track width
(0.1 mm; the custom rules: 0.2 to RF50, 0.3 to AudioAnalog, 0.5 to the amplifier summing
nodes), rule areas that forbid tracks or vias, and 0.3 mm from the board edge. Vias also keep
clear of other nets' In1/In2 tracks. Verify the result with DRC.
"""
import argparse, heapq, math, sys
import numpy as np
import pcbnew

ap = argparse.ArgumentParser()
ap.add_argument('inp'); ap.add_argument('out')
ap.add_argument('--nets', required=True)
ap.add_argument('--grid', type=float, default=0.05)
ap.add_argument('--w', type=float, default=0.12)
ap.add_argument('--via-cost', type=float, default=25.0)
ap.add_argument('--margin', type=float, default=6.0)
ap.add_argument('--layers', default='F,B')
ap.add_argument('--no-via', action='store_true')
ap.add_argument('--window', default='')
ap.add_argument('--debug', action='store_true')
ap.add_argument('--src-at', default='')
ap.add_argument('--ripup', action='store_true')
ap.add_argument('--soft-cost', type=float, default=30.0)
ap.add_argument('--rip-limit', type=int, default=3)
ap.add_argument('--soft-nets', default='', help='regexes: only these nets\' unlocked copper may be ripped')
ap.add_argument('--hist-cost', type=float, default=8.0)
ap.add_argument('--audio-aware', action='store_true')
ap.add_argument('--audio-fast-gap', type=float, default=0.8, help='same-layer gap fast nets keep from analog nets')
ap.add_argument('--keepout', default='', help='LAYER:u0:v0:u1:v1;... extra no-track boxes (F/B/In1/In2)')
ap.add_argument('--reroute', action='store_true', help='delete the listed nets\' unlocked copper first')
ap.add_argument('--layer-cost', default='', help='e.g. In2=6: cost multiplier per cell on that layer')
ap.add_argument('--inner-keepout', default='', help='u0:v0:u1:v1;... boxes where inner-layer tracks are not allowed')
a = ap.parse_args()

MM = pcbnew.ToMM
FM = pcbnew.FromMM
G = a.grid
W = a.w
VIA_R, VIA_D = 0.225, 0.2
EDGE_CLR = 0.3
LAYERS = {'F': pcbnew.F_Cu, 'B': pcbnew.B_Cu, 'In1': pcbnew.In1_Cu, 'In2': pcbnew.In2_Cu}
RLAYERS = [LAYERS[l] for l in a.layers.split(',')]
ALL_CU = [pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu]

b = pcbnew.LoadBoard(a.inp)
outline = pcbnew.SHAPE_POLY_SET()
b.GetBoardPolygonOutlines(outline, True)


def netclass(item):
    try:
        return item.GetNetClassName()
    except Exception:
        return 'Default'


import re as _re
S1 = [r"^(audio::)?DAC_[PN]$", r"^(audio::)?DAC_FILTER_[PN]$", r"^(audio::)?AC_[PN]$", r"^(audio::)?AMP_BYPASS$",
      r"^(audio::)?AUDIO_REF$", r"^(audio::)?ADC_REF$"]
S2 = [r"^MCU_LDO1$"]
AF = [r"^FLASH_", r"^PANEL_(CLK|CS|D[0-3])$", r"^USB_", r"^XTAL48_", r"^MIC_PDM_", r"^(audio::)?MIC_CLOCK$", r"^(rf::)?BT_"]
AS = [r"(^|::)I2C_", r"^PANEL_TE$", r"^DEBUG_", r"^TOUCH_", r"^ENC_", r"^PANEL_RESET$"]
_m = lambda pats, n: any(_re.search(p_, n) for p_ in pats)


def audio_clr(obj_net, my_net):
    """(same-layer, adjacent-layer) clearance the audio checker wants, or (None, None)."""
    if _m(S1, obj_net):
        if _m(AF, my_net):
            return AF_SAME, 0.55
        if _m(AS, my_net):
            return 0.3, 0.3
    if _m(S2, obj_net) and _m(AF, my_net):
        return 0.25, None
    return None, None


AF_SAME = 0.8
ADJ = {pcbnew.F_Cu: pcbnew.In1_Cu, pcbnew.In1_Cu: pcbnew.F_Cu, pcbnew.In2_Cu: pcbnew.B_Cu, pcbnew.B_Cu: pcbnew.In2_Cu}


def clearance_to(obj_net, obj_class, my_class):
    if obj_net in ('audio::AMP_INP', 'audio::AMP_INN'):
        return 0.5
    if obj_class == 'AudioAnalog' and my_class == 'Default':
        return 0.3
    if obj_class == 'RF50':
        return 0.2
    return 0.1


def sps_polys(sps):
    out = []
    for i in range(sps.OutlineCount()):
        o = sps.Outline(i)
        out.append(np.array([(MM(o.CPoint(j).x), MM(o.CPoint(j).y)) for j in range(o.PointCount())]))
    return out


def seg_dist(px, py, x0, y0, x1, y1):
    dx, dy = x1 - x0, y1 - y0
    L = dx * dx + dy * dy
    if L == 0:
        return np.hypot(px - x0, py - y0)
    t = np.clip(((px - x0) * dx + (py - y0) * dy) / L, 0, 1)
    return np.hypot(px - (x0 + t * dx), py - (y0 + t * dy))


def poly_inside(px, py, pts):
    inside = np.zeros(px.shape, bool)
    n = len(pts)
    for k in range(n):
        x0, y0 = pts[k]; x1, y1 = pts[(k + 1) % n]
        cond = ((y0 > py) != (y1 > py))
        with np.errstate(divide='ignore', invalid='ignore'):
            xint = (x1 - x0) * (py - y0) / (y1 - y0 + 1e-12) + x0
        inside ^= cond & (px < xint)
    return inside


def poly_dist(px, py, pts):
    d = np.full(px.shape, 1e9)
    n = len(pts)
    for k in range(n):
        x0, y0 = pts[k]; x1, y1 = pts[(k + 1) % n]
        d = np.minimum(d, seg_dist(px, py, x0, y0, x1, y1))
    ins = poly_inside(px, py, pts)
    d[ins] = -d[ins]                       # signed: negative inside
    return d


class Grid:
    def __init__(self, u0, v0, u1, v1):
        self.u0, self.v0 = u0, v0
        self.nx = int(math.ceil((u1 - u0) / G)) + 1
        self.ny = int(math.ceil((v1 - v0) / G)) + 1

    def sub(self, bu0, bv0, bu1, bv1):
        i0 = max(0, int((bu0 - self.u0) / G)); i1 = min(self.nx, int((bu1 - self.u0) / G) + 2)
        j0 = max(0, int((bv0 - self.v0) / G)); j1 = min(self.ny, int((bv1 - self.v0) / G) + 2)
        if i0 >= i1 or j0 >= j1:
            return None
        xs = self.u0 + np.arange(i0, i1) * G
        ys = self.v0 + np.arange(j0, j1) * G
        px, py = np.meshgrid(xs, ys, indexing='ij')
        return (slice(i0, i1), slice(j0, j1), px, py)

    def uv(self, i, j):
        return (round(float(self.u0 + int(i) * G), 4), round(float(self.v0 + int(j) * G), 4))


def items_of_board():
    """Copper shapes: list of (net, class, layer, kind, data)."""
    items = []
    for t in b.GetTracks():
        n = t.GetNetname(); c = netclass(t)
        if t.GetClass() == 'PCB_VIA':
            x, y = MM(t.GetPosition().x), MM(t.GetPosition().y)
            r = MM(t.GetWidth(pcbnew.F_Cu)) / 2
            for L in ALL_CU:
                items.append((n, c, L, 'circle', (x, y, r), t.IsLocked()))
        else:
            items.append((n, c, t.GetLayer(), 'seg', (MM(t.GetStart().x), MM(t.GetStart().y),
                                                      MM(t.GetEnd().x), MM(t.GetEnd().y), MM(t.GetWidth()) / 2), t.IsLocked()))
    for f in b.GetFootprints():
        for p in f.Pads():
            n = p.GetNetname(); c = netclass(p)
            for L in ALL_CU:
                if p.IsOnLayer(L):
                    try:
                        polys = sps_polys(p.GetEffectivePolygon(L))
                    except Exception:
                        continue
                    for pts in polys:
                        items.append((n, c, L, 'poly', pts, True))
            if p.GetDrillSize().x > 0:          # holes: keep vias off them on every layer
                x, y = MM(p.GetPosition().x), MM(p.GetPosition().y)
                r = MM(max(p.GetDrillSize().x, p.GetDrillSize().y)) / 2
                items.append((n, c, 'HOLE', 'circle', (x, y, r), True))
    return items


def rule_areas():
    out = []
    for z in b.Zones():
        if not z.GetIsRuleArea():
            continue
        polys = sps_polys(z.Outline())
        for L in ALL_CU:
            if z.IsOnLayer(L):
                out.append((L, z.GetDoNotAllowTracks(), z.GetDoNotAllowVias(), polys))
    return out


def bbox_of(kind, data, m):
    if kind == 'circle':
        x, y, r = data; return (x - r - m, y - r - m, x + r + m, y + r + m)
    if kind == 'seg':
        x0, y0, x1, y1, h = data; return (min(x0, x1) - h - m, min(y0, y1) - h - m, max(x0, x1) + h + m, max(y0, y1) + h + m)
    pts = data; return (pts[:, 0].min() - m, pts[:, 1].min() - m, pts[:, 0].max() + m, pts[:, 1].max() + m)


def dist_of(kind, data, px, py):
    if kind == 'circle':
        x, y, r = data; return np.hypot(px - x, py - y) - r
    if kind == 'seg':
        x0, y0, x1, y1, h = data; return seg_dist(px, py, x0, y0, x1, y1) - h
    return poly_dist(px, py, data)


def route_net(net, items, areas):
    my = [it for it in items if it[0] == net]
    if not my:
        print(net, 'no copper'); return 0
    mycls = netclass(b.FindNet(net)) if hasattr(b.FindNet(net), 'GetNetClassName') else 'Default'
    try:
        mycls = b.FindNet(net).GetNetClass().GetName()
    except Exception:
        pass
    xs = []; ys = []
    for it in my:
        bb = bbox_of(it[3], it[4], 0)
        xs += [bb[0], bb[2]]; ys += [bb[1], bb[3]]
    if a.window:
        u0, v0, u1, v1 = map(float, a.window.split(','))
    else:
        u0, v0, u1, v1 = min(xs) - a.margin, min(ys) - a.margin, max(xs) + a.margin, max(ys) + a.margin
    bb = outline.BBox()
    u0 = max(u0, MM(bb.GetLeft())); v0 = max(v0, MM(bb.GetTop()))
    u1 = min(u1, MM(bb.GetRight())); v1 = min(v1, MM(bb.GetBottom()))
    u0 = math.floor(u0 / G + 1e-6) * G; v0 = math.floor(v0 / G + 1e-6) * G     # cells on the 0.05 lattice
    g = Grid(u0, v0, u1, v1)
    shape = (g.nx, g.ny)
    # free maps for track centres on each routing layer, and for via centres
    free = {L: np.ones(shape, bool) for L in RLAYERS}
    vfree = np.ones(shape, bool)
    own = {L: np.zeros(shape, bool) for L in ALL_CU}
    # board edge
    px, py = np.meshgrid(g.u0 + np.arange(g.nx) * G, g.v0 + np.arange(g.ny) * G, indexing='ij')
    inside = np.zeros(shape, bool)
    edge_d = np.full(shape, 1e9)
    for pts in sps_polys(outline):
        ins = poly_inside(px, py, pts)
        inside ^= ins
        n = len(pts)
        for k in range(n):
            x0, y0 = pts[k]; x1, y1 = pts[(k + 1) % n]
            edge_d = np.minimum(edge_d, seg_dist(px, py, x0, y0, x1, y1))
    ok_edge_t = inside & (edge_d >= EDGE_CLR + W / 2)
    ok_edge_v = inside & (edge_d >= EDGE_CLR + VIA_R)
    for L in RLAYERS:
        free[L] &= ok_edge_t
    vfree &= ok_edge_v
    soft = {L: np.zeros(shape, np.int16) for L in RLAYERS}
    vsoft = np.zeros(shape, np.int16)
    soft_items = []
    for (n, c, L, kind, data, locked) in items:
        if n == net:
            continue
        clr = clearance_to(n, c, mycls)
        if a.ripup and not locked and n not in ('GND',) and rips.get(n, 0) < a.rip_limit and \
                (not a.soft_nets or any(_re.search(x, n) for x in a.soft_nets.split(','))):
            mt = clr + W / 2; mv = clr + VIA_R
            s_ = g.sub(*bbox_of(kind, data, max(mt, mv) + G))
            if s_ is None:
                continue
            si, sj, qx, qy = s_
            d = dist_of(kind, data, qx, qy)
            if L in soft:
                soft[L][si, sj] += (d < mt)
            vsoft[si, sj] += (d < mv)
            soft_items.append((n, L, kind, data, mt, mv))
            continue
        if L == 'HOLE':
            m = VIA_R + 0.25
            s = g.sub(*bbox_of(kind, data, m))
            if s is None: continue
            si, sj, qx, qy = s
            vfree[si, sj] &= dist_of(kind, data, qx, qy) >= m - 0.0
            for LL in RLAYERS:
                free[LL][si, sj] &= dist_of(kind, data, qx, qy) >= W / 2 + 0.2
            continue
        mt = clr + W / 2; mv = clr + VIA_R
        sa, aa = audio_clr(n, net) if a.audio_aware else (None, None)
        if sa:
            mt = max(mt, sa + W / 2)
            mv = max(mv, sa + VIA_R)
        s = g.sub(*bbox_of(kind, data, max(mt, mv, (aa or 0) + W / 2) + G))
        if s is None:
            continue
        si, sj, qx, qy = s
        d = dist_of(kind, data, qx, qy)
        if L in free:
            free[L][si, sj] &= d >= mt
        if aa and ADJ.get(L) in free:
            free[ADJ[L]][si, sj] &= d >= aa + W / 2
        vfree[si, sj] &= d >= mv
    for (L, notr, nov, polys) in areas:
        for pts in polys:
            s = g.sub(pts[:, 0].min() - 0.5, pts[:, 1].min() - 0.5, pts[:, 0].max() + 0.5, pts[:, 1].max() + 0.5)
            if s is None: continue
            si, sj, qx, qy = s
            d = poly_dist(qx, qy, pts)
            if notr and L in free:
                free[L][si, sj] &= d >= W / 2
            if nov:
                vfree[si, sj] &= d >= VIA_R
    if a.no_via:
        vfree[:] = False
    for box in [x for x in a.keepout.split(';') if x]:
        ln, bu0, bv0, bu1, bv1 = box.split(':')
        L = LAYERS[ln]; bu0, bv0, bu1, bv1 = map(float, (bu0, bv0, bu1, bv1))
        if L in free:
            s_ = g.sub(bu0, bv0, bu1, bv1)
            if s_ is not None:
                si, sj, qx, qy = s_
                free[L][si, sj] &= ~((qx >= bu0) & (qx <= bu1) & (qy >= bv0) & (qy <= bv1))
    for L in RLAYERS:
        if L in (pcbnew.In1_Cu, pcbnew.In2_Cu):
            for box in [x for x in a.inner_keepout.split(';') if x]:
                bu0, bv0, bu1, bv1 = map(float, box.split(':'))
                s_ = g.sub(bu0, bv0, bu1, bv1)
                if s_ is not None:
                    si, sj, qx, qy = s_
                    free[L][si, sj] &= ~((qx >= bu0) & (qx <= bu1) & (qy >= bv0) & (qy <= bv1))
    # own copper raster (cells whose centre lies inside the copper, shrunk a little)
    for (n, c, L, kind, data, _lk) in my:
        if L == 'HOLE':
            continue
        s = g.sub(*bbox_of(kind, data, G))
        if s is None: continue
        si, sj, qx, qy = s
        d = dist_of(kind, data, qx, qy)
        own[L][si, sj] |= d <= (-0.02 if kind == 'seg' else -0.03)
    # islands: label own cells; connect layers where a via / TH pad covers both
    lab = {L: np.zeros(shape, np.int32) for L in ALL_CU}
    nid = 0
    parent = [0]

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]; x = parent[x]
        return x
    for L in ALL_CU:
        cells = np.argwhere(own[L])
        for (i, j) in cells:
            if lab[L][i, j]:
                continue
            nid += 1; parent.append(nid)
            stack = [(i, j)]; lab[L][i, j] = nid
            while stack:
                ci, cj = stack.pop()
                for di in (-1, 0, 1):
                    for dj in (-1, 0, 1):
                        ni, nj = ci + di, cj + dj
                        if 0 <= ni < g.nx and 0 <= nj < g.ny and own[L][ni, nj] and not lab[L][ni, nj]:
                            lab[L][ni, nj] = nid; stack.append((ni, nj))
    # vias / TH pads join layers
    for (n, c, L, kind, data, _lk) in my:
        if kind == 'circle' and L == pcbnew.F_Cu:
            x, y, r = data
            i, j = int(round((x - g.u0) / G)), int(round((y - g.v0) / G))
            if 0 <= i < g.nx and 0 <= j < g.ny:
                ids = [lab[LL][i, j] for LL in ALL_CU if lab[LL][i, j]]
                for k in ids[1:]:
                    parent[find(k)] = find(ids[0])
    for f in b.GetFootprints():
        for p in f.Pads():
            if p.GetNetname() == net and p.GetDrillSize().x > 0:
                x, y = MM(p.GetPosition().x), MM(p.GetPosition().y)
                i, j = int(round((x - g.u0) / G)), int(round((y - g.v0) / G))
                if 0 <= i < g.nx and 0 <= j < g.ny:
                    ids = [lab[LL][i, j] for LL in ALL_CU if lab[LL][i, j]]
                    for k in ids[1:]:
                        parent[find(k)] = find(ids[0])
    comp = {L: np.vectorize(lambda x: find(x) if x else 0)(lab[L]) if lab[L].any() else lab[L] for L in ALL_CU}
    roots = sorted(set(int(x) for L in RLAYERS for x in np.unique(comp[L]) if x))
    roots_all = sorted(set(int(x) for L in ALL_CU for x in np.unique(comp[L]) if x))
    if '--debug' in sys.argv:
        for r in roots_all:
            cs = [(L, np.argwhere(comp[L] == r)) for L in ALL_CU]
            pts = np.vstack([c for L, c in cs if len(c)])
            print('  island', r, 'cells', len(pts), 'bbox', g.uv(pts[:, 0].min(), pts[:, 1].min()), g.uv(pts[:, 0].max(), pts[:, 1].max()))
    if len(roots_all) <= 1:
        print(f'{net}: already one island'); return 0
    # route: source = biggest island, goals = everything else; repeat until joined
    added = 0
    joined = None
    while True:
        roots_all = sorted(set(int(find(x)) for L in ALL_CU for x in np.unique(comp[L]) if x))
        if len(roots_all) <= 1:
            break
        sizes = {r: sum(int((comp[L] == r).sum()) for L in ALL_CU) for r in roots_all}
        src = max(sizes, key=sizes.get) if joined is None else joined
        if a.src_at and joined is None:
            su, sv = map(float, a.src_at.split(','))
            si_, sj_ = int(round((su - g.u0) / G)), int(round((sv - g.v0) / G))
            for L in ALL_CU:
                if comp[L][si_, sj_]:
                    src = find(int(comp[L][si_, sj_])); break
        LIDX = {L: k for k, L in enumerate(RLAYERS)}
        nl = len(RLAYERS)
        dist = np.full((nl,) + shape, np.inf)
        prev = {}
        pq = []
        for L in RLAYERS:
            cells = np.argwhere(comp[L] == src)
            for (i, j) in cells:
                dist[LIDX[L], i, j] = 0
                heapq.heappush(pq, (0.0, LIDX[L], int(i), int(j)))
        goal = None
        D8 = [(1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0), (1, 1, 1.4142), (1, -1, 1.4142), (-1, 1, 1.4142), (-1, -1, 1.4142)]
        fl = [free[L] for L in RLAYERS]
        cl = [comp[L] for L in RLAYERS]
        sl = [soft[L] for L in RLAYERS]
        SC = a.soft_cost
        LCM = [1.0] * nl
        for spec in [x for x in a.layer_cost.split(',') if x]:
            ln, mult = spec.split('=')
            if LAYERS[ln] in LIDX:
                LCM[LIDX[LAYERS[ln]]] = float(mult)
        gi0 = int(round(g.u0 / G)); gj0 = int(round(g.v0 / G))
        hl = np.zeros((nl,) + shape, np.float32)
        for L_, k_ in LIDX.items():
            sub_ = HISTG[HL[L_], gi0:gi0 + g.nx, gj0:gj0 + g.ny]
            hl[k_, :sub_.shape[0], :sub_.shape[1]] = sub_ * a.hist_cost
        while pq:
            dcur, l, i, j = heapq.heappop(pq)
            if dcur > dist[l, i, j]:
                continue
            r = cl[l][i, j]
            if r and find(int(r)) != src:
                goal = (l, i, j); break
            for di, dj, c in D8:
                ni, nj = i + di, j + dj
                if not (0 <= ni < g.nx and 0 <= nj < g.ny):
                    continue
                if not (fl[l][ni, nj] or cl[l][ni, nj]):
                    continue
                if di and dj and not ((fl[l][i + di, j] or cl[l][i + di, j]) and (fl[l][i, j + dj] or cl[l][i, j + dj])):
                    continue
                nd = dcur + c * LCM[l] + (SC if sl[l][ni, nj] else 0) + hl[l, ni, nj]
                if nd < dist[l, ni, nj]:
                    dist[l, ni, nj] = nd; prev[(l, ni, nj)] = (l, i, j)
                    heapq.heappush(pq, (nd, l, ni, nj))
            if nl > 1 and vfree[i, j]:
                for l2 in range(nl):
                    if l2 != l and (fl[l2][i, j] or cl[l2][i, j]):
                        nd = dcur + a.via_cost + (SC * 3 if vsoft[i, j] else 0)
                        if nd < dist[l2, i, j]:
                            dist[l2, i, j] = nd; prev[(l2, i, j)] = (l, i, j)
                            heapq.heappush(pq, (nd, l2, i, j))
        if goal is None:
            print(f'{net}: no path between islands')
            if a.debug:
                write_png(f'/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb/rt/maze-{net.replace(":", "_")}.png',
                          dist, fl, cl, src, g)
            return added
        path = [goal]
        while path[-1] in prev:
            path.append(prev[path[-1]])
        path.reverse()
        # drop leading/trailing cells that are already inside own copper (keep one)
        while len(path) > 1 and cl[path[1][0]][path[1][1], path[1][2]] and find(int(cl[path[1][0]][path[1][1], path[1][2]])) == src and path[1][0] == path[0][0]:
            path.pop(0)
        cnt, samples, vpts = emit(net, path, g, fl, cl)
        added += cnt
        if a.ripup and soft_items:
            pts = np.array(samples) if samples else np.zeros((0, 3))
            vq = np.array(vpts) if vpts else np.zeros((0, 2))
            for (n_, L_, kind_, data_, mt_, mv_) in soft_items:
                if n_ in RIPPED_NOW:
                    continue
                bb_ = bbox_of(kind_, data_, max(mt_, mv_) + 0.1)
                hit = False
                if len(pts):
                    sel = (pts[:, 0] >= bb_[0]) & (pts[:, 0] <= bb_[2]) & (pts[:, 1] >= bb_[1]) & (pts[:, 1] <= bb_[3]) & (pts[:, 2] == L_)
                    if sel.any():
                        q = pts[sel]
                        hit = bool((dist_of(kind_, data_, q[:, 0], q[:, 1]) < mt_ - 0.005).any())
                if not hit and len(vq):
                    sel = (vq[:, 0] >= bb_[0]) & (vq[:, 0] <= bb_[2]) & (vq[:, 1] >= bb_[1]) & (vq[:, 1] <= bb_[3])
                    if sel.any():
                        q = vq[sel]
                        hit = bool((dist_of(kind_, data_, q[:, 0], q[:, 1]) < mv_ - 0.005).any())
                if hit:
                    RIPPED_NOW.add(n_)
                    if len(pts):
                        sel = (pts[:, 2] == L_)
                        q = pts[sel]
                        if len(q):
                            dd = dist_of(kind_, data_, q[:, 0], q[:, 1])
                            for (qu, qv) in q[dd < mt_ + 0.15][:, :2]:
                                gi_, gj_ = int(round(qu / G)), int(round(qv / G))
                                HISTG[HL[L_], max(0, gi_ - 3):gi_ + 4, max(0, gj_ - 3):gj_ + 4] += 1
        gr = find(int(cl[goal[0]][goal[1], goal[2]]))
        parent[gr] = src
        joined = src
        print(f'{net}: joined island ({len(path)} cells, {sum(1 for k in range(1, len(path)) if path[k][0] != path[k-1][0])} vias)')
    return added


def write_png(path, dist, fl, cl, src, g):
    import zlib, struct
    nl = len(fl)
    h, w = g.ny, g.nx * nl + 4 * (nl - 1)
    img = np.full((h, w, 3), 255, np.uint8)
    for l in range(nl):
        x0 = l * (g.nx + 4)
        blk = ~fl[l]
        sub = img[:, x0:x0 + g.nx]
        sub[blk.T] = (90, 90, 90)
        reach = np.isfinite(dist[l]).T
        sub[reach & ~blk.T] = (120, 200, 255)
        own = (cl[l] != 0).T
        sub[own] = (255, 0, 255)
    raw = b''.join(b'\x00' + img[y].tobytes() for y in range(h))
    def chunk(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b'')
    open(path, 'wb').write(png)
    print('  wrote', path, f'(F | B; window origin {g.u0:.2f},{g.v0:.2f}, {G} mm/px)')


def straight_ok(fl_l, cl_l, p, q):
    (i0, j0), (i1, j1) = p, q
    di, dj = i1 - i0, j1 - j0
    if not (di == 0 or dj == 0 or abs(di) == abs(dj)):
        return False
    n = max(abs(di), abs(dj))
    si, sj = (di > 0) - (di < 0), (dj > 0) - (dj < 0)
    for k in range(n + 1):
        i, j = i0 + si * k, j0 + sj * k
        if not (fl_l[i, j] or cl_l[i, j]):
            return False
    return True


def emit(net, path, g, fl, cl):
    n = b.FindNet(net)
    added = 0
    samples = []; vpts = []
    # split by layer
    runs = []
    cur = [path[0]]
    for s in path[1:]:
        if s[0] != cur[-1][0]:
            runs.append(cur); cur = [s]
        else:
            cur.append(s)
    runs.append(cur)
    for k, run in enumerate(runs):
        l = run[0][0]
        pts = [(s[1], s[2]) for s in run]
        simp = [pts[0]]
        i = 0
        while i < len(pts) - 1:
            j = len(pts) - 1
            while j > i + 1 and not straight_ok(fl[l], cl[l], pts[i], pts[j]):
                j -= 1
            simp.append(pts[j]); i = j
        for (p0, p1) in zip(simp[:-1], simp[1:]):
            t = pcbnew.PCB_TRACK(b)
            u0, v0 = g.uv(*p0); u1, v1 = g.uv(*p1)
            t.SetStart(pcbnew.VECTOR2I(FM(u0), FM(v0))); t.SetEnd(pcbnew.VECTOR2I(FM(u1), FM(v1)))
            t.SetWidth(FM(W)); t.SetLayer(RLAYERS[l]); t.SetNet(n); t.SetLocked(not a.ripup)
            b.Add(t); added += 1
            L_len = math.hypot(u1 - u0, v1 - v0); k_n = max(1, int(L_len / 0.02))
            for k_ in range(k_n + 1):
                samples.append((u0 + (u1 - u0) * k_ / k_n, v0 + (v1 - v0) * k_ / k_n, RLAYERS[l]))
        if k < len(runs) - 1:
            vi, vj = run[-1][1], run[-1][2]
            u, vv = g.uv(vi, vj)
            if any(o.GetClass() == 'PCB_VIA' and o.GetNetCode() == n.GetNetCode()
                   and abs(MM(o.GetPosition().x) - u) < 0.02 and abs(MM(o.GetPosition().y) - vv) < 0.02 for o in b.GetTracks()):
                continue
            v = pcbnew.PCB_VIA(b)
            v.SetPosition(pcbnew.VECTOR2I(FM(u), FM(vv)))
            v.SetWidth(FM(2 * VIA_R)); v.SetDrill(FM(VIA_D)); v.SetNet(n); v.SetLocked(not a.ripup)
            v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
            b.Add(v); added += 1
            vpts.append((u, vv))
    return added, samples, vpts


AF_SAME = a.audio_fast_gap
areas = rule_areas()
total = 0
if a.reroute:
    for t in list(b.GetTracks()):
        if t.GetNetname() in a.nets.split(',') and not t.IsLocked():
            b.Delete(t)
RIPPED_NOW = set()
_bb = outline.BBox()
HISTG = np.zeros((4, int(MM(_bb.GetRight()) / G) + 10, int(MM(_bb.GetBottom()) / G) + 10), np.float32)
HL = {pcbnew.F_Cu: 0, pcbnew.B_Cu: 1, pcbnew.In1_Cu: 2, pcbnew.In2_Cu: 3}
queue = [x for x in a.nets.split(',') if x]
rips = {}
if a.ripup:
    # every net in the queue may be ripped again later: unlock any copper it already has
    pass
failed = []
while queue:
    net = queue.pop(0)
    RIPPED_NOW = set()
    items = items_of_board()
    before = total
    total += route_net(net, items, areas)
    if RIPPED_NOW:
        for n_ in sorted(RIPPED_NOW):
            for t in list(b.GetTracks()):
                if t.GetNetname() == n_ and not t.IsLocked():
                    b.Delete(t)
            rips[n_] = rips.get(n_, 0) + 1
            if n_ not in queue:
                queue.append(n_)
        print(f'  ripped for {net}: {", ".join(sorted(RIPPED_NOW))}')
print('rip counts:', rips)
pcbnew.SaveBoard(a.out, b)
print('added', total, 'items ->', a.out)
