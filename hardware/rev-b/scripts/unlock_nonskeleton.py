#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Unlock every track/via that is not part of the skeleton board (KiCad python).

usage: unlock_nonskeleton.py BOARD.kicad_pcb SKELETON.kicad_pcb OUT.kicad_pcb
Skeleton = hand-routed modules + GND fan-out (locked). Autorouted and maze-routed copper
becomes unlocked, so the rip-up maze router may move it.
"""
import sys
import pcbnew

b = pcbnew.LoadBoard(sys.argv[1]); sk = pcbnew.LoadBoard(sys.argv[2])


def key(t):
    if t.GetClass() == 'PCB_VIA':
        p = t.GetPosition(); return ('V', t.GetNetname(), p.x // 1000, p.y // 1000)
    s, e = t.GetStart(), t.GetEnd()
    a_, b_ = sorted([(s.x // 1000, s.y // 1000), (e.x // 1000, e.y // 1000)])
    return ('T', t.GetNetname(), t.GetLayer(), a_, b_)


skel = set(key(t) for t in sk.GetTracks())
n_un = n_lk = 0
for t in b.GetTracks():
    if key(t) in skel:
        t.SetLocked(True); n_lk += 1
    else:
        t.SetLocked(False); n_un += 1
pcbnew.SaveBoard(sys.argv[3], b)
print(f'skeleton {n_lk} locked, {n_un} unlocked')
