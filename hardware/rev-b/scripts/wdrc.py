#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""DRC summary for a window: wdrc.py BOARD [u0,v0,u1,v1] [--types a,b] [--all]
Runs kicad-cli DRC (no zone refill unless --refill), prints violations whose items lie in the window
(courtyard/silk/lib issues hidden unless --all), and unconnected items in the window."""
import sys, json, subprocess, os, collections
KC = '/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli'
board = sys.argv[1]
win = None
args = sys.argv[2:]
if args and ',' in args[0] and not args[0].startswith('--'):
    win = tuple(float(x) for x in args[0].split(',')); args = args[1:]
show_all = '--all' in args
refill = '--refill' in args
out = os.path.splitext(board)[0] + '-wdrc.json'
cmd = [KC, 'pcb', 'drc', '--format', 'json', '--severity-all', '--units', 'mm', '-o', out]
if refill:
    cmd.append('--refill-zones')
subprocess.run(cmd + [board], capture_output=True)
d = json.load(open(out))
HIDE = {'courtyards_overlap', 'silk_overlap', 'silk_over_copper', 'silk_edge_clearance', 'lib_footprint_issues',
        'lib_footprint_mismatch', 'text_height', 'text_thickness', 'footprint_type_mismatch', 'solder_mask_bridge', 'isolated_copper'}
def inwin(v):
    if not win:
        return True
    return any(win[0] <= it['pos']['x'] <= win[2] and win[1] <= it['pos']['y'] <= win[3] for it in v['items'])
vs = [v for v in d['violations'] if inwin(v) and (show_all or v['type'] not in HIDE)]
us = [v for v in d['unconnected_items'] if inwin(v)]
print(f'violations {len(vs)} (all {len(d["violations"])}), unconnected {len(us)} (all {len(d["unconnected_items"])})')
print('  by type:', dict(collections.Counter(v['type'] for v in vs)))
for v in vs[:40]:
    print('  -', v['type'], '|', v['description'][:70], '|', ' / '.join(f"{it['description'][:55]} @({it['pos']['x']:.2f},{it['pos']['y']:.2f})" for it in v['items'][:2]))
if '--unconn' in args:
    for v in us[:60]:
        print('  ~', ' / '.join(f"{it['description'][:50]} @({it['pos']['x']:.2f},{it['pos']['y']:.2f})" for it in v['items'][:2]))
