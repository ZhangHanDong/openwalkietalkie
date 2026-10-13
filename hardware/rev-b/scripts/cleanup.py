#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Post-route cleanup from a DRC report (KiCad python).

usage: cleanup.py IN.kicad_pcb DRC.json OUT.kicad_pcb
* deletes vias/tracks reported as dangling (vias connected on one layer only are
  superfluous escape vias the routers did not use);
* pads reported in starved_thermal get a solid zone connection (they are exposed / mounting
  pads where a thermal relief cannot fit all its spokes; solid is also better thermally).
Run DRC again afterwards: if a removed via turns out to have joined two tracks, the
unconnected list shows it (restore by re-running without that via, see --keep).
"""
import json, sys
import pcbnew

IN, DRC, OUT = sys.argv[1:4]
keep = set()
if '--keep' in sys.argv:
    keep = set(sys.argv[sys.argv.index('--keep') + 1].split(';'))
b = pcbnew.LoadBoard(IN)
d = json.load(open(DRC))
byid = {t.m_Uuid.AsString(): t for t in b.GetTracks()}
pads = {}
for f in b.GetFootprints():
    for p in f.Pads():
        pads[p.m_Uuid.AsString()] = (f.GetReference(), p)
n_del = n_th = 0
for v in d['violations']:
    if v['type'] in ('via_dangling', 'track_dangling'):
        for it in v['items']:
            t = byid.pop(it.get('uuid'), None)
            if t is not None:
                k = f"{t.GetNetname()}@{pcbnew.ToMM(t.GetPosition().x):.2f},{pcbnew.ToMM(t.GetPosition().y):.2f}"
                if k in keep:
                    continue
                if t.GetClass() == 'PCB_VIA':
                    # tracks that end inside the via pad but off-centre were joined only through
                    # the via's copper: extend them to the centre first
                    c = t.GetPosition(); r = t.GetWidth(pcbnew.F_Cu) / 2
                    for o in list(b.GetTracks()):
                        if o.GetClass() == 'PCB_VIA' or o.GetNetCode() != t.GetNetCode():
                            continue
                        for e in (o.GetStart(), o.GetEnd()):
                            dd = ((e.x - c.x) ** 2 + (e.y - c.y) ** 2) ** 0.5
                            if 0 < dd <= r:
                                br = pcbnew.PCB_TRACK(b)
                                br.SetStart(pcbnew.VECTOR2I(e.x, e.y)); br.SetEnd(pcbnew.VECTOR2I(c.x, c.y))
                                br.SetWidth(o.GetWidth()); br.SetLayer(o.GetLayer()); br.SetNet(o.GetNet())
                                br.SetLocked(o.IsLocked()); b.Add(br)
                b.Delete(t); n_del += 1
    elif v['type'] == 'holes_co_located':
        ts = [byid.get(it.get('uuid')) for it in v['items']]
        ts = [t for t in ts if t is not None and t.GetClass() == 'PCB_VIA']
        if len(ts) == 2 and ts[0].GetNetCode() == ts[1].GetNetCode():
            byid.pop(ts[1].m_Uuid.AsString(), None)
            b.Delete(ts[1]); n_del += 1
    elif v['type'] == 'starved_thermal':
        for it in v['items']:
            if it.get('uuid') in pads:
                ref, p = pads[it['uuid']]
                p.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL); n_th += 1
                print('  solid zone connection:', ref, p.GetNumber())
filler = pcbnew.ZONE_FILLER(b)
filler.Fill(b.Zones())
pcbnew.SaveBoard(OUT, b)
print(f'deleted {n_del} dangling items, {n_th} pads set to solid')
