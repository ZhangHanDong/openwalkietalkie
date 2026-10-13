# SPDX-License-Identifier: MIT
import json, math
from shapely.geometry import Polygon, Point, MultiPoint
from shapely.ops import unary_union
def arcpts(s,e,c,r,ang):
    a0=math.atan2(s[1]-c[1],s[0]-c[0]); n=max(10,int(abs(ang)/2))
    return [(c[0]+r*math.cos(math.radians(ang)*i/n+a0), c[1]+r*math.sin(math.radians(ang)*i/n+a0)) for i in range(n+1)]
def densify(segs):
    out=[]
    for s in segs:
        if s[0]=='L': out.append([tuple(s[1]),tuple(s[2])])
        elif s[0]=='A': out.append(arcpts(tuple(s[1]),tuple(s[2]),tuple(s[3]),s[4],s[5]))
    return out
def ring(segs):
    chains=densify(segs); 
    # stitch into one loop
    used=[False]*len(chains); loop=list(chains[0]); used[0]=True
    def close(a,b): return abs(a[0]-b[0])<0.02 and abs(a[1]-b[1])<0.02
    for _ in range(len(chains)):
        tail=loop[-1]; found=False
        for i,ch in enumerate(chains):
            if used[i]: continue
            if close(ch[0],tail): loop+=ch[1:]; used[i]=True; found=True; break
            if close(ch[-1],tail): loop+=ch[::-1][1:]; used[i]=True; found=True; break
        if not found: break
    return Polygon(loop)
old=ring(json.load(open('rt/edge.json'))); new=ring(json.load(open('rt/edge15.json')))
print('old poly valid',old.is_valid,'area',round(old.area,1),'| new valid',new.is_valid,'area',round(new.area,1))
added=new.difference(old); removed=old.difference(new)
print('added material area',round(added.area,3),'mm2  removed',round(removed.area,3),'mm2')
if not added.is_empty:
    b=added.bounds; print('  added region bbox u %.2f..%.2f v %.2f..%.2f'%(b[0],b[2],b[1],b[3]))
# case section points
case=json.load(open('../understand/case_br.json'))['8.8']
casepts=[Point(u,v) for u,v in case]
# how many case points fall inside the ADDED board material (=interference)
hit=[(round(p.x,2),round(p.y,2)) for p in casepts if added.contains(p) or added.distance(p)<0.05]
print('case section points inside/near the ADDED board material:',len(hit))
for h in hit[:12]: print('   HIT',h)
# also: does new board stay clear of the inner boss? find boss = case pts forming small circle near (51,90.5)
boss=[ (u,v) for u,v in case if 49.5<u<52.5 and 89<v<92 and ((u-51.2)**2+(v-90.2)**2)**.5<1.4]
if boss:
    from shapely.geometry import LineString
    bpoly=MultiPoint(boss).convex_hull
    print('boss hull bounds',[round(x,2) for x in bpoly.bounds],'-> new board covers boss?',new.contains(bpoly.centroid))
