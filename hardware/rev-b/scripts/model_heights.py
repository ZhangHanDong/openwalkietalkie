# SPDX-License-Identifier: MIT
import os, json, sys
from build123d import import_step
d = sys.argv[1]; out = {}
for fn in sorted(os.listdir(d)):
    if not fn.lower().endswith(('.step', '.stp')): continue
    try:
        s = import_step(os.path.join(d, fn)); bb = s.bounding_box()
        out[fn] = dict(zmin=round(bb.min.Z,3), zmax=round(bb.max.Z,3), x=[round(bb.min.X,3), round(bb.max.X,3)], y=[round(bb.min.Y,3), round(bb.max.Y,3)])
    except Exception as e:
        out[fn] = dict(error=str(e))
    print(fn, out[fn], flush=True)
json.dump(out, open(sys.argv[2], 'w'), indent=1)
