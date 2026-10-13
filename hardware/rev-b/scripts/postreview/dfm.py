# SPDX-License-Identifier: MIT
# KiCad python: DFM items from the review.
import pcbnew
FM = pcbnew.FromMM; MM = pcbnew.ToMM
b = pcbnew.LoadBoard('IN.kicad_pcb')
for ref in ('C52', 'C53', 'C58', 'L6', 'R44'):                      # no paste on DNP pads
    for p in b.FindFootprintByReference(ref).Pads():
        ls = p.GetLayerSet(); ls.RemoveLayer(pcbnew.F_Paste); ls.RemoveLayer(pcbnew.B_Paste); p.SetLayerSet(ls)
u5 = b.FindFootprintByReference('U5')                                # pin-1 dot 0.05 mm further out
for g in u5.GraphicalItems():
    if g.GetClass() == 'PCB_SHAPE' and g.GetShape() == pcbnew.SHAPE_T_CIRCLE and g.GetLayer() in (pcbnew.B_SilkS, pcbnew.F_SilkS):
        c = g.GetCenter()
        if abs(MM(c.x) - 35.7) < 1.0 and MM(c.y) > 27.4:
            g.Move(pcbnew.VECTOR2I(0, FM(0.05)))
for f in b.GetFootprints():                                          # TP/MH not in CPL/BOM
    if f.GetReference().startswith(('TP', 'MH')):
        f.SetExcludedFromPosFiles(True); f.SetExcludedFromBOM(True)
tb = b.GetTitleBlock(); tb.SetTitle('OpenWalkieTalkie main board'); tb.SetRevision('B'); tb.SetDate('2026-10-05')
for t in b.GetTracks():                                              # VCC (pin 24) link to C28: 0.2 mm where it fits
    if t.GetNetname() == 'SYSTEM_SUPPLY' and t.GetClass() != 'PCB_VIA' and t.GetLayer() == pcbnew.F_Cu and MM(t.GetWidth()) < 0.13:
        xs = [MM(t.GetStart().x), MM(t.GetEnd().x)]; ys = [MM(t.GetStart().y), MM(t.GetEnd().y)]
        if 37.9 < min(xs) and max(xs) < 39.5 and 18.2 < min(ys) and max(ys) < 20.5 and not (abs(xs[0] - 38.05) < 0.01 and abs(xs[1] - 38.05) < 0.01):
            t.SetWidth(FM(0.2))
for text, (x, y), h in (('OpenWalkieTalkie main Rev B', (17.0, 49.3), 0.9), ('2026-10', (17.0, 50.6), 0.8)):
    t = pcbnew.PCB_TEXT(b); t.SetText(text); t.SetLayer(pcbnew.F_SilkS)
    t.SetPosition(pcbnew.VECTOR2I(FM(x), FM(y))); t.SetTextSize(pcbnew.VECTOR2I(FM(h), FM(h))); t.SetTextThickness(FM(0.15)); b.Add(t)
pcbnew.SaveBoard('OUT.kicad_pcb', b)
