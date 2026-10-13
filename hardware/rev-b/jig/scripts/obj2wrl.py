# SPDX-License-Identifier: MIT
"""Convert an EasyEDA/LCSC OBJ model (mm) to VRML 2.0 for KiCad (1 unit = 2.54 mm)."""
import sys

src, dst = sys.argv[1], sys.argv[2]
verts, faces = [], []
for line in open(src):
    p = line.split()
    if not p:
        continue
    if p[0] == "v":
        verts.append(tuple(float(c) / 2.54 for c in p[1:4]))
    elif p[0] == "f":
        idx = [int(c.split("/")[0]) - 1 for c in p[1:]]
        for i in range(1, len(idx) - 1):
            faces.append((idx[0], idx[i], idx[i + 1]))
with open(dst, "w") as f:
    f.write("#VRML V2.0 utf8\n# Converted from the EasyEDA model of LCSC C5157237 (Xinyangze YZ105312061P-01)\n")
    f.write("Shape { appearance Appearance { material Material { diffuseColor 0.85 0.65 0.2 specularColor 0.9 0.8 0.5 shininess 0.6 } }\n")
    f.write(" geometry IndexedFaceSet { creaseAngle 0.8 coord Coordinate { point [\n")
    f.write(",\n".join("%.5f %.5f %.5f" % v for v in verts))
    f.write("\n] } coordIndex [\n")
    f.write(",\n".join("%d,%d,%d,-1" % t for t in faces))
    f.write("\n] } }\n")
print(len(verts), "verts", len(faces), "tris")
