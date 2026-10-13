# SPDX-License-Identifier: MIT
"""Pad-net / value / DNP diff of a Rev B board against r6 (verification/netlist-diff-vs-r6.txt).
usage: netlist_diff.py REVB_BOARD > netlist-diff-vs-r6.txt"""
import sys
import pcbnew
R6 = '/Users/tony/projects/conol/openmicrodial/hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/main/main-45deg.kicad_pcb'
def pads(path):
    b = pcbnew.LoadBoard(path); d = {}; vals = {}
    for f in b.GetFootprints():
        vals[f.GetReference()] = (f.GetValue(), f.IsDNP())
        for p in f.Pads():
            if p.GetNumber(): d[(f.GetReference(), p.GetNumber())] = p.GetNetname()
    return d, vals
a, va = pads(R6); bb, vb = pads(sys.argv[1])
print('# Pad-net differences, r6 (main-45deg.kicad_pcb) -> Rev B (main-revb.kicad_pcb); pads without a number ignored')
refs_a = set(r for r, _ in a); refs_b = set(r for r, _ in bb)
print('removed parts:', sorted(refs_a - refs_b))
print('added parts:', sorted(refs_b - refs_a))
for r in sorted(refs_b - refs_a):
    print('  +', r, vb[r], {n: bb[(r, n)] for (rr, n) in bb if rr == r})
print('changed pad nets on common parts:')
for k in sorted(set(a) & set(bb)):
    if a[k] != bb[k]: print('  ', k, repr(a[k]), '->', repr(bb[k]))
print('pads only in Rev B (common parts):', sorted(k for k in set(bb) - set(a) if k[0] in refs_a))
print('value / DNP changes:')
for r in sorted(refs_a & refs_b):
    if va[r] != vb[r]: print('  ', r, va[r], '->', vb[r])
