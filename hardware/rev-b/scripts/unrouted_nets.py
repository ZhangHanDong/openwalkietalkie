#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Print a comma list of nets with unconnected items (kicad-cli DRC), excluding GND. usage: unrouted_nets.py BOARD"""
import sys, json, re, subprocess, os
KC = '/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli'
b = sys.argv[1]; out = os.path.splitext(b)[0] + '-unrouted.json'
subprocess.run([KC, 'pcb', 'drc', '--format', 'json', '--severity-all', '--units', 'mm', '-o', out, b], capture_output=True)
d = json.load(open(out))
nets = set()
for u in d['unconnected_items']:
    for it in u['items']:
        m = re.search(r'\[([^\]]+)\]', it['description'])
        if m:
            nets.add(m.group(1))
nets.discard('GND')
print(','.join(sorted(nets)))
