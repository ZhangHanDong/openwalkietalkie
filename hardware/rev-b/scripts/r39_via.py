# SPDX-License-Identifier: MIT
"""Restore R39.2's GND via at (33.22, 30.0) and keep In2 from filling the
0.11 mm^2 sliver it would create between USB_MCU_P/N (In2 no-fill rule area).
usage: r39_via.py IN OUT"""
import pcbnew, sys
b = pcbnew.LoadBoard(sys.argv[1])
mm = pcbnew.FromMM
ref = next(t for t in b.GetTracks() if t.GetClass() == 'PCB_VIA' and t.GetNetname() == 'GND'
           and abs(t.GetPosition().x - mm(33.22)) < 1000 and abs(t.GetPosition().y - mm(28.9)) < 1000)
v = pcbnew.PCB_VIA(b)
v.SetPosition(pcbnew.VECTOR2I(mm(33.22), mm(30.0)))
v.SetWidth(ref.GetWidth()); v.SetDrill(ref.GetDrill())
v.SetNet(ref.GetNet()); v.SetLocked(True)
b.Add(v)
z = pcbnew.ZONE(b)
z.SetIsRuleArea(True); z.SetZoneName('In2 no fill: USB pair at R39 via')
z.SetLayer(pcbnew.In2_Cu)
z.SetDoNotAllowZoneFills(True); z.SetDoNotAllowTracks(False); z.SetDoNotAllowVias(False)
z.SetDoNotAllowPads(False); z.SetDoNotAllowFootprints(False)
o = z.Outline(); o.NewOutline()
for x, y in ((32.90, 29.78), (33.56, 29.78), (33.56, 30.22), (32.90, 30.22)):
    o.Append(mm(x), mm(y))
b.Add(z)
b.Save(sys.argv[2])
print('via', round(ref.GetWidth()/1e6, 3), round(ref.GetDrill()/1e6, 3))
