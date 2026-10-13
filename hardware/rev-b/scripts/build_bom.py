#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Rev B BOMs from the board (KiCad python).

usage: build_bom.py BOARD.kicad_pcb OUTDIR
Writes OUTDIR/assembly/bom.csv (design BOM: Comment,Designator,Footprint,Manufacturer,MPN,Quantity)
and OUTDIR/fab-jlcpcb/main-bom-jlcpcb.csv (r6 JLC format without its 'Exact-MPN C# (alt)'
column). Part data comes from the r6 JLC BOM
by designator where the part is unchanged, and from REVB below for new or changed parts.
DNP / not-in-BOM footprints, test points and mounting holes are left out.
"""
import csv, os, sys, collections
import pcbnew

R6BOM = '/Users/tony/projects/conol/openmicrodial/hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/fab-jlcpcb/main/main-bom-jlcpcb.csv'
# designator -> (comment, LCSC, design MPN, manufacturer, substitute placed, note)
REVB = {}
def add(refs, *row):
    for r in refs.split(','):
        REVB[r] = row
add('C13,C17,C29,C30', '4.7uF', 'C23733', 'CL05A475MP5NRNC', 'Samsung Electro-Mechanics', '', 'Rev B: 0603 -> 0402 at the relocated MCU. 4.7uF 10V X5R 0402 (JLC Basic). Rails <= 3.3 V.')
add('C28', '4.7uF', 'C23733', 'CL05A475MP5NRNC', 'Samsung Electro-Mechanics', '', 'Rev B: 0603 -> 0402 to fit the F-side cap row behind the MCU (U5 pin 24 VCC decoupling). On SYSTEM_SUPPLY (up to 4.4 V): 10V X5R keeps 2x voltage margin, but DC-bias derating leaves roughly half the nominal capacitance; the SYSTEM_SUPPLY bulk caps (C12 10uF 0805, C42/C45 10uF 1206) carry the rest.')
add('C14,C16,C23,C24,C27,C31', '2.2uF', 'C107369', 'CL05A225KP5NSNC', 'Samsung Electro-Mechanics', '', 'Rev B: 0603 -> 0402 at the relocated MCU. 2.2uF 10V X5R 0402 (10 V chosen for the 3.3 V rails; the 6.3 V Basic C12530 also fits).')
add('C15,C26', '1uF', 'C52923', 'CL05A105KA5NQNC', 'Samsung Electro-Mechanics', '', 'Rev B: 0603 -> 0402 at the relocated MCU. 1uF 25V X5R 0402 (JLC Basic).')
add('C57', '15pF', 'C1548', '0402CG150J500NT', 'FH (Guangdong Fenghua)', '', 'Rev B: RF DC block, series in the SiFli pi at U5 pin 53. C0G 50V (JLC Basic).')
add('J3', '53261-0371', 'C293630', '53261-0371', 'Molex', '', 'Rev B battery connector (MX1.25 / PicoBlade 3-pin SMT right angle): 1 VCELL, 2 NTC, 3 GND. Mates with 51021-0300.')
add('R45', '100kohm', 'C25741', 'RC0402FR-07100KL', 'Yageo', 'UniOhm 0402WGF1003TCE 100k 1% 0402 (JLC Basic)', 'Rev B: TOUCH_RESET pull-down.')
add('R46', '100kohm', 'C25741', 'RC0402FR-07100KL', 'Yageo', 'UniOhm 0402WGF1003TCE 100k 1% 0402 (JLC Basic)', 'Rev B: CHG_STATUS pull-up to IO_3V3 (charger CHG -> MCU PA33).')
add('R40,R47', '0ohm', 'C17168', 'RC0402JR-070RL', 'Yageo', 'UniOhm 0402WGF0000TCE 0R 0402 (JLC Basic)', 'Rev B: antenna-side match S1/S2 (0R until tuned; Johanson eval values S1 1.2 pF, S2 3.3 nH).')
add('R48', '0ohm', 'C17168', 'RC0402JR-070RL', 'Yageo', 'UniOhm 0402WGF0000TCE 0R 0402 (JLC Basic)', 'Rev B: ties display TP_VCC (U3 pin 6) to IO_3V3; remove to float the pin as on r6.')

b = pcbnew.LoadBoard(sys.argv[1])
out = sys.argv[2]
r6 = {}
for row in csv.DictReader(open(R6BOM)):
    for d in row['Designator'].split(','):
        r6[d.strip()] = row

groups = collections.OrderedDict()
missing = []
for f in sorted(b.GetFootprints(), key=lambda f: (f.GetReference()[0], int(''.join(c for c in f.GetReference() if c.isdigit()) or 0))):
    ref = f.GetReference()
    if f.IsDNP() or f.IsExcludedFromBOM() or ref.startswith(('TP', 'MH')):
        continue
    fpname = str(f.GetFPID().GetLibItemName())
    fpname = fpname[fpname.index('FP_'):] if 'FP_' in fpname else fpname.split('-')[-1]
    if ref in REVB:
        comment, lcsc, mpn, mfr, sub, note = REVB[ref]
    elif ref in r6:
        row = r6[ref]
        comment, lcsc, mpn, mfr, sub, note = row['Comment'], row['LCSC Part #'], row['Design MPN'], row['Design Manufacturer'], row['Part placed (if substituted)'], row['Note']
        if f.GetValue() != comment and not (comment.startswith(f.GetValue()) or f.GetValue() in comment):
            missing.append(f'{ref}: board value {f.GetValue()} vs r6 comment {comment}')
    else:
        missing.append(f'{ref}: no BOM data')
        continue
    key = (comment, fpname, lcsc, mpn, note)          # same part with a different note -> own line
    g = groups.setdefault(key, dict(refs=[], mfr=mfr, sub=sub, note=note))
    g['refs'].append(ref)

os.makedirs(os.path.join(out, 'assembly'), exist_ok=True)
os.makedirs(os.path.join(out, 'fab-jlcpcb'), exist_ok=True)
with open(os.path.join(out, 'assembly', 'bom.csv'), 'w', newline='') as fh:
    w = csv.writer(fh)
    w.writerow(['Comment', 'Designator', 'Footprint', 'Manufacturer', 'MPN', 'Quantity'])
    for (comment, fpname, lcsc, mpn, _n), g in groups.items():
        w.writerow([comment, ','.join(g['refs']), fpname, g['mfr'], mpn, len(g['refs'])])
with open(os.path.join(out, 'fab-jlcpcb', 'main-bom-jlcpcb.csv'), 'w', newline='') as fh:
    w = csv.writer(fh)
    w.writerow(['Comment', 'Designator', 'Footprint', 'LCSC Part #', 'Quantity', 'Design MPN', 'Design Manufacturer', 'Part placed (if substituted)', 'Note'])
    for (comment, fpname, lcsc, mpn, _n), g in groups.items():
        w.writerow([comment, ','.join(g['refs']), fpname, lcsc, len(g['refs']), mpn, g['mfr'], g['sub'], g['note']])
n = sum(len(g['refs']) for g in groups.values())
print(f'{len(groups)} BOM lines, {n} placed parts')
for m in missing:
    print('CHECK', m)
