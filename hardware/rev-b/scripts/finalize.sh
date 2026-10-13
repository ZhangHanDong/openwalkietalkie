#!/bin/zsh
# SPDX-License-Identifier: MIT
# Finalize a routed Rev B board: pours + stitching, DRC, audio check, release exports.
# usage: finalize.sh ROUTED.kicad_pcb RELEASE_DIR [--no-pour]   (--no-pour: board already poured + cleaned)
set -e
RB=/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad/revb
T=$RB/../understand/tooling
KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
KC=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli
IN=${1:A}; REL=${2:A}
mkdir -p $REL/main $REL/verification $REL/fab-jlcpcb $REL/3d
B=$REL/main/main-revb.kicad_pcb
cp ${IN:r}.kicad_pro $REL/main/main-revb.kicad_pro
cp ${IN:r}.kicad_dru $REL/main/main-revb.kicad_dru
if [ "$3" = "--no-pour" ]; then cp $IN $B; else $KPY $RB/scripts/pours.py $IN $B 2>&1 | grep -v "assert\|Debug"; fi
# DRC (zones refilled and saved back)
$KC pcb drc --format json --severity-all --units mm --refill-zones --save-board -o $REL/verification/main-revb-drc.json $B > /dev/null 2>&1 || true
$KC pcb drc --format report --severity-all --units mm -o $REL/verification/main-revb-drc.rpt $B > /dev/null 2>&1 || true
python3 - $REL/verification/main-revb-drc.json <<'EOF'
import json, sys, collections
d = json.load(open(sys.argv[1]))
v = [x for x in d['violations'] if not x.get('excluded')]
print('DRC: %d violations (%s), %d unconnected' % (len(v), dict(collections.Counter(x['severity'] for x in v)), len(d['unconnected_items'])))
for t, n in collections.Counter(x['type'] for x in v).most_common():
    print('   ', t, n)
EOF
# audio crosstalk checker
$KPY $RB/../understand/audio_xtalk_check.py $B --refill --amp U1 --mcu U5 --flash U4 \
    --md $REL/verification/audio-xtalk.md --json $REL/verification/audio-xtalk.json 2>&1 | grep -v "assert\|Debug" | tail -3 || true
# placement check
$KPY $RB/scripts/extract_place.py $B $REL/verification/placement.json 2>&1 | grep -v "assert\|Debug" > /dev/null
$RB/scripts/geopy $RB/scripts/check_place.py $REL/verification/placement.json --md $REL/verification/placement-check.md --quiet || true
head -3 $REL/verification/placement-check.md
# 3D models actually referenced by the board, next to main/ (paths are ${KIPRJMOD}/../models/...)
mkdir -p $REL/models
grep -o 'models/[^"]*\.step' $B | sort -u | while read m; do cp $RB/$m $REL/models/; done
ls $REL/models | wc -l | xargs echo "models copied:"
# fabrication exports
zsh $T/export_fab.sh $B $REL/main > /dev/null 2>&1 || echo "export_fab failed"
mv $REL/main/3d/*.step $REL/3d/ 2>/dev/null || true
rm -rf $REL/main/3d
python3 $T/jlc_cpl.py $REL/main/assembly/positions-kicad-all.csv $REL/fab-jlcpcb/main-cpl-jlcpcb.csv --exclude 'MH*,TP*,C52,C53,C58,L6,R44'
$KPY $RB/scripts/build_bom.py $B $REL 2>&1 | grep -v "assert\|Debug"
$KPY $RB/scripts/orient_check.py $B $REL/fab-jlcpcb/main-bom-jlcpcb.csv $REL/fab-jlcpcb/main-orientation-check.csv 2>&1 | grep -v "assert\|Debug"
mv $REL/main/main-revb-jlcpcb-gerber.zip $REL/fab-jlcpcb/ 2>/dev/null || true
mv $REL/assembly/bom.csv $REL/main/assembly/bom.csv 2>/dev/null && rmdir $REL/assembly 2>/dev/null || true
ls $REL $REL/main $REL/fab-jlcpcb $REL/3d
