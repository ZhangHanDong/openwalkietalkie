# SPDX-License-Identifier: MIT
import pcbnew, sys, math
b = pcbnew.LoadBoard(sys.argv[1]); mm=lambda v:round(v/1e6,3)
segs=[]
for d in b.GetDrawings():
    if d.GetLayer()!=pcbnew.Edge_Cuts: continue
    t=d.GetShape()
    if t==pcbnew.SHAPE_T_SEGMENT:
        segs.append(('L',(mm(d.GetStart().x),mm(d.GetStart().y)),(mm(d.GetEnd().x),mm(d.GetEnd().y))))
    elif t==pcbnew.SHAPE_T_ARC:
        c=d.GetCenter()
        segs.append(('A',(mm(d.GetStart().x),mm(d.GetStart().y)),(mm(d.GetEnd().x),mm(d.GetEnd().y)),(mm(c.x),mm(c.y)),round(d.GetRadius()/1e6,3),round(d.GetArcAngle().AsDegrees(),1)))
    elif t==pcbnew.SHAPE_T_CIRCLE:
        c=d.GetCenter(); segs.append(('C',(mm(c.x),mm(c.y)),round(d.GetRadius()/1e6,3)))
xs=[];ys=[]
for s in segs:
    for p in s[1:]:
        if isinstance(p,tuple) and len(p)==2: xs.append(p[0]); ys.append(p[1])
print('bbox u',min(xs),max(xs),' v',min(ys),max(ys),' count',len(segs))
# order doesn't matter; print all, flag corners
print('\n== all edge shapes ==')
for s in segs:
    print(s)
