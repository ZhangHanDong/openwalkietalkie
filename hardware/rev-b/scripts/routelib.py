# SPDX-License-Identifier: MIT
"""Scripted routing helpers (KiCad python).

R = Router(board)
R.path(net, layer, [(u,v), (u,v), ...], width)      polyline of tracks
R.via(net, (u,v), d=0.45, drill=0.2)                 through via
R.pad(ref, num) -> (u, v)                            pad centre
R.pv(ref, num, du=0, dv=0) -> (u, v)                 pad centre + offset
All items are locked (they are the fixed skeleton for the autorouter).
"""
import pcbnew

mm = pcbnew.FromMM
MM = pcbnew.ToMM
LAY = {'F': pcbnew.F_Cu, 'B': pcbnew.B_Cu, 'In1': pcbnew.In1_Cu, 'In2': pcbnew.In2_Cu}


class Router:
    def __init__(self, b, lock=True):
        self.b = b
        self.lock = lock
        self.n_tracks = 0
        self.n_vias = 0

    def net(self, name):
        n = self.b.FindNet(name)
        if n is None:
            raise KeyError('net ' + name)
        return n

    def pad(self, ref, num):
        f = self.b.FindFootprintByReference(ref)
        for p in f.Pads():
            if p.GetNumber() == str(num):
                return (MM(p.GetPosition().x), MM(p.GetPosition().y))
        raise KeyError(f'{ref}.{num}')

    def pv(self, ref, num, du=0.0, dv=0.0):
        u, v = self.pad(ref, num)
        return (round(u + du, 4), round(v + dv, 4))

    def padnet(self, ref, num):
        f = self.b.FindFootprintByReference(ref)
        for p in f.Pads():
            if p.GetNumber() == str(num):
                return p.GetNetname()
        raise KeyError(f'{ref}.{num}')

    def path(self, net, layer, pts, w=0.12):
        n = self.net(net)
        for (u0, v0), (u1, v1) in zip(pts[:-1], pts[1:]):
            if abs(u0 - u1) < 1e-6 and abs(v0 - v1) < 1e-6:
                continue
            t = pcbnew.PCB_TRACK(self.b)
            t.SetStart(pcbnew.VECTOR2I(mm(u0), mm(v0)))
            t.SetEnd(pcbnew.VECTOR2I(mm(u1), mm(v1)))
            t.SetWidth(mm(w))
            t.SetLayer(LAY[layer])
            t.SetNet(n)
            t.SetLocked(self.lock)
            self.b.Add(t)
            self.n_tracks += 1

    def via(self, net, p, d=0.45, drill=0.2):
        v = pcbnew.PCB_VIA(self.b)
        v.SetPosition(pcbnew.VECTOR2I(mm(p[0]), mm(p[1])))
        v.SetWidth(mm(d))
        v.SetDrill(mm(drill))
        v.SetNet(self.net(net))
        v.SetLocked(self.lock)
        self.b.Add(v)
        self.n_vias += 1
        return v

    def delete_net(self, names):
        names = set(names)
        for t in list(self.b.GetTracks()):
            if t.GetNetname() in names:
                self.b.Delete(t)
