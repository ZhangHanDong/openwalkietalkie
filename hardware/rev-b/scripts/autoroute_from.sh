#!/bin/zsh
# SPDX-License-Identifier: MIT
# Freerouting pass from an already prepared board (hand routes + GND fan-out + any maze routes).
# usage: autoroute_from.sh BOARD.kicad_pcb WORKDIR [passes=50] [threads=6]
set -e
RB=/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb
T=$RB/../understand/tooling
KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
IN=${1:A}; W=${2:A}; PASSES=${3:-50}; THREADS=${4:-6}
mkdir -p $W/imp
cd $RB
cp ${IN:r}.kicad_pro $W/gnd.kicad_pro; cp ${IN:r}.kicad_dru $W/gnd.kicad_dru
$KPY scripts/ar_keepouts.py add $IN $W/gnd.kicad_pcb 2>&1 | grep -v "assert\|Debug" || true
$KPY $T/dsn_export.py $W/gnd.kicad_pcb $W/raw.dsn --lock-rest --save-prepped $W/prepped.kicad_pcb 2>&1 | grep -v "assert\|Debug\|dropped" || true
cp $W/gnd.kicad_pro $W/prepped.kicad_pro; cp $W/gnd.kicad_dru $W/prepped.kicad_dru
python3 $RB/scripts/dsn_classes.py $W/raw.dsn $W/cls.dsn > /dev/null
NETS=$(python3 $RB/scripts/unrouted_nets.py $W/prepped.kicad_pcb)
echo "nets to route: $(echo $NETS | tr ',' '\n' | wc -l | tr -d ' ')"
python3 $T/dsn_tweak.py $W/cls.dsn $W/route.dsn --route-nets "$NETS" --isolate > /dev/null
python3 $T/fr_route.py $W/route.dsn $W/route.ses --passes $PASSES --threads $THREADS --timeout 3000 | tail -2
cp $W/prepped.kicad_pro $W/imp/routed.kicad_pro; cp $W/prepped.kicad_dru $W/imp/routed.kicad_dru
$KPY $T/ses_import.py $W/prepped.kicad_pcb $W/route.ses $W/imp/routed.kicad_pcb --refill --only-nets "$NETS" --report $W/import.json 2>&1 | grep -v "assert\|Debug" | tail -2
$KPY scripts/ar_keepouts.py remove $W/imp/routed.kicad_pcb $W/imp/routed.kicad_pcb 2>&1 | grep -v "assert\|Debug" || true
python3 $RB/scripts/wdrc.py $W/imp/routed.kicad_pcb --refill --unconn | grep -v "dangling" | grep -v "GND\]" | head -40
