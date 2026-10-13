# SPDX-License-Identifier: MIT
"""JLCPCB BOM + CPL for the flash jig (assembled parts only)."""
import csv
import os
import sys

import pcbnew as P

board = P.LoadBoard(sys.argv[1])
out = sys.argv[2]
rows, cpl = {}, []
for fp in sorted(board.GetFootprints(), key=lambda f: f.GetReference()):
    if not fp.HasField("LCSC"):
        continue
    lcsc = fp.GetFieldText("LCSC")
    key = (fp.GetValue(), fp.GetFPID().GetLibItemName().wx_str(), lcsc)
    rows.setdefault(key, {"refs": [], "mpn": fp.GetFieldText("MPN"), "mfr": fp.GetFieldText("Manufacturer")})["refs"].append(fp.GetReference())
    p = fp.GetPosition()
    cpl.append((fp.GetReference(), "%.4f" % P.ToMM(p.x), "%.4f" % -P.ToMM(p.y), "Bottom" if fp.IsFlipped() else "Top", "%g" % fp.GetOrientationDegrees()))
with open(os.path.join(out, "revb-flash-jig-bom-jlcpcb.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #", "Quantity", "Design MPN", "Design Manufacturer"])
    for (value, foot, lcsc), r in rows.items():
        w.writerow([value, ",".join(r["refs"]), foot, lcsc, len(r["refs"]), r["mpn"], r["mfr"]])
with open(os.path.join(out, "revb-flash-jig-cpl-jlcpcb.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
    w.writerows(cpl)
print("bom rows", len(rows), "cpl rows", len(cpl))
