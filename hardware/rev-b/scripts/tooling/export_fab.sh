#!/bin/zsh
# SPDX-License-Identifier: MIT
# Fabrication exports with kicad-cli 10.0.4, reproducing the r6 release layout/naming.
# usage: export_fab.sh BOARD.kicad_pcb OUTDIR
#   OUTDIR/gerber   9 layers (Cu x4, Mask x2, Silk x2, Edge) + job file   (= r6 main/gerber)
#   OUTDIR/stencil  F/B paste + job file                                   (= r6 main/stencil)
#   OUTDIR/drill    PTH + NPTH Excellon (mm, decimal, absolute, slots as routed G00/M15),
#                   SVG drill maps, drill report                           (= r6 main/drill)
#   OUTDIR/assembly positions-kicad-all.csv (KiCad CSV, mm, both sides)    (= r6 main/reference)
#   OUTDIR/3d       populated STEP (models via ${KIPRJMOD}/../models)
#   OUTDIR/<name>-jlcpcb-gerber.zip  gerber+paste+drill, flat (= r6 fab-jlcpcb/main zip)
# STEP_EXTRA="--include-tracks --include-pads --include-zones" adds copper to the STEP.
# Gerber names keep KiCad's .gbr extension (r6 used --no-protel-ext; JLCPCB accepts both,
# and the .gbrjob + X2 FileFunction attributes identify the layers).
set -e
KC=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli
B=$1; O=$2
N=$(basename "$B" .kicad_pcb)
mkdir -p "$O"/{gerber,stencil,drill,assembly,3d}
CU=F.Cu,In1.Cu,In2.Cu,B.Cu,F.Mask,B.Mask,F.Silkscreen,B.Silkscreen,Edge.Cuts
$KC pcb export gerbers --no-protel-ext -l $CU -o "$O/gerber/" "$B"
$KC pcb export gerbers --no-protel-ext -l F.Paste,B.Paste -o "$O/stencil/" "$B"
$KC pcb export drill --format excellon --drill-origin absolute --excellon-zeros-format decimal \
    --excellon-oval-format route -u mm --excellon-separate-th --generate-map --map-format svg \
    --generate-report --report-path "$O/drill/drill-report.txt" -o "$O/drill/" "$B"
$KC pcb export pos --format csv --units mm --side both -o "$O/assembly/positions-kicad-all.csv" "$B"
$KC pcb export step -f --subst-models ${=STEP_EXTRA} \
    -o "$O/3d/$N-populated-native.step" "$B" > "$O/3d/step-export.log" 2>&1 || echo "STEP export failed, see $O/3d/step-export.log"
( cd "$O" && rm -f "$N-jlcpcb-gerber.zip" && \
  zip -j -q "$N-jlcpcb-gerber.zip" gerber/*.gbr stencil/*.gbr drill/*.drl )
ls -la "$O"/gerber "$O"/stencil "$O"/drill "$O"/assembly "$O"/3d "$O"/*.zip
