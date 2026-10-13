#!/bin/zsh
# SPDX-License-Identifier: MIT
# Repeat DRC + cleanup.py until no dangling vias/tracks remain; keeps the project rules files.
# usage: clean_loop.sh BOARD.kicad_pcb
RB=/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb
KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
B=${1:A}; T=${B:r}-cl.kicad_pcb
cd $RB
for i in 1 2 3 4 5 6; do
  python3 scripts/wdrc.py $B --refill > /dev/null
  n=$(python3 -c "import json;d=json.load(open('${B:r}-wdrc.json'));print(sum(1 for v in d['violations'] if v['type'] in ('track_dangling','via_dangling','starved_thermal','holes_co_located')))")
  echo "clean pass $i: $n items"
  [ "$n" = "0" ] && break
  cp $B $T; cp ${B:r}.kicad_pro ${T:r}.kicad_pro; cp ${B:r}.kicad_dru ${T:r}.kicad_dru
  $KPY scripts/cleanup.py $T ${B:r}-wdrc.json $B 2>&1 | grep -v "assert\|Debug" | tail -1
  cp ${T:r}.kicad_pro ${B:r}.kicad_pro; cp ${T:r}.kicad_dru ${B:r}.kicad_dru
done
rm -f $T ${T:r}.kicad_pro ${T:r}.kicad_dru ${T:r}.kicad_prl
python3 scripts/wdrc.py $B --refill --unconn --all | head -12
