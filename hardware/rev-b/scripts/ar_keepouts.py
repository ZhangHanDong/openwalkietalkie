#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Temporary autorouter keep-outs (KiCad python).

usage: ar_keepouts.py add IN.kicad_pcb OUT.kicad_pcb
       ar_keepouts.py remove IN.kicad_pcb OUT.kicad_pcb
'add' puts 'AR keepout: ...' rule areas (no tracks, no vias) over the analog/RF areas whose
copper is already hand-routed and locked, so Freerouting keeps new wires out of them;
'remove' deletes them again after the SES import.
"""
import sys
import pcbnew

mm = pcbnew.FromMM
op, IN, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
b = pcbnew.LoadBoard(IN)

AREAS = [  # name, layers, (u0, v0, u1, v1)
    ('RF line (B)', (pcbnew.B_Cu,), (43.0, 29.35, 51.6, 33.45)),
    ('RF network (F)', (pcbnew.F_Cu,), (49.55, 28.6, 53.3, 33.3)),
    ('DAC filter (B)', (pcbnew.B_Cu,), (42.95, 18.7, 47.5, 22.3)),
    ('DAC pair corridor (F)', (pcbnew.F_Cu,), (45.25, 18.75, 46.6, 57.6)),
    ('amplifier inputs (F)', (pcbnew.F_Cu,), (38.7, 64.3, 43.3, 71.4)),
    ('crystal (B)', (pcbnew.B_Cu,), (40.1, 29.95, 42.95, 32.15)),
    ('AUDIO_REF via + C15 (F)', (pcbnew.F_Cu,), (42.95, 22.75, 44.0, 24.1)),
    # 3 mm halo around the antenna clearance, the crystal pads and the strip east of the DAC pair
    ('antenna halo (F)', (pcbnew.F_Cu,), (47.0, 18.5, 53.6, 29.6)),
    ('antenna halo low (F)', (pcbnew.F_Cu,), (47.4, 29.6, 53.6, 31.6)),
    ('antenna halo (B)', (pcbnew.B_Cu,), (45.0, 18.5, 53.6, 31.6)),
    ('crystal pads (B)', (pcbnew.B_Cu,), (39.9, 29.5, 43.2, 32.6)),
    ('DAC east strip (F)', (pcbnew.F_Cu,), (45.2, 31.6, 48.2, 58.5)),
    # no vias over the In1 supply links (FR treats In1 as a plane and would drop vias on them)
    ('In1 VSYS link, channel', (pcbnew.In1_Cu,), (36.95, 20.9, 37.72, 28.4)),
    ('In1 VSYS link, south', (pcbnew.In1_Cu,), (36.95, 27.6, 43.05, 28.4)),
    ('In1 LDO1 link', (pcbnew.In1_Cu,), (37.6, 20.85, 44.25, 21.55)),
    ('In1 LDO1 link, east', (pcbnew.In1_Cu,), (43.55, 20.85, 45.0, 22.3)),
]

for z in list(b.Zones()):
    if z.GetIsRuleArea() and z.GetZoneName().startswith('AR keepout'):
        b.Delete(z)

if op == 'add':
    for name, layers, (u0, v0, u1, v1) in AREAS:
        z = pcbnew.ZONE(b)
        z.SetIsRuleArea(True)
        ls = pcbnew.LSET()
        for l in layers:
            ls.AddLayer(l)
        z.SetLayerSet(ls)
        z.SetZoneName('AR keepout: ' + name)
        z.SetDoNotAllowTracks(True); z.SetDoNotAllowVias(True)
        z.SetDoNotAllowPads(False); z.SetDoNotAllowFootprints(False); z.SetDoNotAllowZoneFills(False)
        o = z.Outline()
        o.RemoveAllContours()
        o.NewOutline()
        for x, y in ((u0, v0), (u1, v0), (u1, v1), (u0, v1)):
            o.Append(mm(x), mm(y))
        b.Add(z)
pcbnew.SaveBoard(OUT, b)
print(op, 'keepouts ->', OUT)
