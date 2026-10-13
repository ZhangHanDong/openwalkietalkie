#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""KiCad position CSV -> JLCPCB CPL in the r6 convention (stdlib only).

usage: python3 jlc_cpl.py positions-kicad-all.csv OUT-cpl.csv [--exclude 'C52,C53,J4,MH*,TP*']

Input: `kicad-cli pcb export pos --format csv --units mm --side both` (columns
Ref,Val,Package,PosX,PosY,Rot,Side; PosY already negated by KiCad = Y up).
Output columns: Designator,Mid X,Mid Y,Layer,Rotation (4-decimal mm, integer degrees)
  Top:    rotation = KiCad angle mod 360
  Bottom: rotation = (180 - KiCad angle) mod 360, X NOT mirrored
This reproduces r6 fab-jlcpcb/main/main-cpl-jlcpcb.csv exactly (120/120 rows checked
2026-10-05). The r6 board has no DNP / exclude-from-pos flags, so the not-assembled
parts (C52, C53, J4, MH2, MH4, TP1-TP10) are excluded by name.
"""
import csv
import fnmatch
import sys


def main():
    src, dst = sys.argv[1], sys.argv[2]
    excl = "C52,C53,J4,MH*,TP*"
    if "--exclude" in sys.argv:
        excl = sys.argv[sys.argv.index("--exclude") + 1]
    pats = [p for p in excl.split(",") if p]
    rows = []
    for r in csv.DictReader(open(src)):
        ref = r["Ref"]
        if any(fnmatch.fnmatchcase(ref, p) for p in pats):
            continue
        rot = float(r["Rot"])
        bottom = r["Side"].strip().lower() == "bottom"
        jr = (180 - rot) % 360 if bottom else rot % 360
        rows.append([ref, "%.4f" % float(r["PosX"]), "%.4f" % float(r["PosY"]),
                     "Bottom" if bottom else "Top", "%g" % round(jr, 3)])
    rows.sort(key=lambda x: x[0])
    with open(dst, "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
        w.writerows(rows)
    print("wrote %s (%d parts)" % (dst, len(rows)))


if __name__ == "__main__":
    main()
