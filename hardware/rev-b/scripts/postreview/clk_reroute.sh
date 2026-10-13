#!/bin/zsh
# SPDX-License-Identifier: MIT
# PANEL_CLK off In2 under the crystal (its 3.69 mm In2 run passed under the XO/XI escapes).
# Board: after the contained-segment dedupe and the CELL stub deletion (cell_stub.py).
# PANEL_CLK's copper is deleted by --reroute; PANEL_D2 and the nets in --soft-nets may be
# ripped up and re-routed. No In2 under Y1 ("40.2:27.5:42.8:32.8" in IK), B keep-out around Y1.
$KPY ar_keepouts.py add IN.kicad_pcb K.kicad_pcb        # without the antenna halo / DAC-strip boxes
IK="42.9:27.5:51.7:33.6;39.9:29.8:43.0:32.4;42.5:18.5:47.6:24.2;45.0:18.5:46.9:58.0;38.5:63.0:45.0:76.0;44.0:60.0:52.0:75.0;35.4:20.9:42.6:28.1;33.5:27.8:36.5:29.6;40.2:27.5:42.8:32.8"
KO="B:39.9:29.5:43.2:32.6;F:47.4:18.5:53.6:29.2;B:47.4:18.5:53.6:29.2"
$KPY maze.py K.kicad_pcb OUT.kicad_pcb --nets PANEL_CLK,PANEL_D2 --reroute --audio-aware --audio-fast-gap 0.55 \
    --via-cost 120 --layers F,B,In2 --layer-cost In2=6 --inner-keepout "$IK" --keepout "$KO" \
    --ripup --soft-cost 80 --rip-limit 3 --hist-cost 15 \
    --soft-nets "^PANEL_(CS|D0|D2|TE|RESET|CLK)$,^CAL$,^AMP_ENABLE$"
$KPY ar_keepouts.py remove OUT.kicad_pcb OUT.kicad_pcb
$KPY dedupe_tracks.py OUT.kicad_pcb OUT.kicad_pcb && zsh clean_loop.sh OUT.kicad_pcb
# R39.2's GND via back at (33.22, 30.0), with an In2 no-fill area for the sliver it forms
$KPY r39_via.py OUT.kicad_pcb FINAL.kicad_pcb
# clean_loop's end bridging can leave exact duplicates: drop them last
$KPY dedupe_tracks.py FINAL.kicad_pcb FINAL.kicad_pcb
