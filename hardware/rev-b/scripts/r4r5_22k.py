# SPDX-License-Identifier: MIT
"""R4/R5 back to r6's 22k (RC0402FR-0722KL). They stay at the NS4150B input pins; only the
value change is reverted (the r6 hiss was flash XIP clicks at the summing node, set by the
amplifier's feedback, not by R_in; see rev-a README).
usage: r4r5_22k.py IN OUT"""
import pcbnew, sys
b = pcbnew.LoadBoard(sys.argv[1])
for f in b.GetFootprints():
    if f.GetReference() in ('R4', 'R5'):
        f.SetValue('22kohm')
        f.Models().clear()
        m = pcbnew.FP_3DMODEL(); m.m_Filename = '${KIPRJMOD}/../models/RC0402FR-0722KL.step'
        f.Add3DModel(m)
        print(f.GetReference(), f.GetValue(), [x.m_Filename for x in f.Models()])
b.Save(sys.argv[2])
