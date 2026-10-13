#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build the Rev B working board from the r6 release (unplaced, unrouted).

Run with KiCad's python. Writes revb/main/main-revb.kicad_pcb.

Changes applied here (netlist/part level only; placement and routing come later):
  * all tracks and vias removed; r6 antenna / U5 rule areas and zones removed
  * In2.Cu becomes a second full GND plane (copy of the In1 zone)
  * J4 deleted; J3 -> Molex 53261-0371 (pin1 CELL, pin2 power::CELL_TEMPERATURE, pin3 GND)
  * R44 10k DNP from CELL_TEMPERATURE (charger TS) to GND (bench use without an NTC)
  * U3 pin 6 (TP_VCC, touch-panel supply) -> IO_3V3 through fitted 0R R48 (C20 100 nF at pin 8 decouples it)
  * R45 100k TOUCH_RESET -> GND
  * R46 100k CHG_STATUS -> IO_3V3, CHG_STATUS also to U5 pin 67 (PA33)
  * AMP_ENABLE PA20 (pin 25) -> PA01 (pin 50); CAL / TP3 PA01 -> PA00 (pin 51); PA20 left free
  * R4/R5 22k -> 100k (reverted to r6's 22k after release by r4r5_22k.py; postreview step 12)
  * RF chain: chip-side pi C52 (shunt, DNP) / C57 15 pF series / C53 (shunt, DNP);
    antenna-side network R40 S1 0R / L6 P1 shunt DNP / R47 S2 0R / C58 P2 shunt DNP
  * MH2/MH4 get a library id (Freerouting SES import needs one)
"""
import os, sys
import pcbnew

S = '/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/67e4cc0e-1314-4924-bb64-e646ad85da32/scratchpad'
R6 = '/Users/tony/projects/conol/openmicrodial/hardware/rev-a/pcb-release-bodyv6.13-20260923-r6/main/main-45deg.kicad_pcb'
OUT = S + '/revb/main/main-revb.kicad_pcb'
FPLIB = S + '/understand/fp/openmicrodial_rev_b.pretty'

mm = pcbnew.FromMM
pt = lambda x, y: pcbnew.VECTOR2I(mm(x), mm(y))

b = pcbnew.LoadBoard(R6)

# ------------------------------------------------------------------ copper
for t in list(b.GetTracks()):
    b.Delete(t)

keep_zones = {'Right screw M2 head/boss keepout', 'MK1 body: no vias',
              'BATTERY_USB_FACE_COMPONENT_RESERVE', 'J3_USB_FACE_PLUG_WITHDRAWAL_RESERVE',
              'In1 continuous GND reference'}
in1 = None
for z in list(b.Zones()):
    name = z.GetZoneName()
    if name == 'In1 continuous GND reference':
        in1 = z
    if name not in keep_zones:
        b.Delete(z)

z2 = pcbnew.ZONE(in1)
z2.SetLayer(pcbnew.In2_Cu)
z2.SetZoneName('In2 continuous GND reference')
b.Add(z2)


def net(name):
    n = b.FindNet(name)
    if n is None:
        n = pcbnew.NETINFO_ITEM(b, name)
        b.Add(n)
        n = b.FindNet(name)
    return n


def fpr(ref):
    f = b.FindFootprintByReference(ref)
    if f is None:
        raise KeyError(ref)
    return f


def clone(src_ref, new_ref, value, pad_nets, at=(60.0, 10.0), dnp=False, fields=None):
    src = fpr(src_ref)
    f = pcbnew.FOOTPRINT(src)
    f.SetReference(new_ref)
    f.SetValue(value)
    f.SetPosition(pt(*at))
    b.Add(f)
    for p in f.Pads():
        p.SetNet(net(pad_nets[p.GetNumber()]))
    if dnp:
        set_dnp(f)
    for k, v in (fields or {}).items():
        f.SetField(k, v)
    return f


def set_dnp(f, on=True):
    f.SetDNP(on)
    f.SetExcludedFromBOM(on)
    f.SetExcludedFromPosFiles(on)


# ------------------------------------------------------------------ battery connector (item 6)
old = fpr('J3')
j3pos, j3rot = old.GetPosition(), old.GetOrientationDegrees()
b.Delete(old)
b.Delete(fpr('J4'))
j3 = pcbnew.FootprintLoad(FPLIB, 'FP_MOLEX_53261_0371')
j3.SetReference('J3')
j3.SetValue('53261-0371')
j3.SetFPID(pcbnew.LIB_ID('openmicrodial_rev_b', 'FP_MOLEX_53261_0371'))
b.Add(j3)
j3.SetPosition(j3pos)
j3.SetOrientationDegrees(j3rot)
for p in j3.Pads():
    n = {'1': 'CELL', '2': 'power::CELL_TEMPERATURE', '3': 'GND'}.get(p.GetNumber())
    if n:
        p.SetNet(net(n))
for m in j3.Models():
    m.m_Filename = '${KIPRJMOD}/../models/53261-0371.step'

clone('R28', 'R44', '10kohm', {'1': 'power::CELL_TEMPERATURE', '2': 'GND'}, dnp=True)

# ------------------------------------------------------------------ display touch supply (item 7)
u3 = fpr('U3')
for p in u3.Pads():
    if p.GetNumber() == '6':
        p.SetNet(net('IO_3V3'))
# fitted 0R so the TP_VCC tie can be undone without a respin (the panel spec calls pin 6 an output)
for p in u3.Pads():
    if p.GetNumber() == '6':
        p.SetNet(net('TP_VCC'))
clone('R40', 'R48', '0ohm', {'1': 'IO_3V3', '2': 'TP_VCC'})

# ------------------------------------------------------------------ carry-overs from the r6 notes
clone('R30', 'R45', '100kohm', {'1': 'TOUCH_RESET', '2': 'GND'})
clone('R30', 'R46', '100kohm', {'1': 'power::CHG_STATUS', '2': 'IO_3V3'})
# AMP_ENABLE PA20 (pin 25, north face) -> PA01 (pin 50, at the SE corner where the amplifier
# lies); CAL test pad TP3 (unused by firmware) PA01 -> PA00 (pin 51, free on r6); pin 25 free
for p in fpr('U5').Pads():
    if p.GetNumber() == '67':
        p.SetNet(net('power::CHG_STATUS'))
    elif p.GetNumber() == '50':
        p.SetNet(net('AMP_ENABLE'))
    elif p.GetNumber() == '51':
        p.SetNet(net('CAL'))
    elif p.GetNumber() == '25':
        p.SetNetCode(0)
for r in ('R4', 'R5'):
    fpr(r).SetValue('100kohm')

# ------------------------------------------------------------------ RF chain (item 3)
# chip-side pi (SiFli reference): C52 shunt DNP, C57 15 pF series, C53 shunt DNP
for p in fpr('C52').Pads():
    p.SetNet(net({'1': 'BT_RF', '2': 'GND'}[p.GetNumber()]))
set_dnp(fpr('C52'))
fpr('C52').SetValue('DNP')
clone('C52', 'C57', '15pF', {'1': 'BT_RF', '2': 'rf::BT_RF_LINE'})
set_dnp(fpr('C57'), False)
for p in fpr('C53').Pads():
    p.SetNet(net({'1': 'rf::BT_RF_LINE', '2': 'GND'}[p.GetNumber()]))
set_dnp(fpr('C53'))
fpr('C53').SetValue('DNP')
# antenna-side network (Johanson order from the line): S1 series, P1 shunt, S2 series, P2 shunt
for p in fpr('R40').Pads():
    p.SetNet(net({'1': 'rf::BT_RF_LINE', '2': 'rf::ANT_MATCH'}[p.GetNumber()]))
clone('C52', 'L6', 'DNP', {'1': 'rf::ANT_MATCH', '2': 'GND'}, dnp=True)
clone('R40', 'R47', '0ohm', {'1': 'rf::ANT_MATCH', '2': 'rf::BT_ANT'})
clone('C52', 'C58', 'DNP', {'1': 'rf::BT_ANT', '2': 'GND'}, dnp=True)
for p in fpr('AE1').Pads():
    if p.GetNumber() == '1':
        p.SetNet(net('rf::BT_ANT'))

# ------------------------------------------------------------------ 3D models / DNP values
def model(ref, fn):
    # iterating Models() yields copies; replace the list instead
    for f in b.GetFootprints():
        if f.GetReference() == ref:
            f.Models().clear()
            m = pcbnew.FP_3DMODEL()
            m.m_Filename = '${KIPRJMOD}/../models/' + fn
            f.Add3DModel(m)
for r in ('C57', 'C52', 'C53', 'C58', 'L6'):
    model(r, 'CC0402JRNPO9BN102.step')
for r in ('R4', 'R5'):
    model(r, 'RC0402FR-07100KL.step')
for r in ('R47', 'R48'):
    model(r, 'RC0402JR-070RL.step')
for r in ('R45', 'R46'):
    model(r, 'RC0402FR-07100KL.step')
model('R44', 'RC0402FR-0710KL.step')
for f in b.GetFootprints():
    if f.GetReference() in ('C52', 'C53', 'C58', 'L6'):
        f.SetValue('NF')

# ------------------------------------------------------------------ mounting holes
for r in ('MH2', 'MH4'):
    fpr(r).SetFPID(pcbnew.LIB_ID('openmicrodial_rev_b', 'MountingHole_2.0mm_NPTH'))

# r6 had no DNP flags; keep everything else fitted
for r in ('R3',):
    pass

# ------------------------------------------------------------------ U5 on the B side: no PTH pads in the footprint
# r6's footprint carries 16 x 0.2 mm PTH 'pads' in the exposed pad; flipped to B they would put
# mask-open GND discs on the battery face. Remove them; the routing stage adds 16 tented board
# vias (filled & capped) at the same positions.
u5 = fpr('U5')
for p in list(u5.Pads()):
    if p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH and p.GetDrillSizeX() > 0:
        u5.Remove(p)
u5.SetFPID(pcbnew.LIB_ID('openmicrodial_rev_b', 'FP_SF32LB52X_QFN68L_BSide'))

b.BuildConnectivity()
pcbnew.SaveBoard(OUT, b)

# clones and swaps copy UUIDs: give every duplicated (uuid ...) a fresh one
import re, uuid
txt = open(OUT).read()
seen = set()
def fresh(m):
    u = m.group(1)
    if u in seen:
        return '(uuid "%s")' % uuid.uuid4()
    seen.add(u)
    return m.group(0)
txt2 = re.sub(r'\(uuid "([0-9a-f-]+)"\)', fresh, txt)
open(OUT, 'w').write(txt2)
print('saved', OUT, 'footprints', len(b.GetFootprints()), 'nets', b.GetNetCount())
