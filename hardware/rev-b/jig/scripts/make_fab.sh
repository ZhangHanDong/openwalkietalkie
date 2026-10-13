#!/bin/sh
# SPDX-License-Identifier: MIT
# Rev B flash jig: DRC, JLCPCB Gerber/drill zip, BOM, CPL and renders.
set -eu
cd "$(dirname "$0")/.."
KC=${KICAD_CLI:-/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli}
PY=${KICAD_PY:-/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3}
B=revb-flash-jig.kicad_pcb
OUT=fab-jlcpcb
mkdir -p "$OUT" verification
"$KC" pcb drc --severity-all --exit-code-violations -o verification/revb-flash-jig-drc.rpt "$B"
G=$(mktemp -d)
"$KC" pcb export gerbers --no-protel-ext -l F.Cu,B.Cu,F.Paste,F.Mask,B.Mask,F.SilkS,B.SilkS,Edge.Cuts -o "$G/" "$B" >/dev/null
"$KC" pcb export drill --format excellon --excellon-separate-th --excellon-units mm -o "$G/" "$B" >/dev/null
rm -f "$OUT/revb-flash-jig-gerber.zip"
(cd "$G" && zip -qX -r - .) > "$OUT/revb-flash-jig-gerber.zip"
rm -rf "$G"
"$PY" scripts/bom_cpl.py "$B" "$OUT" 2>/dev/null
"$KC" pcb render --side top -w 900 -h 1300 --quality high -o render-top.png "$B" >/dev/null
"$KC" pcb render --side bottom -w 900 -h 1300 --quality high -o render-bottom.png "$B" >/dev/null
"$KC" pcb render --side top --rotate 300,0,30 --zoom 1.2 -w 1200 -h 900 --quality high -o render-3d.png "$B" >/dev/null
echo "ok: $OUT/"
