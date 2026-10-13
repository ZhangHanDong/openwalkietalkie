#!/bin/zsh
# SPDX-License-Identifier: MIT
# PANEL_D1 / PANEL_D3 away from the antenna ground edge (run on the board with their copper
# deleted and their east-face + U3 escape vias restored; temporary AR keep-outs added).
IK="42.9:27.5:51.7:33.6;39.9:29.8:43.0:32.4;42.5:18.5:47.6:24.2;45.0:18.5:46.9:58.0;38.5:63.0:45.0:76.0;44.0:60.0:52.0:75.0;35.4:20.9:42.6:28.1;33.5:27.8:36.5:29.6"
# D3 first, halo 0.6 mm around the clearance (u >= 48.0, v <= 28.6)
$KPY maze.py IN.kicad_pcb STEP1.kicad_pcb --nets PANEL_D3 --audio-aware --via-cost 120 --layers F,B,In2 \
    --layer-cost In2=6 --inner-keepout "$IK" --keepout "F:47.4:18.5:53.6:29.2;B:47.4:18.5:53.6:29.2"
# then D1, halo 0.45 mm (the 0.6 mm lane is taken by D3)
$KPY maze.py STEP1.kicad_pcb OUT.kicad_pcb --nets PANEL_D1 --audio-aware --via-cost 120 --layers F,B,In2 \
    --layer-cost In2=6 --inner-keepout "$IK" --keepout "F:47.55:18.5:53.6:29.05;B:47.55:18.5:53.6:29.05"
