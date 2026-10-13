# SPDX-License-Identifier: MIT
"""Delete the redundant CELL path left on B by the C22 re-route: (36.95,19.5)-(36.85,19.6)
-(36.85,20.81), which ran beside the kept (36.95,18.3)-(36.95,20.15)-(36.85,20.25)-(36.85,21.0).
Run after dedupe_tracks.py (which drops the contained (36.95,18.3)-(36.95,19.5)).
usage: cell_stub.py IN OUT"""
import pcbnew, sys
b = pcbnew.LoadBoard(sys.argv[1]); mm = pcbnew.FromMM
gone = [{(36.85, 19.6), (36.85, 20.81)}, {(36.95, 19.5), (36.85, 19.6)}]
for t in list(b.GetTracks()):
    if t.GetClass() == 'PCB_VIA' or t.GetNetname() != 'CELL':
        continue
    ends = {(round(t.GetStart().x / 1e6, 2), round(t.GetStart().y / 1e6, 2)),
            (round(t.GetEnd().x / 1e6, 2), round(t.GetEnd().y / 1e6, 2))}
    if ends in gone:
        b.Delete(t); print('deleted', sorted(ends))
b.Save(sys.argv[2])
