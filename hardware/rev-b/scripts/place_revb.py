#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Rev B placement (KiCad python). Reads main-revb.kicad_pcb, writes main-revb-placed.kicad_pcb.

Coordinates are board (u, v) mm. 'B' = display side (front), 'F' = battery side.
place(ref, side, x, y, rot) for parts with a defined rotation; p2(ref, side, x, y, toward)
for 2-pad parts aimed at a pad ('U5.12') so the pad sharing that net faces it.
"""
import sys
sys.path.insert(0, '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb/scripts')
import pcbnew
from placelib import place, place2, swap_fp, fpr

S = '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb/main'
IN = S + '/main-revb.kicad_pcb'
OUT = sys.argv[1] if len(sys.argv) > 1 else S + '/main-revb-placed.kicad_pcb'
b = pcbnew.LoadBoard(IN)
P = lambda *a: place(b, *a)
P2 = lambda *a: place2(b, *a)

# ---------------------------------------------------------------- package changes
# MCU decoupling 0603 -> 0402 (same values) so the caps fit at the pins of the
# relocated MCU under the display; C28 (VCC, 4.4 V) stays 0603.
M0402 = '${KIPRJMOD}/../models/CC0402KRX5R6BB474.step'
for r in ('C13', 'C14', 'C15', 'C16', 'C17', 'C23', 'C24', 'C26', 'C27', 'C28', 'C29', 'C30', 'C31'):
    swap_fp(b, r, 'C25', M0402)

import importlib
sys.path.insert(0, '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb/scripts/regions')
for mod in ('mcu', 'power', 'topleft', 'audio'):
    importlib.import_module(mod).apply(b, P, P2)

pcbnew.SaveBoard(OUT, b)
from placelib import dedupe_uuids
dedupe_uuids(OUT)
print('saved', OUT)
