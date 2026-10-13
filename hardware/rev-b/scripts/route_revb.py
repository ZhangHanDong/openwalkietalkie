#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Apply hand-route modules to the rules board. usage: route_revb.py OUT.kicad_pcb mod1,mod2,..."""
import sys, os, importlib, shutil
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE); sys.path.insert(0, os.path.join(HERE, 'route'))
import pcbnew
from routelib import Router
IN = os.path.join(HERE, '..', 'main', 'main-revb-rules.kicad_pcb')
OUT = sys.argv[1]
mods = sys.argv[2].split(',') if len(sys.argv) > 2 else []
b = pcbnew.LoadBoard(IN)
R = Router(b)
for m in mods:
    importlib.import_module(m).apply(R)
pcbnew.SaveBoard(OUT, b)
for ext in ('.kicad_pro', '.kicad_dru'):
    src = os.path.splitext(IN)[0] + ext
    if os.path.exists(src):
        shutil.copy(src, os.path.splitext(OUT)[0] + ext)
print(f'saved {OUT}: +{R.n_tracks} tracks, +{R.n_vias} vias')
