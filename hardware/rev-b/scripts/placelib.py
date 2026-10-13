# SPDX-License-Identifier: MIT
"""Placement helpers for Rev B (KiCad python).

place(b, ref, side, x, y, rot)          absolute placement; flips TOP_BOTTOM when the side changes
place2(b, ref, side, x, y, toward)      2-pad part: picks rot in {0,90,180,270} so the pad that
                                        shares a net with `toward` (ref.pad or (x,y)) is nearest to it
swap_fp(b, ref, src_ref, model=None)    replace ref's footprint geometry with src_ref's (keeps ref,
                                        value, pad nets by pad number); used for 0603 -> 0402
"""
import math
import pcbnew

mm = pcbnew.FromMM
MM = pcbnew.ToMM
pt = lambda x, y: pcbnew.VECTOR2I(mm(x), mm(y))


def fpr(b, ref):
    f = b.FindFootprintByReference(ref)
    if f is None:
        raise KeyError(ref)
    return f


def set_side(f, side):
    on_b = f.GetLayer() == pcbnew.B_Cu
    if (side == 'B') != on_b:
        f.Flip(f.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)


def place(b, ref, side, x, y, rot):
    f = fpr(b, ref)
    set_side(f, side)
    f.SetPosition(pt(x, y))
    f.SetOrientationDegrees(float(rot))
    return f


def padxy(b, ref, num):
    f = fpr(b, ref)
    for p in f.Pads():
        if p.GetNumber() == str(num):
            return MM(p.GetPosition().x), MM(p.GetPosition().y), p.GetNetname()
    raise KeyError(f'{ref}.{num}')


def place2(b, ref, side, x, y, toward, rots=(0, 90, 180, 270)):
    """toward: 'U5.12' style pad spec or (x, y). The pad of `ref` whose net equals the
    target pad's net is aimed at it; with a point target, pad 1 is aimed."""
    f = place(b, ref, side, x, y, 0)
    if isinstance(toward, str):
        tr, tn = toward.split('.')
        tx, ty, tnet = padxy(b, tr, tn)
    else:
        tx, ty = toward
        tnet = None
    best = None
    for r in rots:
        f.SetOrientationDegrees(float(r))
        for p in f.Pads():
            if (tnet is None and p.GetNumber() == '1') or (tnet is not None and p.GetNetname() == tnet):
                d = math.hypot(MM(p.GetPosition().x) - tx, MM(p.GetPosition().y) - ty)
                if best is None or d < best[0] - 1e-6:
                    best = (d, r)
    f.SetOrientationDegrees(float(best[1] if best else 0))
    return f


def swap_fp(b, ref, src_ref, model=None):
    """Replace footprint `ref` with a copy of `src_ref`'s footprint (same pad numbers),
    keeping reference, value, fields, side, position, orientation and pad nets."""
    old = fpr(b, ref)
    src = fpr(b, src_ref)
    nets = {p.GetNumber(): p.GetNetname() for p in old.Pads()}
    side = 'B' if old.GetLayer() == pcbnew.B_Cu else 'F'
    pos, rot, val = old.GetPosition(), old.GetOrientationDegrees(), old.GetValue()
    dnp = old.IsDNP()
    new = pcbnew.FOOTPRINT(src)
    b.Delete(old)
    new.SetReference(ref)
    new.SetValue(val)
    b.Add(new)
    set_side(new, side)
    new.SetPosition(pos)
    new.SetOrientationDegrees(rot)
    for p in new.Pads():
        p.SetNet(b.FindNet(nets.get(p.GetNumber(), '')) if nets.get(p.GetNumber()) else b.FindNet(''))
    if model:
        new.Models().clear()
        m = pcbnew.FP_3DMODEL()
        m.m_Filename = model
        new.Add3DModel(m)
    new.SetDNP(dnp)
    return new


def dedupe_uuids(path):
    """Footprint copies keep their source's UUIDs; give every repeated (uuid ...) a fresh one."""
    import re, uuid
    txt = open(path).read()
    seen = set()

    def fresh(m):
        u = m.group(1)
        if u in seen:
            return '(uuid "%s")' % uuid.uuid4()
        seen.add(u)
        return m.group(0)
    open(path, 'w').write(re.sub(r'\(uuid "([0-9a-f-]+)"\)', fresh, txt))
