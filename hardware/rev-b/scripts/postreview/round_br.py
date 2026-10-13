# SPDX-License-Identifier: MIT
"""Make the bottom-right board corner match the top-left: replace its sharp-framed
corner chain with a 180-degree-rotated copy of the top-left corner chain (about the
board centre 28.0, 47.5). Keeps the 2.0 mm case locating lobe, drops the sharp cuts.
usage: round_br.py IN OUT"""
import pcbnew, sys
b = pcbnew.LoadBoard(sys.argv[1]); mm=lambda v:v/1e6
CX,CY = pcbnew.FromMM(28.0), pcbnew.FromMM(47.5)
def rot(p):  # 180 deg about (CX,CY)
    return pcbnew.VECTOR2I(2*CX - p.x, 2*CY - p.y)
inreg = lambda p, u0,v0: (p.x>=pcbnew.FromMM(u0) and p.y>=pcbnew.FromMM(v0))
intl  = lambda p: (p.x<=pcbnew.FromMM(8.1) and p.y<=pcbnew.FromMM(8.6))
edges=[d for d in b.GetDrawings() if d.GetLayer()==pcbnew.Edge_Cuts]
width = edges[0].GetWidth()
tl=[]; killed=0; botedge=None
for d in list(edges):
    s,e=d.GetStart(),d.GetEnd()
    if intl(s) and intl(e):              # top-left corner chain -> remember
        tl.append(d)
    if inreg(s,47.4,86.4) and inreg(e,47.4,86.4):   # bottom-right corner -> delete
        b.Remove(d); killed+=1
    # the bottom edge: (29.41,93.69)-(47.5,93.69)
    if abs(s.y-pcbnew.FromMM(93.69))<2000 and abs(e.y-pcbnew.FromMM(93.69))<2000 \
       and min(mm(s.x),mm(e.x))<40 and 47.0<max(mm(s.x),mm(e.x))<47.9:
        botedge=d
print('top-left chain shapes:',len(tl),' deleted BR shapes:',killed,' bottom edge found:',botedge is not None)
# re-point the bottom edge end from (47.5,93.69) to the rotated top-edge junction (47.987,93.689)
J=rot(pcbnew.VECTOR2I(pcbnew.FromMM(8.013),pcbnew.FromMM(1.311)))
if botedge:
    if mm(botedge.GetStart().x)>mm(botedge.GetEnd().x): botedge.SetStart(J)
    else: botedge.SetEnd(J)
    print('  bottom edge end ->', round(mm(J.x),3), round(mm(J.y),3))
# add rotated copies of the top-left chain
for d in tl:
    n=pcbnew.PCB_SHAPE(b); n.SetLayer(pcbnew.Edge_Cuts); n.SetWidth(width)
    if d.GetShape()==pcbnew.SHAPE_T_ARC:
        n.SetShape(pcbnew.SHAPE_T_ARC)
        n.SetArcGeometry(rot(d.GetStart()), rot(d.GetArcMid()), rot(d.GetEnd()))
    else:
        n.SetShape(pcbnew.SHAPE_T_SEGMENT); n.SetStart(rot(d.GetStart())); n.SetEnd(rot(d.GetEnd()))
    b.Add(n)
# closure check
pts={}
for d in [x for x in b.GetDrawings() if x.GetLayer()==pcbnew.Edge_Cuts]:
    for p in (d.GetStart(),d.GetEnd()):
        k=(round(mm(p.x),2),round(mm(p.y),2)); pts[k]=pts.get(k,0)+1
odd=[k for k,c in pts.items() if c%2]
print('open endpoints (should be none):',odd[:8])
b.Save(sys.argv[2]); print('saved',sys.argv[2])
