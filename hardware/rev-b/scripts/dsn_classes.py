#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Set Freerouting widths/clearances per net class in a KiCad-exported DSN (stdlib only).

Power trunks are pre-routed by hand, so the autorouter only adds branches: narrow widths that
fit the 0.35 mm QFN pins. AudioAnalog gets a 0.5 mm clearance so new wires keep away from the
locked analog audio copper; RF50 keeps its 0.2 mm coplanar gap.
usage: dsn_classes.py IN.dsn OUT.dsn
"""
import re, sys
W = {  # class: (width um, clearance um)
    'kicad_default': (120, 100), 'Default': (120, 100),
    'SystemBatteryTrunks': (250, 100), 'UsbPowerTrunk': (250, 100), 'LogicPowerTrunks': (150, 100),
    'PanelPowerTrunk': (250, 100), 'AudioOutputTrunks': (300, 150), 'McuLdoDistribution': (200, 100),
    'McuPower': (200, 100), 'GroundLocal': (200, 100), 'AudioAnalog': (150, 500), 'RF50': (160, 200),
}
s = open(sys.argv[1]).read()


def fix(m):
    name, block = m.group(1), m.group(0)
    if name in W:
        w, c = W[name]
        block = re.sub(r'\(width \d+(\.\d+)?\)', f'(width {w})', block, count=1)
        block = re.sub(r'\(clearance \d+(\.\d+)?\)', f'(clearance {c})', block, count=1)
    return block


s2 = re.sub(r'\(class (\S+) .*?\n    \)', fix, s, flags=re.S)

# Freerouting kept only ~0.08 mm from the long side of 0402 pads on parts rotated by 90 deg
# (KiCad DRC: 0.1). Square up near-square passive pads (aspect <= 1.3, both sides >= 0.45 mm)
# in the router's copy only, so it keeps the full clearance whatever the rotation.
def square(m):
    x1, y1, x2, y2 = (float(v) for v in m.group(2, 3, 4, 5))
    w, h = x2 - x1, y2 - y1
    if min(w, h) >= 450 and max(w, h) / min(w, h) <= 1.3:
        a = max(w, h) / 2
        return f'(rect {m.group(1)} {-a:g} {-a:g} {a:g} {a:g})'
    return m.group(0)
s2, nsq = re.subn(r'\(rect (\S+) (-?[\d.]+) (-?[\d.]+) (-?[\d.]+) (-?[\d.]+)\)', square, s2)
open(sys.argv[2], 'w').write(s2)
print('rect padstacks scanned:', nsq)
print('classes:', re.findall(r'\(class (\S+)[^(]*?\(circuit.*?\(width (\d+)\)\s*\(clearance (\d+)\)', s2, flags=re.S))
