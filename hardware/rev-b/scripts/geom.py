# SPDX-License-Identifier: MIT
"""Pure-python 2D geometry for KiCad python scripts (mm units).
Obstacles: ('poly', pts, net, layers) / ('seg', (a,b), w, net, layers) / ('circ', c, r, net, layers)."""
import math


def pt_seg_dist(p, a, b):
    ax, ay = a; bx, by = b; px, py = p
    dx, dy = bx - ax, by - ay
    L = dx * dx + dy * dy
    t = 0.0 if L == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / L))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def seg_seg_dist(a, b, c, d):
    if segs_intersect(a, b, c, d):
        return 0.0
    return min(pt_seg_dist(a, c, d), pt_seg_dist(b, c, d), pt_seg_dist(c, a, b), pt_seg_dist(d, a, b))


def _ccw(a, b, c):
    return (c[1] - a[1]) * (b[0] - a[0]) - (b[1] - a[1]) * (c[0] - a[0])


def segs_intersect(a, b, c, d):
    d1, d2, d3, d4 = _ccw(c, d, a), _ccw(c, d, b), _ccw(a, b, c), _ccw(a, b, d)
    return ((d1 > 0) != (d2 > 0)) and ((d3 > 0) != (d4 > 0))


def pt_in_poly(p, pts):
    x, y = p; inside = False
    n = len(pts)
    for i in range(n):
        x1, y1 = pts[i]; x2, y2 = pts[(i + 1) % n]
        if (y1 > y) != (y2 > y):
            xi = x1 + (y - y1) * (x2 - x1) / (y2 - y1)
            if xi > x:
                inside = not inside
    return inside


def pt_poly_dist(p, pts):
    if pt_in_poly(p, pts):
        return 0.0
    return min(pt_seg_dist(p, pts[i], pts[(i + 1) % len(pts)]) for i in range(len(pts)))


def seg_poly_dist(a, b, pts):
    if pt_in_poly(a, pts) or pt_in_poly(b, pts):
        return 0.0
    return min(seg_seg_dist(a, b, pts[i], pts[(i + 1) % len(pts)]) for i in range(len(pts)))


def bbox_of(pts):
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs), max(ys))


def bbox_dist(b1, b2):
    dx = max(0, max(b1[0], b2[0]) - min(b1[2], b2[2]))
    dy = max(0, max(b1[1], b2[1]) - min(b1[3], b2[3]))
    return math.hypot(dx, dy)
