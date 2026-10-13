# SPDX-License-Identifier: MIT
"""KiCad 10.0.4 pcbnew helpers (tested 2026-10-05 against the r6 main board).

Run with KiCad's bundled python:
  KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
  $KPY -c "import sys; sys.path.insert(0, '<this dir>'); import kicad_helpers as kh; ..."
(KiCad prints a harmless 'wxApp' assert line on import; ignore it.)

Units: KiCad internal units are nanometres (int). Everything here takes mm.
Board frame: (u, v) mm == KiCad (x, y) mm, y grows DOWN (screen frame).
"""
import os
import time

import pcbnew

KICAD_CLI = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"

# ---------------------------------------------------------------- basics

def mm(v):
    """mm -> internal units (int nm)."""
    return pcbnew.FromMM(float(v))


def to_mm(v):
    return pcbnew.ToMM(v)


def pt(x, y):
    """Point in mm -> VECTOR2I."""
    return pcbnew.VECTOR2I(mm(x), mm(y))


def load(path):
    """Load a .kicad_pcb (the .kicad_pro next to it is read too: netclasses,
    DRC severities, patterns)."""
    return pcbnew.LoadBoard(path)


def save(board, path, skip_settings=False):
    """Save the board. With skip_settings=False KiCad also writes the
    .kicad_pro/.kicad_prl next to the new path (netclasses etc. go along when
    you save to a new directory)."""
    ok = pcbnew.SaveBoard(path, board, skip_settings)
    if not ok:
        raise RuntimeError("SaveBoard failed: " + path)
    return path


def fp(board, ref):
    f = board.FindFootprintByReference(ref)
    if f is None:
        raise KeyError(ref)
    return f


def layer_id(board, name):
    """'F.Cu' -> PCB_LAYER_ID (board.GetLayerID also accepts user names)."""
    return board.GetLayerID(name)


# ---------------------------------------------------------------- footprints

def move_fp(board, ref, x, y, rot_deg=None, unlock=False):
    """Absolute move of a footprint origin to (x, y) mm; optional absolute
    rotation in degrees (KiCad angle, CCW positive as seen from the FRONT,
    the same number the GUI shows, also for back-side parts).
    Locked footprints can be moved from python; `unlock` only clears the flag."""
    f = fp(board, ref)
    if unlock:
        f.SetLocked(False)
    f.SetPosition(pt(x, y))
    if rot_deg is not None:
        f.SetOrientationDegrees(float(rot_deg))
    return f


def flip_fp(board, ref, to_side=None, direction="TB", keep_angle=None):
    """Flip a footprint to the other copper side about its own origin.

    KiCad 10 API: FOOTPRINT.Flip(VECTOR2I centre, FLIP_DIRECTION)
      FLIP_DIRECTION_TOP_BOTTOM  - mirror about the horizontal line through centre:
                                   pad y mirrored, angle' = -angle. This is what the
                                   GUI 'F' key does here (pcbnew.json editing.flip_left_right=false)
      FLIP_DIRECTION_LEFT_RIGHT  - mirror about the vertical line through centre:
                                   pad x mirrored, angle' = 180 - angle
      (measured on r6: U6 rot 0 -> LR 180 / TB 0; SW5 rot 90 -> LR 90 / TB -90)
    to_side: 'F' / 'B' / None (None = always toggle; 'F'/'B' = no-op if already there)
    keep_angle: if given, set this absolute KiCad orientation after the flip.
    Returns the footprint. Pads, graphics, fields and 3D model follow.
    """
    f = fp(board, ref)
    on_back = f.GetLayer() == pcbnew.B_Cu
    if to_side == "F" and not on_back:
        return f
    if to_side == "B" and on_back:
        return f
    d = pcbnew.FLIP_DIRECTION_LEFT_RIGHT if direction.upper() == "LR" else pcbnew.FLIP_DIRECTION_TOP_BOTTOM
    f.Flip(f.GetPosition(), d)
    if keep_angle is not None:
        f.SetOrientationDegrees(float(keep_angle))
    return f


def fp_info(f):
    p = f.GetPosition()
    return {
        "ref": f.GetReference(),
        "side": "B" if f.GetLayer() == pcbnew.B_Cu else "F",
        "x": round(to_mm(p.x), 4),
        "y": round(to_mm(p.y), 4),
        "rot": round(f.GetOrientationDegrees(), 3),
        "locked": f.IsLocked(),
        "pads": {pd.GetNumber(): (round(to_mm(pd.GetPosition().x), 4), round(to_mm(pd.GetPosition().y), 4),
                                   pd.GetNetname()) for pd in f.Pads()},
    }


# ---------------------------------------------------------------- tracks / vias

def tracks_of(board, nets=None, kinds=("PCB_TRACK", "PCB_ARC", "PCB_VIA")):
    nets = set(nets) if nets is not None else None
    out = []
    for t in board.GetTracks():
        if t.GetClass() not in kinds:
            continue
        if nets is None or t.GetNetname() in nets:
            out.append(t)
    return out


def _inside(t, w):
    bb = t.GetBoundingBox()
    u0, v0, u1, v1 = w
    return (to_mm(bb.GetLeft()) >= u0 and to_mm(bb.GetRight()) <= u1 and
            to_mm(bb.GetTop()) >= v0 and to_mm(bb.GetBottom()) <= v1)


def delete_tracks(board, nets, include_vias=True, keep_locked=False, window=None, keep_windows=()):
    """Delete tracks (and vias) of the given net names. window=(u0,v0,u1,v1) mm
    restricts to items whose bbox lies inside; keep_windows=[(u0,v0,u1,v1),...]
    spares items lying fully inside any of them (e.g. hand-made QFN escapes +
    fan-out vias that the autorouter should start from). Returns number deleted.
    board.Delete(item) removes and frees; do not touch `item` afterwards."""
    kinds = ("PCB_TRACK", "PCB_ARC", "PCB_VIA") if include_vias else ("PCB_TRACK", "PCB_ARC")
    victims = []
    for t in tracks_of(board, nets, kinds):
        if keep_locked and t.IsLocked():
            continue
        if window and not _inside(t, window):
            continue
        if any(_inside(t, w) for w in keep_windows):
            continue
        victims.append(t)
    for t in victims:
        board.Delete(t)
    return len(victims)


def set_tracks_locked(board, locked=True, nets=None, exclude_nets=()):
    n = 0
    ex = set(exclude_nets)
    for t in tracks_of(board, nets):
        if t.GetNetname() in ex:
            continue
        t.SetLocked(locked)
        n += 1
    return n


def add_track(board, net, layer, pts_mm, width_mm):
    """Polyline of PCB_TRACK segments on one layer. pts_mm = [(x,y), ...]."""
    ni = board.FindNet(net)
    lid = layer_id(board, layer) if isinstance(layer, str) else layer
    made = []
    for (x0, y0), (x1, y1) in zip(pts_mm, pts_mm[1:]):
        t = pcbnew.PCB_TRACK(board)
        t.SetStart(pt(x0, y0))
        t.SetEnd(pt(x1, y1))
        t.SetWidth(mm(width_mm))
        t.SetLayer(lid)
        t.SetNet(ni)
        board.Add(t)
        made.append(t)
    return made


def add_via(board, net, x, y, dia_mm=0.45, drill_mm=0.2):
    """Through via (r6 standard is 0.45/0.20; the power vias are 0.6/0.3)."""
    v = pcbnew.PCB_VIA(board)
    v.SetViaType(pcbnew.VIATYPE_THROUGH)
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
    v.SetPosition(pt(x, y))
    v.SetWidth(mm(dia_mm))          # KiCad 10: SetWidth(int) sets all layers; SetWidth(layer, int) per layer
    v.SetDrill(mm(drill_mm))
    v.SetNet(board.FindNet(net))
    board.Add(v)
    return v


# ---------------------------------------------------------------- zones

def fill_zones(board):
    """Refill all copper zones (rule areas are skipped by the filler)."""
    t0 = time.time()
    filler = pcbnew.ZONE_FILLER(board)
    ok = filler.Fill(board.Zones())
    return ok, time.time() - t0


def unfill_zones(board):
    for z in board.Zones():
        if not z.GetIsRuleArea():
            z.UnFill()


# ---------------------------------------------------------------- nets / rules

def netclass_of(board, net):
    """Effective NETCLASS object for a net name (patterns resolved)."""
    return board.GetDesignSettings().m_NetSettings.GetEffectiveNetClass(net)


def rules_summary(board):
    bds = board.GetDesignSettings()
    ns = bds.m_NetSettings
    out = {k: round(to_mm(getattr(bds, k)), 4) for k in (
        "m_MinClearance", "m_TrackMinWidth", "m_ViasMinSize", "m_MinThroughDrill",
        "m_CopperEdgeClearance", "m_HoleClearance", "m_HoleToHoleMin", "m_ViasMinAnnularWidth")}
    ncs = {}
    for name, nc in list(ns.GetNetclasses().items()) + [("Default", ns.GetDefaultNetclass())]:
        ncs[str(name)] = {"clearance": to_mm(nc.GetClearance()), "track": to_mm(nc.GetTrackWidth()),
                          "via": to_mm(nc.GetViaDiameter()), "drill": to_mm(nc.GetViaDrill()),
                          "priority": nc.GetPriority()}
    out["netclasses"] = ncs
    return out


def stats(board):
    tr = list(board.GetTracks())
    return {
        "footprints": len(board.GetFootprints()),
        "tracks": sum(t.GetClass() == "PCB_TRACK" for t in tr),
        "arcs": sum(t.GetClass() == "PCB_ARC" for t in tr),
        "vias": sum(t.GetClass() == "PCB_VIA" for t in tr),
        "locked_tracks": sum(t.IsLocked() for t in tr),
        "zones": sum(not z.GetIsRuleArea() for z in board.Zones()),
        "rule_areas": sum(z.GetIsRuleArea() for z in board.Zones()),
        "unrouted_connections": unrouted_count(board),
    }


def unrouted_count(board):
    """Ratsnest count from the in-memory connectivity (no DRC). Needs
    BuildConnectivity after edits."""
    board.BuildConnectivity()
    return board.GetConnectivity().GetUnconnectedCount(False)
