#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Orientation check list for JLC's placement preview (KiCad python), r6 format.

usage: orient_check.py BOARD.kicad_pcb BOM-jlcpcb.csv OUT.csv
One row per fitted polarized part (ICs, connectors, switches, MK1, Y1, AE1): JLC rotation
(top = KiCad angle, bottom = 180 - angle), footprint origin and pin-1 pad in CPL coordinates
(mm, Y up), where pin 1 lies seen from the top, and the origin-to-pad-centre offset.
"""
import csv, math, sys
import pcbnew

MM = pcbnew.ToMM
b = pcbnew.LoadBoard(sys.argv[1])
mpn = {}
for row in csv.DictReader(open(sys.argv[2])):
    for d in row['Designator'].split(','):
        mpn[d.strip()] = row['Design MPN']
DIRS = ['right', 'upper right', 'up', 'upper left', 'left', 'lower left', 'down', 'lower right']
rows = []
for f in b.GetFootprints():
    ref = f.GetReference()
    if f.IsDNP() or ref.startswith(('TP', 'MH')) or not ref.startswith(('U', 'J', 'SW', 'MK', 'Y', 'AE')):
        continue
    pads = [p for p in f.Pads() if p.GetNumber()]
    p1 = next((p for p in pads if p.GetNumber() in ('1', 'A1')), None)
    if p1 is None:
        continue
    cu, cv = MM(f.GetPosition().x), MM(f.GetPosition().y)
    pu, pv = MM(p1.GetPosition().x), MM(p1.GetPosition().y)
    bot = f.GetLayer() == pcbnew.B_Cu
    ang = f.GetOrientationDegrees()
    jrot = round((180 - ang) % 360) if bot else round(ang % 360)
    a = math.degrees(math.atan2(-(pv - cv), pu - cu))          # seen from the top, Y up
    d = DIRS[int(((a + 22.5) % 360) // 45)]
    # centre of the bounding box of all pads (pad shapes, locating holes included)
    bbs = [p.GetBoundingBox() for p in f.Pads()]          # all pads, locating holes included
    x0 = min(MM(q.GetLeft()) for q in bbs); x1 = max(MM(q.GetRight()) for q in bbs)
    y0 = min(MM(q.GetTop()) for q in bbs); y1 = max(MM(q.GetBottom()) for q in bbs)
    off = math.hypot((x0 + x1) / 2 - cu, (y0 + y1) / 2 - cv)
    rows.append([ref, mpn.get(ref, f.GetValue()), 'Bottom' if bot else 'Top', jrot,
                 f'{cu:.3f}', f'{-cv:.3f}', f'{pu:.3f}', f'{-pv:.3f}', d, f'{off:.2f}'])
rows.sort(key=lambda r: (''.join(c for c in r[0] if c.isalpha()), int(''.join(c for c in r[0] if c.isdigit()) or 0)))
with open(sys.argv[3], 'w', newline='') as fh:
    w = csv.writer(fh)
    w.writerow(['Designator', 'Design MPN', 'Layer', 'JLC Rotation', 'Centre X', 'Centre Y', 'Pin 1 X', 'Pin 1 Y',
                'Pin 1 is toward (seen from top)', 'Origin to pad-centre offset mm'])
    w.writerows(rows)
print(len(rows), 'parts in the orientation check list')
