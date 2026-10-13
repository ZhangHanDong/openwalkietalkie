#!/bin/zsh
# SPDX-License-Identifier: MIT
# Full Rev B routing pass: hand modules -> GND fan-out -> Freerouting (rest) -> import -> DRC
# usage: autoroute.sh WORKDIR [modules=mcu,rf,audio,power] [passes=40] [threads=4]
set -e
RB=/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb
T=$RB/../understand/tooling
KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
W=${1:A}; MODS=${2:-mcu,rf,audio,power}; PASSES=${3:-40}; THREADS=${4:-4}
mkdir -p $W/imp
cd $RB
$KPY scripts/route_revb.py $W/hand.kicad_pcb $MODS 2>&1 | grep -v "assert\|Debug" || true
cp $W/hand.kicad_pro $W/gnd.kicad_pro; cp $W/hand.kicad_dru $W/gnd.kicad_dru
$KPY scripts/gnd_fanout.py $W/hand.kicad_pcb $W/gnd0.kicad_pcb --skip U5 2>&1 | grep -v "assert\|Debug" || true
$KPY scripts/ar_keepouts.py add $W/gnd0.kicad_pcb $W/gnd.kicad_pcb 2>&1 | grep -v "assert\|Debug" || true
$KPY $T/dsn_export.py $W/gnd.kicad_pcb $W/raw.dsn --lock-rest --save-prepped $W/prepped.kicad_pcb 2>&1 | grep -v "assert\|Debug\|dropped" || true
cp $W/gnd.kicad_pro $W/prepped.kicad_pro; cp $W/gnd.kicad_dru $W/prepped.kicad_dru
python3 $RB/scripts/dsn_classes.py $W/raw.dsn $W/cls.dsn
NETS=$(python3 $RB/scripts/unrouted_nets.py $W/prepped.kicad_pcb)
echo "nets to route: $(echo $NETS | tr ',' '\n' | wc -l)"
python3 $T/dsn_tweak.py $W/cls.dsn $W/route.dsn --route-nets "$NETS" --isolate > /dev/null
python3 $T/fr_route.py $W/route.dsn $W/route.ses --passes $PASSES --threads $THREADS --timeout 3000 | tail -4
cp $W/prepped.kicad_pro $W/imp/routed.kicad_pro; cp $W/prepped.kicad_dru $W/imp/routed.kicad_dru
$KPY $T/ses_import.py $W/prepped.kicad_pcb $W/route.ses $W/imp/routed.kicad_pcb --refill --only-nets "$NETS" --report $W/import.json 2>&1 | grep -v "assert\|Debug" | tail -3
$KPY scripts/ar_keepouts.py remove $W/imp/routed.kicad_pcb $W/imp/routed.kicad_pcb 2>&1 | grep -v "assert\|Debug" || true
python3 $RB/scripts/wdrc.py $W/imp/routed.kicad_pcb --refill | head -30
