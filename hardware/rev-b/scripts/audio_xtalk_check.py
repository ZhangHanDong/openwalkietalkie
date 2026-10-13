#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Audio crosstalk / placement checker for the OpenWalkieTalkie main board.

Run with KiCad's bundled python (has pcbnew + numpy):

  KPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
  $KPY audio_xtalk_check.py BOARD.kicad_pcb [--rules rules.json] [--md out.md] [--json out.json]
                            [--amp U1] [--mcu U5] [--flash U4] [--refill] [--quiet]

Exit status: 0 = no ERROR findings, 1 = at least one ERROR, 2 = usage / load problem.

What it checks (all distances in mm, measured edge-to-edge on copper, projected
onto the board plane for different layers):

  XT-SAME   victim and aggressor copper on the same layer closer than the class
            minimum gap, or running in parallel (gap < par_gap) for more than
            par_len.
  XT-ADJ    victim and aggressor on different layers with NO GND fill on any
            layer between them at that spot (e.g. In2.Cu vs B.Cu in the r6
            stackup, 0.0994 mm apart), projected gap < adj_gap (negative =
            broadside overlap / crossing).
  XT-SHLD   crossing on different layers that IS separated by a GND fill.
            Reported as INFO for line-level nets, ERROR for the summing node
            (it should be too short to cross anything).
  SUM-GEOM  summing node (AMP_INP/AMP_INN) geometry: Rin pad to amplifier pin
            centre distance, routed length, vias, layers.
  REF       fraction of each victim track whose nearest copper layer in the
            stack is GND fill under the track (+margin).
  PAIR      differential pairs (DAC, DAC_FILTER, AC, AMP_IN, AMP_VO, SPEAKER):
            length mismatch, layer mismatch, p95 / max P-to-N separation.
  SUPPLY    AVDD33_AUD (MCU pin 38) sharing a net with the external flash VCC;
            decoupling cap distance for AVDD33_AUD / AUD_VREF.
  CLASSD    output ferrites / caps distance from the amplifier output pins.

Victim / aggressor classes are net-name regexes (see DEFAULT_RULES); a JSON file
passed with --rules is merged over the defaults (top-level keys replaced).
"""
import sys, re, json, math, argparse
from collections import defaultdict

import numpy as np
import pcbnew

# ----------------------------------------------------------------------------
# Rules
# ----------------------------------------------------------------------------
DEFAULT_RULES = {
    "victims": {
        # high-impedance summing node: injected current x Rf (240-300 kOhm)
        "S0_summing": [r"^(audio::)?AMP_IN[PN]$"],
        # line-level analog, ~1 kOhm node impedance
        "S1_analog": [r"^(audio::)?DAC_[PN]$", r"^(audio::)?DAC_FILTER_[PN]$",
                      r"^(audio::)?AC_[PN]$", r"^(audio::)?AMP_BYPASS$",
                      r"^(audio::)?AUDIO_REF$", r"^(audio::)?AUD_VREF$", r"^(audio::)?ADC_REF$"],
        # analog supply (AVDD33_AUD)
        "S2_supply": [r"^MCU_LDO1$", r"^(audio::)?AVDD33_AUD$", r"^(audio::)?AUDIO_SUPPLY$"],
    },
    "aggressors": {
        "A_fast": [r"^FLASH_", r"^PANEL_(CLK|CS|D[0-3])$", r"^USB_", r"^XTAL48_",
                   r"^MIC_PDM_", r"^(audio::)?MIC_CLOCK$", r"^(rf::)?BT_", r"^PSRAM_"],
        "A_power": [r"^BUCK_LX$", r"^power::.*_LX\d*$", r"_LX\d*$", r"_SW$"],
        "A_classd": [r"^(audio::)?AMP_VO[PN]$", r"^(audio::)?SPEAKER_"],
        "A_slow": [r"(^|::)I2C_", r"_(SDA|SCL)$", r"^PANEL_TE$", r"^DEBUG_", r"^TOUCH_",
                   r"^ENC_", r"^PANEL_RESET$", r"(^|::)CHG_INT"],
    },
    # per (victim class, aggressor class): same-layer min gap, parallel gap,
    # max parallel length, unshielded different-layer min projected gap,
    # severity of a shielded crossing (None = not reported)
    "limits": {
        "S0_summing": {
            "A_fast":   {"same": 1.0, "par_gap": 1.5, "par_len": 0.0, "adj": 1.0, "shielded": "ERROR"},
            "A_power":  {"same": 1.5, "par_gap": 2.0, "par_len": 0.0, "adj": 1.5, "shielded": "ERROR"},
            "A_classd": {"same": 1.0, "par_gap": 1.5, "par_len": 0.0, "adj": 1.0, "shielded": "ERROR"},
            "A_slow":   {"same": 0.5, "par_gap": 1.0, "par_len": 0.5, "adj": 0.5, "shielded": "WARN", "sev": "WARN"},
        },
        "S1_analog": {
            "A_fast":   {"same": 0.5, "par_gap": 0.75, "par_len": 5.0, "adj": 0.5, "shielded": "INFO"},
            "A_power":  {"same": 1.0, "par_gap": 1.5, "par_len": 1.0, "adj": 1.0, "shielded": "WARN"},
            "A_classd": {"same": 0.5, "par_gap": 0.75, "par_len": 5.0, "adj": 0.5, "shielded": "INFO"},
            "A_slow":   {"same": 0.25, "par_gap": 0.5, "par_len": 5.0, "adj": 0.25, "shielded": None, "sev": "WARN"},
        },
        "S2_supply": {
            "A_fast":   {"same": 0.2, "par_gap": 0.3, "par_len": 5.0, "adj": 0.0, "shielded": None, "sev": "WARN"},
            "A_power":  {"same": 0.3, "par_gap": 0.5, "par_len": 3.0, "adj": 0.3, "shielded": None, "sev": "WARN"},
            "A_classd": {"same": 0.2, "par_gap": 0.3, "par_len": 5.0, "adj": 0.0, "shielded": None, "sev": "WARN"},
        },
    },
    # nets whose own pins legitimately sit next to an aggressor pin (pin-field
    # escape at the MCU); proximity *inside* this radius of the listed pads is
    # reported as INFO instead of ERROR.
    "pin_escape_radius": 1.2,
    "pin_escape_refs": ["MCU"],          # "MCU" = the --mcu reference
    "sample_step": 0.025,
    "shield_margin": 0.20,     # GND fill must cover a point by this much
    "ref_margin": 0.15,        # reference plane must extend this far past the track edge
    "ref_min_fraction": {"S0_summing": 0.95, "S1_analog": 0.90},
    "summing": {"max_pad_dist": 2.5, "max_len": 3.0, "max_vias": 0},
    "pairs": [["DAC_P", "DAC_N"], ["DAC_FILTER_P", "DAC_FILTER_N"], ["AC_P", "AC_N"],
              ["AMP_INP", "AMP_INN"], ["AMP_VOP", "AMP_VON"], ["SPEAKER_PLUS", "SPEAKER_MINUS"]],
    "pair_max_p95_sep": 1.0,     # mm, P centreline to nearest N copper (95th percentile)
    "pair_max_mismatch": 1.5,    # mm or 25 %, whichever is larger
    "classd_max_dist": 3.5,      # amp output pin centre to ferrite pad centre
    "decap_max_dist": 1.5,       # MCU audio pin pad centre to its nearest decap pad centre
    "mcu_audio_pins": {"38": "AVDD33_AUD", "39": "AUD_VREF"},
    "flash_vcc_pin": "8",
    "max_sites": 400,
    "amp_vdd_pin": "6",
    "amp_gnd_pin": "7",
    "amp_decap_max_dist": 2.0,      # VDD pad centre to decap pad centre
    "amp_gnd_via_max_dist": 0.8,
    # coupling budget (fF, estimated with the xsec tables); r6 measured with this script:
    # AMP_INN 920 fF fast + 163 classd, AMP_INP 273 + 392, weighted total ~300 fF*Mohm
    "budget": {
        "S0_summing": {"fast_warn": 2.0, "fast_error": 5.0, "classd_warn": 10.0},
        "S1_analog":  {"fast_warn": 25.0, "fast_error": 60.0, "classd_warn": 60.0},
        "weighted_total_max": 10.0,
        "r6_weighted_total": 307.8,
    },
}


def ToMM(v):
    return v / 1e6


def cls_of(name, table):
    for c, pats in table.items():
        for p in pats:
            if re.search(p, name):
                return c
    return None


def short(n):
    return n.split("::")[-1]


# ----------------------------------------------------------------------------
# Geometry helpers (numpy)
# ----------------------------------------------------------------------------
def pt_seg(px, py, ax, ay, bx, by):
    """px,py (P,1); a,b (1,N) -> distance (P,N), nearest point cx,cy."""
    dx = bx - ax
    dy = by - ay
    L2 = dx * dx + dy * dy
    safe = np.where(L2 > 0, L2, 1.0)
    t = ((px - ax) * dx + (py - ay) * dy) / safe
    t = np.clip(t, 0.0, 1.0)
    t = np.where(L2 > 0, t, 0.0)
    cx = ax + t * dx
    cy = ay + t * dy
    return np.hypot(px - cx, py - cy), cx, cy


def seg_seg_intersect(p1, p2, q1, q2):
    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    d1 = orient(q1, q2, p1); d2 = orient(q1, q2, p2)
    d3 = orient(p1, p2, q1); d4 = orient(p1, p2, q2)
    return (d1 * d2 < 0) and (d3 * d4 < 0)


class Prims:
    """Copper primitives of one (net, layer): capsules + polygons."""
    def __init__(self):
        self.caps = []      # (ax, ay, bx, by, r, item_kind)
        self.polys = []     # Nx2 arrays (closed implicitly)

    def finalize(self):
        self.C = np.array([c[:5] for c in self.caps], dtype=float).reshape(-1, 5)
        self.barrel = np.array([c[5] in ("via", "pth") for c in self.caps], dtype=bool)
        self.has_planar = bool(len(self.polys)) or bool((~self.barrel).any())
        edges = []
        pid = []
        for i, P in enumerate(self.polys):
            Q = np.roll(P, -1, axis=0)
            edges.append(np.hstack([P, Q]))
            pid.extend([i] * len(P))
        self.E = np.vstack(edges) if edges else np.zeros((0, 4))
        self.Epid = np.array(pid, dtype=int)
        self.npoly = len(self.polys)
        allpts = [self.C[:, 0:2], self.C[:, 2:4]] + [P for P in self.polys]
        if any(len(a) for a in allpts):
            A = np.vstack([a for a in allpts if len(a)])
            rmax = self.C[:, 4].max() if len(self.C) else 0
            self.bbox = (A[:, 0].min() - rmax, A[:, 1].min() - rmax, A[:, 0].max() + rmax, A[:, 1].max() + rmax)
        else:
            self.bbox = None

    def dist(self, px, py, planar_only=False):
        """Edge distance from points (P,) to this copper; <0 inside. Returns d, nx, ny.
        planar_only drops via / PTH barrels (they exist on every layer, so they are
        handled by the same-layer check)."""
        P = len(px)
        best = np.full(P, np.inf)
        bx_ = np.zeros(P); by_ = np.zeros(P)
        X = px[:, None]; Y = py[:, None]
        Cs = self.C[~self.barrel] if planar_only else self.C
        if len(Cs):
            d, cx, cy = pt_seg(X, Y, Cs[None, :, 0], Cs[None, :, 1], Cs[None, :, 2], Cs[None, :, 3])
            d = d - Cs[None, :, 4]
            j = np.argmin(d, axis=1)
            dj = d[np.arange(P), j]
            m = dj < best
            best[m] = dj[m]; bx_[m] = cx[np.arange(P), j][m]; by_[m] = cy[np.arange(P), j][m]
        if len(self.E):
            d, cx, cy = pt_seg(X, Y, self.E[None, :, 0], self.E[None, :, 1], self.E[None, :, 2], self.E[None, :, 3])
            # inside test (even-odd) per polygon
            ay = self.E[None, :, 1]; by = self.E[None, :, 3]
            ax = self.E[None, :, 0]; bx = self.E[None, :, 2]
            cond = (ay > Y) != (by > Y)
            with np.errstate(divide="ignore", invalid="ignore"):
                xint = (bx - ax) * (Y - ay) / np.where(by - ay == 0, 1e-12, by - ay) + ax
            cross = cond & (X < xint)
            inside_any = np.zeros(P, dtype=bool)
            starts = np.r_[0, np.flatnonzero(np.diff(self.Epid)) + 1]
            sums = np.add.reduceat(cross.astype(np.int32), starts, axis=1)
            inside_any = (sums % 2 == 1).any(axis=1)
            j = np.argmin(d, axis=1)
            dj = d[np.arange(P), j]
            dj = np.where(inside_any, -dj, dj)
            m = dj < best
            best[m] = dj[m]; bx_[m] = cx[np.arange(P), j][m]; by_[m] = cy[np.arange(P), j][m]
        return best, bx_, by_


# ----------------------------------------------------------------------------
# Board extraction
# ----------------------------------------------------------------------------
def poly_points(sps, maxpts=None):
    out = []
    for i in range(sps.OutlineCount()):
        o = sps.Outline(i)
        pts = [(ToMM(o.CPoint(j).x), ToMM(o.CPoint(j).y)) for j in range(o.PointCount())]
        if len(pts) >= 3:
            out.append(np.array(pts))
        for h in range(sps.HoleCount(i)):
            ho = sps.Hole(i, h)
            hp = [(ToMM(ho.CPoint(j).x), ToMM(ho.CPoint(j).y)) for j in range(ho.PointCount())]
            if len(hp) >= 3:
                out.append(np.array(hp))  # even-odd: holes as extra rings
    return out


def stackup_from_file(path):
    txt = open(path).read()
    m = re.search(r"\(stackup(.*?)\(copper_finish", txt, re.S)
    layers = []
    if not m:
        return layers
    for lm in re.finditer(r'\(layer "([^"]+)"\s*\(type "([^"]+)"\)(.*?)\n\t\t\t\)', m.group(1), re.S):
        name, typ, rest = lm.group(1), lm.group(2), lm.group(3)
        th = re.search(r"\(thickness ([\d.]+)\)", rest)
        er = re.search(r"\(epsilon_r ([\d.]+)\)", rest)
        layers.append((name, typ, float(th.group(1)) if th else 0.0, float(er.group(1)) if er else None))
    return layers


class Board:
    def __init__(self, path, rules, amp_ref, mcu_ref, flash_ref, refill=False):
        self.path = path
        self.b = pcbnew.LoadBoard(path)
        if refill:
            # fill zones in memory only (the file is never written)
            pcbnew.ZONE_FILLER(self.b).Fill(self.b.Zones())
        self.R = rules
        b = self.b
        self.cu = [b.GetLayerName(l) for l in b.GetEnabledLayers().CuStack()]
        self.lid = {b.GetLayerName(l): l for l in b.GetEnabledLayers().CuStack()}
        self.order = {n: i for i, n in enumerate(self.cu)}
        self.stack = stackup_from_file(path)
        self.gap_h = self._dielectrics()
        self.amp_ref, self.mcu_ref, self.flash_ref = amp_ref, mcu_ref, flash_ref
        self._collect()

    def _dielectrics(self):
        """thickness/eps between consecutive copper layers."""
        out = {}
        seq = [s for s in self.stack if s[1] in ("copper",) or "dielectric" in s[0] or s[1] in ("core", "prepreg")]
        prev = None; acc = 0.0; er = None
        for name, typ, th, e in seq:
            if typ == "copper":
                if prev is not None:
                    out[(prev, name)] = (acc, er)
                    out[(name, prev)] = (acc, er)
                prev = name; acc = 0.0; er = None
            else:
                acc += th; er = e if e else er
        return out

    def _collect(self):
        b = self.b
        R = self.R
        self.net_items = defaultdict(lambda: defaultdict(Prims))   # net -> layer -> Prims
        self.tracks = defaultdict(list)    # net -> [(layer, ax, ay, bx, by, w)]
        self.vias = defaultdict(list)      # net -> [(x, y, d, drill, layers)]
        self.pads = defaultdict(list)      # net -> [(ref, num, layer, x, y, poly)]
        self.fp_by_ref = {}
        for t in b.GetTracks():
            net = t.GetNetname()
            cls = t.GetClass()
            if cls == "PCB_VIA":
                x, y = ToMM(t.GetPosition().x), ToMM(t.GetPosition().y)
                top, bot = self.order[b.GetLayerName(t.TopLayer())], self.order[b.GetLayerName(t.BottomLayer())]
                lays = self.cu[min(top, bot):max(top, bot) + 1]
                drill = ToMM(t.GetDrillValue())
                for L in lays:
                    r = ToMM(t.GetWidth(self.lid[L])) / 2 if t.FlashLayer(self.lid[L]) else drill / 2 + 0.025
                    self.net_items[net][L].caps.append((x, y, x, y, r, "via"))
                self.vias[net].append((x, y, ToMM(t.GetWidth(self.lid[lays[0]])), drill, lays))
                continue
            L = t.GetLayerName()
            w = ToMM(t.GetWidth())
            if cls == "PCB_ARC":
                st, md, en = t.GetStart(), t.GetMid(), t.GetEnd()
                pts = self._arc_pts(st, md, en)
            else:
                pts = [(ToMM(t.GetStart().x), ToMM(t.GetStart().y)), (ToMM(t.GetEnd().x), ToMM(t.GetEnd().y))]
            for (ax, ay), (bx, by) in zip(pts[:-1], pts[1:]):
                self.net_items[net][L].caps.append((ax, ay, bx, by, w / 2, "track"))
                self.tracks[net].append((L, ax, ay, bx, by, w))
        for fp in b.GetFootprints():
            ref = fp.GetReference()
            self.fp_by_ref[ref] = fp
            for pad in fp.Pads():
                net = pad.GetNetname()
                for L in self.cu:
                    lid = self.lid[L]
                    if not pad.IsOnLayer(lid):
                        continue
                    if pad.GetAttribute() in (pcbnew.PAD_ATTRIB_PTH,) and not pad.FlashLayer(lid):
                        # barrel only
                        x, y = ToMM(pad.GetPosition().x), ToMM(pad.GetPosition().y)
                        dr = pad.GetDrillSize()
                        r = ToMM(max(dr.x, dr.y)) / 2 + 0.025
                        self.net_items[net][L].caps.append((x, y, x, y, r, "pth"))
                        continue
                    try:
                        sps = pad.GetEffectivePolygon(lid, pcbnew.ERROR_INSIDE)
                    except TypeError:
                        sps = pad.GetEffectivePolygon(lid)
                    for P in poly_points(sps):
                        self.net_items[net][L].polys.append(P)
                        self.pads[net].append((ref, pad.GetNumber(), L, ToMM(pad.GetPosition().x), ToMM(pad.GetPosition().y), P))
        # zone fills: GND fills are shields / references; other fills are copper of their net
        self.gnd_fill = {}
        self.gnd_fill_inset = {}
        for z in b.Zones():
            if z.GetIsRuleArea():
                continue
            net = z.GetNetname()
            for L in self.cu:
                lid = self.lid[L]
                if not z.IsOnLayer(lid):
                    continue
                fp_ = z.GetFilledPolysList(lid)
                if fp_.OutlineCount() == 0:
                    continue
                if net == "GND":
                    s = self.gnd_fill.get(L) or pcbnew.SHAPE_POLY_SET()
                    s.BooleanAdd(fp_)
                    self.gnd_fill[L] = s
                else:
                    cp = pcbnew.SHAPE_POLY_SET(fp_)
                    cp.Unfracture()
                    for P in poly_points(cp):
                        self.net_items[net][L].polys.append(P)
        for L, s in self.gnd_fill.items():
            s2 = pcbnew.SHAPE_POLY_SET(s)
            s2.Unfracture()
            ins = pcbnew.SHAPE_POLY_SET(s2)
            ins.Inflate(-int(self.R["shield_margin"] * 1e6), pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, 5000)
            self.gnd_fill_inset[L] = ins
            ref = pcbnew.SHAPE_POLY_SET(s2)
            self.gnd_fill[L] = s2
        for net in self.net_items:
            for L in self.net_items[net]:
                self.net_items[net][L].finalize()

    @staticmethod
    def _arc_pts(st, md, en, n=12):
        x1, y1 = ToMM(st.x), ToMM(st.y)
        x2, y2 = ToMM(md.x), ToMM(md.y)
        x3, y3 = ToMM(en.x), ToMM(en.y)
        d = 2 * (x1 * (y2 - y3) + x2 * (y3 - y1) + x3 * (y1 - y2))
        if abs(d) < 1e-12:
            return [(x1, y1), (x3, y3)]
        ux = ((x1 * x1 + y1 * y1) * (y2 - y3) + (x2 * x2 + y2 * y2) * (y3 - y1) + (x3 * x3 + y3 * y3) * (y1 - y2)) / d
        uy = ((x1 * x1 + y1 * y1) * (x3 - x2) + (x2 * x2 + y2 * y2) * (x1 - x3) + (x3 * x3 + y3 * y3) * (x2 - x1)) / d
        r = math.hypot(x1 - ux, y1 - uy)
        a1 = math.atan2(y1 - uy, x1 - ux); a2 = math.atan2(y2 - uy, x2 - ux); a3 = math.atan2(y3 - uy, x3 - ux)
        def norm(a):
            return (a + 2 * math.pi) % (2 * math.pi)
        # choose direction passing through a2
        ccw = norm(a2 - a1) < norm(a3 - a1)
        span = norm(a3 - a1) if ccw else -norm(a1 - a3)
        return [(ux + r * math.cos(a1 + span * k / n), uy + r * math.sin(a1 + span * k / n)) for k in range(n + 1)]

    # ------------------------------------------------------------------
    def gnd_covers(self, L, x, y, inset=True):
        s = (self.gnd_fill_inset if inset else self.gnd_fill).get(L)
        if s is None:
            return False
        return s.Contains(pcbnew.VECTOR2I(int(x * 1e6), int(y * 1e6)))

    def layers_between(self, La, Lb):
        i, j = sorted((self.order[La], self.order[Lb]))
        return self.cu[i + 1:j]


# ----------------------------------------------------------------------------
# Victim sampling
# ----------------------------------------------------------------------------
def victim_samples(B, net, step):
    """Return list of per-layer sample arrays: dict L -> dict(x, y, r, ds, kind, item)."""
    out = defaultdict(lambda: {"x": [], "y": [], "r": [], "ds": [], "kind": [], "item": []})
    for k, (L, ax, ay, bx, by, w) in enumerate(B.tracks.get(net, [])):
        L_ = math.hypot(bx - ax, by - ay)
        n = max(1, int(math.ceil(L_ / step)))
        for i in range(n):
            t = (i + 0.5) / n
            o = out[L]
            o["x"].append(ax + t * (bx - ax)); o["y"].append(ay + t * (by - ay))
            o["r"].append(w / 2); o["ds"].append(L_ / n); o["kind"].append("track"); o["item"].append(k)
    for k, (x, y, d, drill, lays) in enumerate(B.vias.get(net, [])):
        for L in lays:
            o = out[L]
            o["x"].append(x); o["y"].append(y); o["r"].append(d / 2); o["ds"].append(0.0)
            o["kind"].append("via"); o["item"].append(10000 + k)
    for k, (ref, num, L, x, y, P) in enumerate(B.pads.get(net, [])):
        o = out[L]
        Q = np.roll(P, -1, axis=0)
        for (ax, ay), (bx, by) in zip(P, Q):
            L_ = math.hypot(bx - ax, by - ay)
            n = max(1, int(math.ceil(L_ / step)))
            for i in range(n):
                t = i / n
                o["x"].append(ax + t * (bx - ax)); o["y"].append(ay + t * (by - ay)); o["r"].append(0.0)
                o["ds"].append(0.0); o["kind"].append("pad:%s.%s" % (ref, num)); o["item"].append(20000 + k)
    res = {}
    for L, o in out.items():
        res[L] = {kk: np.array(v) if kk not in ("kind",) else v for kk, v in o.items()}
        res[L]["isvia"] = np.array([k == "via" for k in o["kind"]], dtype=bool)
    return res


def cluster(points, link=0.4):
    """Union-find clustering of (x,y) points; returns list of index lists."""
    n = len(points)
    if n == 0:
        return []
    P = np.array(points)
    parent = list(range(n))
    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a
    order = np.argsort(P[:, 0])
    Ps = P[order]
    for ii in range(n):
        jj = ii + 1
        while jj < n and Ps[jj, 0] - Ps[ii, 0] <= link:
            if abs(Ps[jj, 1] - Ps[ii, 1]) <= link:
                ra, rb = find(order[ii]), find(order[jj])
                if ra != rb:
                    parent[ra] = rb
            jj += 1
    groups = defaultdict(list)
    for i in range(n):
        groups[find(i)].append(i)
    return list(groups.values())


# ----------------------------------------------------------------------------
# Checks
# ----------------------------------------------------------------------------
class Report:
    def __init__(self):
        self.findings = []
        self.info = {}

    def add(self, sev, rule, msg, **kw):
        d = {"sev": sev, "rule": rule, "msg": msg}
        d.update(kw)
        self.findings.append(d)


def overlap_area(B, net_a, La, net_b, Lb, pads_only_a=False):
    """Exact broadside overlap area (mm^2) of two nets' copper on two layers."""
    def netpoly(net, L, pads_only=False):
        s = pcbnew.SHAPE_POLY_SET()
        for t in ([] if pads_only else B.b.GetTracks()):
            if t.GetNetname() != net:
                continue
            if t.GetClass() == "PCB_VIA":
                if not t.IsOnLayer(B.lid[L]):
                    continue
            elif t.GetLayerName() != L:
                continue
            t.TransformShapeToPolygon(s, B.lid[L], 0, 5000, pcbnew.ERROR_INSIDE)
        for fp in B.b.GetFootprints():
            for pad in fp.Pads():
                if pad.GetNetname() == net and pad.IsOnLayer(B.lid[L]):
                    pad.TransformShapeToPolygon(s, B.lid[L], 0, 5000, pcbnew.ERROR_INSIDE)
        s.Simplify()
        return s
    a = netpoly(net_a, La, pads_only_a)
    bb = netpoly(net_b, Lb)
    if a.OutlineCount() == 0 or bb.OutlineCount() == 0:
        return 0.0
    a.BooleanIntersection(bb)
    return a.Area() / 1e12


def check_crosstalk(B, rep):
    R = B.R
    nets = list(B.net_items.keys())
    vclass = {n: cls_of(n, R["victims"]) for n in nets}
    aclass = {n: cls_of(n, R["aggressors"]) for n in nets}
    victims = [n for n in nets if vclass[n]]
    aggressors = [n for n in nets if aclass[n]]
    rep.info["victims"] = {n: vclass[n] for n in victims}
    rep.info["aggressors"] = {n: aclass[n] for n in aggressors}
    mcu = B.fp_by_ref.get(B.mcu_ref)
    mcu_pads = []
    if mcu is not None:
        for p in mcu.Pads():
            mcu_pads.append((ToMM(p.GetPosition().x), ToMM(p.GetPosition().y), p.GetNetname()))
    esc_r = R["pin_escape_radius"]
    step = R["sample_step"]
    sites = []
    coupling = defaultdict(lambda: {"same_len": 0.0, "same_min": np.inf, "adj_len": 0.0, "adj_min": np.inf,
                                    "adj_area": 0.0, "shld_len": 0.0})
    for v in victims:
        vc = vclass[v]
        lim_v = R["limits"].get(vc, {})
        S = victim_samples(B, v, step)
        for Lv, s in S.items():
            if len(s["x"]) == 0:
                continue
            for a in aggressors:
                ac = aclass[a]
                lim = lim_v.get(ac)
                if lim is None or a == v:
                    continue
                for La, prim in B.net_items[a].items():
                    if prim.bbox is None:
                        continue
                    thr = max(lim["same"], lim["par_gap"], lim["adj"]) if La == Lv else max(lim["adj"], 0.3)
                    if La != Lv and not prim.has_planar:
                        continue
                    x0, y0, x1, y1 = prim.bbox
                    m = (s["x"] > x0 - thr - 1) & (s["x"] < x1 + thr + 1) & (s["y"] > y0 - thr - 1) & (s["y"] < y1 + thr + 1)
                    if La != Lv:
                        m &= s["isvia"] == False
                    if not m.any():
                        continue
                    idx = np.flatnonzero(m)
                    d, nx, ny = prim.dist(s["x"][idx], s["y"][idx], planar_only=(La != Lv))
                    gap = d - s["r"][idx]
                    if La == Lv:
                        rel = "same"
                    else:
                        between = B.layers_between(Lv, La)
                        shield_layers = [L for L in between if L in B.gnd_fill]
                        rel = "adj"
                    near = gap < thr
                    if not near.any():
                        continue
                    for k in np.flatnonzero(near):
                        i = idx[k]
                        g = float(gap[k])
                        kind = s["kind"][i]
                        x, y = float(s["x"][i]), float(s["y"][i])
                        ds = float(s["ds"][i])
                        if rel == "same":
                            if g >= lim["same"] and g >= lim["par_gap"]:
                                continue
                            sub = "same"
                        else:
                            shielded = False
                            for L in shield_layers:
                                if B.gnd_covers(L, x, y) and B.gnd_covers(L, float(nx[k]), float(ny[k])):
                                    shielded = True
                                    break
                            if shielded:
                                if g >= 0.0:      # shielded and not even overlapping: ignore
                                    continue
                                sub = "shld"
                            else:
                                if g >= lim["adj"]:
                                    continue
                                sub = "adj"
                        # pin-field escape exemption: both points near the same MCU pin row
                        esc = False
                        if mcu_pads and ("MCU" in R["pin_escape_refs"]):
                            dmin = min(math.hypot(x - px, y - py) for px, py, pn in mcu_pads if pn == v) if any(pn == v for _, _, pn in mcu_pads) else np.inf
                            if dmin < esc_r:
                                esc = True
                        sites.append({"victim": v, "vclass": vc, "aggr": a, "aclass": ac, "Lv": Lv, "La": La,
                                      "sub": sub, "gap": g, "x": x, "y": y, "ds": ds, "kind": kind, "esc": esc})
                        c = coupling[(v, a, Lv, La)]
                        if sub == "same":
                            c["same_min"] = min(c["same_min"], g)
                            if g < lim["par_gap"]:
                                c["same_len"] += ds
                        elif sub == "adj":
                            c["adj_min"] = min(c["adj_min"], g)
                            c["adj_len"] += ds
                        else:
                            c["shld_len"] += ds
    # group sites into findings
    groups = defaultdict(list)
    for s in sites:
        groups[(s["victim"], s["aggr"], s["Lv"], s["La"], s["sub"])].append(s)
    for key, lst in groups.items():
        v, a, Lv, La, sub = key
        vc, ac = lst[0]["vclass"], lst[0]["aclass"]
        lim = R["limits"][vc][ac]
        for grp in cluster([(s["x"], s["y"]) for s in lst], link=0.4):
            ss = [lst[i] for i in grp]
            gmin = min(s["gap"] for s in ss)
            plen = sum(s["ds"] for s in ss if (sub != "same" or s["gap"] < lim["par_gap"]))
            xs = [s["x"] for s in ss]; ys = [s["y"] for s in ss]
            cx, cy = float(np.mean(xs)), float(np.mean(ys))
            esc = all(s["esc"] for s in ss)
            kinds = sorted(set(s["kind"].split(":")[0] for s in ss))
            pads = sorted(set(s["kind"].split(":")[1] for s in ss if s["kind"].startswith("pad:")))
            if sub == "same":
                viol_gap = gmin < lim["same"]
                viol_par = plen > lim["par_len"] and any(s["gap"] < lim["par_gap"] for s in ss)
                if not (viol_gap or viol_par):
                    continue
                sev = lim.get("sev", "ERROR")
                if sev == "ERROR" and vc != "S0_summing" and not viol_par and plen < 0.2:
                    sev = "WARN"   # point contact (pad corner / via) without a parallel run
                rule = "XT-SAME"
                what = "min gap %.3f (limit %.2f), parallel %.2f mm at gap<%.2f (limit %.1f)" % (gmin, lim["same"], plen, lim["par_gap"], lim["par_len"])
            elif sub == "adj":
                sev = lim.get("sev", "ERROR")
                rule = "XT-ADJ"
                what = "%s %s/%s, no GND fill between; projected gap %.3f (limit %.2f), coupled length %.2f" % (
                    "CROSSES/OVERLAPS" if gmin < 0 else "near", Lv, La, gmin, lim["adj"], plen)
            else:
                sev = lim.get("shielded")
                if sev is None:
                    continue
                rule = "XT-SHLD"
                what = "crosses %s/%s through GND fill; overlap length %.2f" % (Lv, La, plen)
            if esc and sev == "ERROR":
                sev = "INFO"
                what += " [MCU pin-field escape]"
            rep.add(sev, rule, "%s (%s) vs %s (%s): %s" % (short(v), vc, short(a), ac, what),
                    victim=v, aggressor=a, Lv=Lv, La=La, u=round(cx, 2), v=round(cy, 2),
                    bbox=[round(min(xs), 2), round(min(ys), 2), round(max(xs), 2), round(max(ys), 2)],
                    gap=round(gmin, 3), length=round(plen, 2), kinds=kinds, pads=pads)
    # overlap areas + capacitance estimate for unshielded different-layer pairs
    eps0 = 8.854e-3  # pF/mm
    cap = []
    for (v, a, Lv, La), c in coupling.items():
        if Lv != La and c["adj_len"] > 0:
            A = overlap_area(B, v, Lv, a, La)
            h, er = B.gap_h.get((Lv, La), (None, None))
            if h is None:
                # non-adjacent layers: sum dielectrics
                i, j = sorted((B.order[Lv], B.order[La]))
                h = sum(B.gap_h.get((B.cu[k], B.cu[k + 1]), (0, 0))[0] for k in range(i, j))
                er = 4.3
            C = eps0 * (er or 4.3) * A / h if h else 0.0
            c["adj_area"] = A
            c["adj_cpf"] = C
            cap.append({"victim": v, "aggr": a, "Lv": Lv, "La": La, "area_mm2": round(A, 4),
                        "h_mm": round(h, 4), "C_pp_pF": round(C, 4), "len_mm": round(c["adj_len"], 2),
                        "min_gap": round(c["adj_min"], 3)})
    rep.info["unshielded_overlap"] = sorted(cap, key=lambda r: -r["C_pp_pF"])
    rep.info["coupling_table"] = [
        {"victim": v, "aggr": a, "Lv": Lv, "La": La,
         "same_min": (None if c["same_min"] == np.inf else round(c["same_min"], 3)),
         "same_par_len": round(c["same_len"], 2),
         "adj_min": (None if c["adj_min"] == np.inf else round(c["adj_min"], 3)),
         "adj_len": round(c["adj_len"], 2), "shld_len": round(c["shld_len"], 2)}
        for (v, a, Lv, La), c in sorted(coupling.items())]


def net_len(B, net):
    return sum(math.hypot(bx - ax, by - ay) for (L, ax, ay, bx, by, w) in B.tracks.get(net, []))


def find_net(B, base):
    for n in B.net_items:
        if short(n) == base:
            return n
    return None


def check_summing(B, rep):
    R = B.R
    amp = B.fp_by_ref.get(B.amp_ref)
    for n in B.net_items:
        if cls_of(n, R["victims"]) != "S0_summing":
            continue
        L = net_len(B, n)
        vias = len(B.vias.get(n, []))
        layers = sorted(set(t[0] for t in B.tracks.get(n, [])))
        pads = [(ref, num, x, y) for (ref, num, Lp, x, y, P) in B.pads.get(n, [])]
        amp_p = [p for p in pads if p[0] == B.amp_ref]
        oth = [p for p in pads if p[0] != B.amp_ref]
        dist = None
        if amp_p and oth:
            dist = min(math.hypot(a[2] - o[2], a[3] - o[3]) for a in amp_p for o in oth)
        rep.info.setdefault("summing", {})[n] = {"len_mm": round(L, 2), "vias": vias, "layers": layers,
                                                 "pads": ["%s.%s" % (p[0], p[1]) for p in pads],
                                                 "rin_to_pin_mm": None if dist is None else round(dist, 2)}
        s = R["summing"]
        if dist is not None and dist > s["max_pad_dist"]:
            rep.add("ERROR", "SUM-GEOM", "%s: Rin pad to %s pin centre distance %.2f mm > %.1f" % (short(n), B.amp_ref, dist, s["max_pad_dist"]), victim=n)
        if L > s["max_len"]:
            rep.add("ERROR", "SUM-GEOM", "%s: routed length %.2f mm > %.1f" % (short(n), L, s["max_len"]), victim=n)
        if vias > s["max_vias"]:
            rep.add("ERROR", "SUM-GEOM", "%s: %d vias (max %d); layers %s" % (short(n), vias, s["max_vias"], layers), victim=n)
        if len(layers) > 1:
            rep.add("ERROR", "SUM-GEOM", "%s: routed on %d layers %s" % (short(n), len(layers), layers), victim=n)


def check_reference(B, rep):
    R = B.R
    res = {}
    for n in B.net_items:
        vc = cls_of(n, R["victims"])
        if vc not in R["ref_min_fraction"]:
            continue
        tot = 0.0; ok = 0.0
        per_layer = defaultdict(lambda: [0.0, 0.0])
        for (L, ax, ay, bx, by, w) in B.tracks.get(n, []):
            i = B.order[L]
            nbrs = [B.cu[j] for j in (i - 1, i + 1) if 0 <= j < len(B.cu)]
            # nearest neighbour(s) by dielectric thickness
            hs = [(B.gap_h.get((L, Ln), (9, None))[0], Ln) for Ln in nbrs]
            hmin = min(h for h, _ in hs)
            refs = [Ln for h, Ln in hs if abs(h - hmin) < 1e-6]
            Ls = math.hypot(bx - ax, by - ay)
            k = max(1, int(Ls / 0.05))
            # offsets to both track edges + margin
            if Ls > 0:
                nx_, ny_ = -(by - ay) / Ls, (bx - ax) / Ls
            else:
                nx_, ny_ = 0, 0
            off = w / 2 + R["ref_margin"]
            for q in range(k):
                t = (q + 0.5) / k
                x = ax + t * (bx - ax); y = ay + t * (by - ay)
                good = any(all(B.gnd_covers(Ln, x + sgn * off * nx_, y + sgn * off * ny_, inset=False) for sgn in (-1, 0, 1)) for Ln in refs)
                tot += Ls / k; per_layer[L][0] += Ls / k
                if good:
                    ok += Ls / k; per_layer[L][1] += Ls / k
        if tot == 0:
            continue
        frac = ok / tot
        res[n] = {"len": round(tot, 2), "ref_frac": round(frac, 3),
                  "per_layer": {L: "%.2f/%.2f mm" % (v[1], v[0]) for L, v in per_layer.items()}}
        if frac < R["ref_min_fraction"][vc]:
            worst = [L for L, v in per_layer.items() if v[0] - v[1] > 0.05]
            rep.add("ERROR" if vc == "S0_summing" else "WARN", "REF",
                    "%s (%s): only %.0f%% of %.2f mm has GND fill on the nearest layer (unreferenced on %s)" % (
                        short(n), vc, 100 * frac, tot, ",".join(worst)), victim=n)
    rep.info["reference"] = res


def check_pairs(B, rep):
    R = B.R
    out = {}
    for pbase, nbase in R["pairs"]:
        p, n = find_net(B, pbase), find_net(B, nbase)
        if not p or not n:
            continue
        Lp, Ln = net_len(B, p), net_len(B, n)
        layp = sorted(set(t[0] for t in B.tracks.get(p, [])))
        layn = sorted(set(t[0] for t in B.tracks.get(n, [])))
        S = victim_samples(B, p, 0.05)
        seps = []
        for L, s in S.items():
            m = np.array([k == "track" for k in s["kind"]], dtype=bool)
            if not m.any():
                continue
            # distance to N copper on ANY layer (projected)
            best = np.full(m.sum(), np.inf)
            for Lq, prim in B.net_items[n].items():
                if prim.bbox is None:
                    continue
                d, _, _ = prim.dist(s["x"][m], s["y"][m])
                best = np.minimum(best, d)
            seps.extend(list(best))
        seps = np.array(seps) if seps else np.array([0.0])
        p95 = float(np.percentile(seps, 95)); mx = float(seps.max())
        out["%s/%s" % (pbase, nbase)] = {"len_p": round(Lp, 2), "len_n": round(Ln, 2), "layers_p": layp,
                                         "layers_n": layn, "vias_p": len(B.vias.get(p, [])), "vias_n": len(B.vias.get(n, [])),
                                         "sep_p95": round(p95, 2), "sep_max": round(mx, 2)}
        mm = abs(Lp - Ln)
        if mm > max(R["pair_max_mismatch"], 0.25 * max(Lp, Ln)):
            rep.add("WARN", "PAIR", "%s/%s length mismatch %.2f vs %.2f mm" % (pbase, nbase, Lp, Ln))
        if layp != layn:
            rep.add("WARN", "PAIR", "%s/%s on different layers %s vs %s" % (pbase, nbase, layp, layn))
        if p95 > R["pair_max_p95_sep"]:
            rep.add("WARN", "PAIR", "%s/%s not routed together: P-to-N separation p95 %.2f mm, max %.2f mm (limit p95 %.1f)" % (
                pbase, nbase, p95, mx, R["pair_max_p95_sep"]))
    rep.info["pairs"] = out


def check_supply(B, rep):
    R = B.R
    mcu = B.fp_by_ref.get(B.mcu_ref)
    fl = B.fp_by_ref.get(B.flash_ref)
    if mcu is None:
        return
    pads = {p.GetNumber(): p for p in mcu.Pads()}
    info = {}
    for num, fn in R["mcu_audio_pins"].items():
        p = pads.get(num)
        if p is None:
            continue
        net = p.GetNetname()
        px, py = ToMM(p.GetPosition().x), ToMM(p.GetPosition().y)
        # nearest capacitor pad on the same net
        best = None
        for fp in B.b.GetFootprints():
            if not fp.GetReference().startswith("C"):
                continue
            for q in fp.Pads():
                if q.GetNetname() == net:
                    d = math.hypot(ToMM(q.GetPosition().x) - px, ToMM(q.GetPosition().y) - py)
                    if best is None or d < best[0]:
                        best = (d, fp.GetReference(), fp.GetValue(), fp.GetLayerName())
        info[fn] = {"pin": num, "net": net, "nearest_cap": None if best is None else
                    {"ref": best[1], "value": best[2], "dist_mm": round(best[0], 2), "side": best[3]}}
        if best is None or best[0] > R["decap_max_dist"]:
            rep.add("WARN", "SUPPLY", "%s (pin %s, %s): nearest decoupling cap %s at %s mm (limit %.1f)" % (
                fn, num, net, best[1] if best else "-", "%.2f" % best[0] if best else "-", R["decap_max_dist"]))
        if fn == "AVDD33_AUD" and fl is not None:
            for q in fl.Pads():
                if q.GetNumber() == R["flash_vcc_pin"] and q.GetNetname() == net:
                    rep.add("WARN", "SUPPLY", "AVDD33_AUD (%s pin %s) shares net %s with flash %s VCC (pin %s): flash read current ripples the audio supply" % (
                        B.mcu_ref, num, net, B.flash_ref, q.GetNumber()), victim=net)
    rep.info["supply"] = info


def check_classd(B, rep):
    R = B.R
    amp = B.fp_by_ref.get(B.amp_ref)
    if amp is None:
        return
    info = {}
    for p in amp.Pads():
        net = p.GetNetname()
        if not re.search(r"AMP_VO[PN]$", net):
            continue
        px, py = ToMM(p.GetPosition().x), ToMM(p.GetPosition().y)
        others = []
        for fp in B.b.GetFootprints():
            if fp.GetReference() == B.amp_ref:
                continue
            for q in fp.Pads():
                if q.GetNetname() == net:
                    others.append((math.hypot(ToMM(q.GetPosition().x) - px, ToMM(q.GetPosition().y) - py), fp.GetReference()))
        L = net_len(B, net)
        info[short(net)] = {"pin": p.GetNumber(), "len_mm": round(L, 2),
                            "to": [{"ref": r, "dist": round(d, 2)} for d, r in sorted(others)]}
        if others and min(others)[0] > R["classd_max_dist"]:
            rep.add("WARN", "CLASSD", "%s: first part (%s) %.2f mm from %s pin %s (limit %.1f); routed %.2f mm" % (
                short(net), min(others)[1], min(others)[0], B.amp_ref, p.GetNumber(), R["classd_max_dist"], L))
    # amplifier orientation: input pins should face the input network, outputs the ferrites
    pins = {q.GetNumber(): (ToMM(q.GetPosition().x), ToMM(q.GetPosition().y)) for q in amp.Pads()}
    if all(k in pins for k in "3458"):
        rin = []
        for net in ("AMP_INP", "AMP_INN"):
            n = find_net(B, net)
            for (ref, num, L, x, y, P) in B.pads.get(n, []) if n else []:
                if ref != B.amp_ref:
                    rin.append((x, y))
        if rin:
            cx = np.mean([r[0] for r in rin]); cy = np.mean([r[1] for r in rin])
            din = np.mean([math.hypot(pins[k][0] - cx, pins[k][1] - cy) for k in "34"])
            dout = np.mean([math.hypot(pins[k][0] - cx, pins[k][1] - cy) for k in "58"])
            info["orientation"] = {"rin_to_in_pins": round(float(din), 2), "rin_to_out_pins": round(float(dout), 2)}
            if dout < din:
                rep.add("ERROR", "CLASSD", "%s orientation: Rin parts are closer to the OUTPUT pins (%.2f mm) than to the INPUT pins (%.2f mm); input traces must pass the class-D outputs" % (
                    B.amp_ref, dout, din))
    rep.info["classd"] = info




# ----------------------------------------------------------------------------
# Coupling-capacitance estimate (2D field-solver tables, see xsec_cap.py)
# ----------------------------------------------------------------------------
# mutual C per mm (pF/mm) of two 0.2 mm traces vs edge gap, r6 stackup
_G = [0.10, 0.15, 0.20, 0.30, 0.50, 0.75, 1.00, 1.50, 2.00]
_TAB = {
    "In2_unref":   [0.0513, 0.0387, 0.0337, 0.0247, 0.0148, 0.0081, 0.0049, 0.0017, 0.0006],
    "In2_refB":    [0.0323, 0.0208, 0.0164, 0.0096, 0.0038, 0.0013, 0.0005, 0.0001, 0.0000],
    "outer_unref": [0.0389, 0.0283, 0.0243, 0.0175, 0.0103, 0.0056, 0.0034, 0.0012, 0.0004],
    "outer_ref":   [0.0213, 0.0115, 0.0079, 0.0033, 0.0007, 0.0001, 0.0000, 0.0000, 0.0000],
}
# broadside In2<->B.Cu (0.0994 mm), lateral edge offset (negative = overlap)
_S = [-0.20, -0.10, 0.00, 0.10, 0.20, 0.30, 0.50, 0.75, 1.00]
_BROAD = [0.0929, 0.0816, 0.0585, 0.0389, 0.0274, 0.0203, 0.0122, 0.0067, 0.0041]
# in-band transimpedance from the victim node to the BTL output (ohm); see audio.md
_RF, _RIN, _RDAC, _ZDAC = 240e3, 22e3, 1e3, 100.0
_ZAC = (_RDAC * _RIN / (_RDAC + _RIN)) * _RF / _RIN
ZT = {"AMP_INP": _RF, "AMP_INN": _RF, "AC_P": _ZAC, "AC_N": _ZAC, "DAC_FILTER_P": _ZAC, "DAC_FILTER_N": _ZAC,
      "DAC_P": _ZDAC * _RF / (_RIN + _RDAC), "DAC_N": _ZDAC * _RF / (_RIN + _RDAC)}


def _interp(xs, ys, x):
    if x <= xs[0]:
        return ys[0]
    if x >= xs[-1]:
        return 0.0 if x > xs[-1] + 0.5 else ys[-1]
    return float(np.interp(x, xs, ys))


def estimate_coupling(B, window=2.0):
    """Rows: victim, aggressor, relation, min gap, lengths at gap thresholds, C_est (fF)."""
    R = B.R
    nets = list(B.net_items)
    victims = [n for n in nets if cls_of(n, R["victims"])]
    aggr = [n for n in nets if cls_of(n, R["aggressors"])]
    rows = []
    for v in victims:
        Sv = victim_samples(B, v, 0.025)
        for ag in aggr:
            if ag == v:
                continue
            acc = defaultdict(lambda: {"min": np.inf, "l02": 0.0, "l05": 0.0, "l10": 0.0, "C": 0.0, "pts": []})
            for Lv, s in Sv.items():
                for La, prim in B.net_items[ag].items():
                    if prim.bbox is None:
                        continue
                    same = (La == Lv)
                    if not same and not prim.has_planar:
                        continue
                    m = np.ones(len(s["x"]), dtype=bool) if same else ~s["isvia"]
                    x0, y0, x1, y1 = prim.bbox
                    m &= (s["x"] > x0 - window - .5) & (s["x"] < x1 + window + .5) & (s["y"] > y0 - window - .5) & (s["y"] < y1 + window + .5)
                    if not m.any():
                        continue
                    idx = np.flatnonzero(m)
                    d, nx, ny = prim.dist(s["x"][idx], s["y"][idx], planar_only=not same)
                    gap = d - s["r"][idx]
                    for k in np.flatnonzero(gap < window):
                        i = idx[k]; g = float(gap[k]); x = float(s["x"][i]); y = float(s["y"][i]); ds = float(s["ds"][i])
                        if same:
                            rel = "same:" + Lv
                            io = B.order[Lv]
                            nb = [B.cu[j] for j in (io - 1, io + 1) if 0 <= j < len(B.cu)]
                            hs = sorted((B.gap_h[(Lv, Ln)][0], Ln) for Ln in nb)
                            if Lv in (B.cu[0], B.cu[-1]):
                                tab = _TAB["outer_ref"] if B.gnd_covers(hs[0][1], x, y, inset=False) else _TAB["outer_unref"]
                            else:
                                refd = [Ln for h, Ln in hs if h < 0.2 and B.gnd_covers(Ln, x, y, inset=False)]
                                tab = _TAB["In2_refB"] if refd else _TAB["In2_unref"]
                            c = _interp(_G, tab, max(g, 0.1)) * ds
                        else:
                            between = B.layers_between(Lv, La)
                            shield = [L for L in between if L in B.gnd_fill]
                            if any(B.gnd_covers(L, x, y) and B.gnd_covers(L, float(nx[k]), float(ny[k])) for L in shield):
                                rel = "shielded:%s/%s" % (Lv, La)
                                c = 0.0
                            else:
                                rel = "unshielded:%s/%s" % (Lv, La)
                                h = B.gap_h.get((Lv, La), (None,))[0]
                                scale = 1.0 if (h is not None and h < 0.2) else 0.33
                                c = scale * _interp(_S, _BROAD, g) * ds
                        e = acc[rel]
                        e["min"] = min(e["min"], g)
                        if g < 0.2: e["l02"] += ds
                        if g < 0.5: e["l05"] += ds
                        if g < 1.0:
                            e["l10"] += ds
                            e["pts"].append((x, y))
                        e["C"] += c
            for rel, e in acc.items():
                area = 0.0
                if rel.startswith("unshielded"):
                    Lv_, La_ = rel.split(":")[1].split("/")
                    h = B.gap_h.get((Lv_, La_), (None, None))
                    if h[0] is not None and h[0] < 0.2:
                        area = overlap_area(B, v, Lv_, ag, La_, pads_only_a=True)
                        e["C"] += 1.3 * 8.854e-3 * (h[1] or 4.1) * area / h[0]   # pads: plate + 30 % fringe
                if e["min"] >= 1.0 and e["C"] < 1e-4:
                    continue
                pts = np.array(e["pts"]) if e["pts"] else None
                where = "" if pts is None else "u%.1f-%.1f v%.1f-%.1f" % (pts[:, 0].min(), pts[:, 0].max(), pts[:, 1].min(), pts[:, 1].max())
                vs = short(v)
                zt = ZT.get(vs, 0.0)
                rows.append({"victim": vs, "vclass": cls_of(v, R["victims"]), "aggressor": short(ag),
                             "aclass": cls_of(ag, R["aggressors"]), "relation": rel, "min_gap": round(e["min"], 3),
                             "len_lt0.2": round(e["l02"], 2), "len_lt0.5": round(e["l05"], 2), "len_lt1.0": round(e["l10"], 2),
                             "pad_overlap_mm2": round(area, 4), "C_est_fF": round(1000 * e["C"], 2),
                             "Zt_kohm": round(zt / 1e3, 2), "weight_fF_Mohm": round(e["C"] * 1000 * zt / 1e6, 3),
                             "where": where})
    rows.sort(key=lambda r: (-r["weight_fF_Mohm"], -r["C_est_fF"]))
    return rows


def check_budget(B, rep):
    R = B.R
    bud = R["budget"]
    rows = estimate_coupling(B)
    rep.info["coupling_rows"] = rows
    per = defaultdict(lambda: defaultdict(float))
    wtot = 0.0
    for r in rows:
        per[r["victim"]][r["aclass"]] += r["C_est_fF"]
        if r["aclass"] in ("A_fast", "A_power"):
            wtot += r["weight_fF_Mohm"]
    rep.info["coupling_totals_fF"] = {k: dict(v) for k, v in per.items()}
    rep.info["weighted_total_fF_Mohm"] = round(wtot, 3)
    for vnet, d in per.items():
        vc = cls_of(vnet, R["victims"]) or cls_of("audio::" + vnet, R["victims"])
        fast = d.get("A_fast", 0.0) + d.get("A_power", 0.0)
        cd = d.get("A_classd", 0.0)
        lim = bud.get(vc)
        if not lim:
            continue
        if fast > lim["fast_error"]:
            rep.add("ERROR", "BUDGET", "%s (%s): estimated coupling to fast/switching nets %.1f fF > %.1f fF" % (vnet, vc, fast, lim["fast_error"]), victim=vnet)
        elif fast > lim["fast_warn"]:
            rep.add("WARN", "BUDGET", "%s (%s): estimated coupling to fast/switching nets %.1f fF > %.1f fF" % (vnet, vc, fast, lim["fast_warn"]), victim=vnet)
        if cd > lim["classd_warn"]:
            rep.add("WARN", "BUDGET", "%s (%s): estimated coupling to class-D outputs %.1f fF > %.1f fF" % (vnet, vc, cd, lim["classd_warn"]), victim=vnet)
    ref = bud["r6_weighted_total"]
    imp = 20 * math.log10(ref / wtot) if wtot > 0 else float("inf")
    rep.info["improvement_vs_r6_dB"] = None if wtot == 0 else round(imp, 1)
    sev = "ERROR" if wtot > bud["weighted_total_max"] else "INFO"
    rep.add(sev, "BUDGET", "output-referred coupling sum(C_est x Zt) to fast/switching nets = %.2f fF*Mohm (limit %.1f; r6 = %.1f, i.e. %s dB vs r6)" % (
        wtot, bud["weighted_total_max"], ref, "inf" if wtot == 0 else "%.1f" % imp))


def check_amp_power(B, rep):
    """NS4150B VDD decoupling and GND via distance."""
    R = B.R
    amp = B.fp_by_ref.get(B.amp_ref)
    if amp is None:
        return
    info = {}
    pads = {p.GetNumber(): p for p in amp.Pads()}
    vdd, gnd = pads.get(R["amp_vdd_pin"]), pads.get(R["amp_gnd_pin"])
    if vdd is not None:
        vx, vy = ToMM(vdd.GetPosition().x), ToMM(vdd.GetPosition().y)
        best = None
        for fp in B.b.GetFootprints():
            if not fp.GetReference().startswith("C"):
                continue
            ps = {q.GetNetname(): q for q in fp.Pads()}
            if vdd.GetNetname() in ps and "GND" in ps:
                q = ps[vdd.GetNetname()]
                d = math.hypot(ToMM(q.GetPosition().x) - vx, ToMM(q.GetPosition().y) - vy)
                if best is None or d < best[0]:
                    best = (d, fp.GetReference(), fp.GetValue(), fp.GetLayerName())
        info["vdd_decap"] = None if best is None else {"ref": best[1], "value": best[2], "dist_mm": round(best[0], 2), "side": best[3]}
        if best is None or best[0] > R["amp_decap_max_dist"]:
            rep.add("ERROR", "AMP-PWR", "%s VDD pin %s: nearest decoupling cap %s at %.2f mm (limit %.1f; datasheet: 1 uF as close as possible to VDD)" % (
                B.amp_ref, R["amp_vdd_pin"], best[1] if best else "-", best[0] if best else -1, R["amp_decap_max_dist"]))
    if gnd is not None:
        gx, gy = ToMM(gnd.GetPosition().x), ToMM(gnd.GetPosition().y)
        dv = [math.hypot(x - gx, y - gy) for (x, y, d, dr, lays) in B.vias.get("GND", [])]
        dmin = min(dv) if dv else None
        info["gnd_via_mm"] = None if dmin is None else round(dmin, 2)
        if dmin is None or dmin > R["amp_gnd_via_max_dist"]:
            rep.add("WARN", "AMP-PWR", "%s GND pin %s: nearest GND via %.2f mm (limit %.1f)" % (B.amp_ref, R["amp_gnd_pin"], dmin if dmin else -1, R["amp_gnd_via_max_dist"]))
    rep.info["amp_power"] = info


# ----------------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("board")
    ap.add_argument("--rules")
    ap.add_argument("--md")
    ap.add_argument("--json")
    ap.add_argument("--amp", default="U1")
    ap.add_argument("--mcu", default="U5")
    ap.add_argument("--flash", default="U4")
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--refill", action="store_true", help="refill all zones in memory before checking")
    a = ap.parse_args()
    R = json.loads(json.dumps(DEFAULT_RULES))
    if a.rules:
        R.update(json.load(open(a.rules)))
    try:
        B = Board(a.board, R, a.amp, a.mcu, a.flash, refill=a.refill)
    except Exception as e:
        print("cannot load board:", e)
        return 2
    rep = Report()
    rep.info["stackup_gaps"] = {"%s-%s" % k: v for k, v in B.gap_h.items() if B.order[k[0]] < B.order[k[1]]}
    rep.info["gnd_fill_layers"] = sorted(B.gnd_fill.keys())
    check_summing(B, rep)
    check_crosstalk(B, rep)
    check_reference(B, rep)
    check_pairs(B, rep)
    check_supply(B, rep)
    check_classd(B, rep)
    check_amp_power(B, rep)
    check_budget(B, rep)
    order = {"ERROR": 0, "WARN": 1, "INFO": 2}
    rep.findings.sort(key=lambda f: (order.get(f["sev"], 3), f["rule"], f.get("victim", ""), f.get("gap", 0)))
    n_err = sum(1 for f in rep.findings if f["sev"] == "ERROR")
    n_warn = sum(1 for f in rep.findings if f["sev"] == "WARN")
    lines = []
    lines.append("# Audio crosstalk check: %s" % a.board)
    lines.append("")
    lines.append("Copper stack: %s; GND fills on: %s" % (" / ".join(B.cu), ", ".join(rep.info["gnd_fill_layers"]) or "none"))
    lines.append("Dielectric between layers: " + ", ".join("%s %.4f mm (er %s)" % (k, v[0], v[1]) for k, v in rep.info["stackup_gaps"].items()))
    lines.append("")
    lines.append("**%d ERROR, %d WARN, %d INFO**" % (n_err, n_warn, len(rep.findings) - n_err - n_warn))
    lines.append("")
    lines.append("| sev | rule | at (u,v) | finding |")
    lines.append("|---|---|---|---|")
    for f in rep.findings[:R["max_sites"]]:
        at = "(%.2f, %.2f)" % (f["u"], f["v"]) if "u" in f else ""
        lines.append("| %s | %s | %s | %s |" % (f["sev"], f["rule"], at, f["msg"]))
    lines.append("")
    if rep.info.get("summing"):
        lines.append("## Summing node")
        for n, d in rep.info["summing"].items():
            lines.append("- %s: %s" % (short(n), d))
    if rep.info.get("unshielded_overlap"):
        lines.append("")
        lines.append("## Unshielded different-layer overlap (parallel-plate C = e0*er*A/h, fringing not included)")
        lines.append("| victim | aggressor | layers | overlap mm^2 | h mm | C pF | coupled len mm | min proj gap |")
        lines.append("|---|---|---|---|---|---|---|---|")
        for r in rep.info["unshielded_overlap"]:
            lines.append("| %s | %s | %s/%s | %.4f | %.4f | %.4f | %.2f | %.3f |" % (
                short(r["victim"]), short(r["aggr"]), r["Lv"], r["La"], r["area_mm2"], r["h_mm"], r["C_pp_pF"], r["len_mm"], r["min_gap"]))
    if rep.info.get("coupling_rows"):
        lines.append("")
        lines.append("## Estimated coupling (2D-table estimate; C x Zt = output-referred weight)")
        lines.append("weighted total to fast/switching nets: %s fF*Mohm; improvement vs r6: %s dB" % (
            rep.info.get("weighted_total_fF_Mohm"), rep.info.get("improvement_vs_r6_dB")))
        lines.append("")
        lines.append("| victim | aggressor | relation | min gap | len<0.2 | len<0.5 | len<1.0 | pad ovl mm2 | C_est fF | Zt kohm | C*Zt fF*Mohm | where |")
        lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|")
        for r in rep.info["coupling_rows"]:
            if r["C_est_fF"] < 1.0 and r["min_gap"] > 0.5:
                continue
            lines.append("| %s | %s | %s | %.3f | %.2f | %.2f | %.2f | %.4f | %.1f | %.1f | %.3f | %s |" % (
                r["victim"], r["aggressor"], r["relation"], r["min_gap"], r["len_lt0.2"], r["len_lt0.5"], r["len_lt1.0"],
                r["pad_overlap_mm2"], r["C_est_fF"], r["Zt_kohm"], r["weight_fF_Mohm"], r["where"]))
        lines.append("")
        lines.append("Totals C_est (fF) per victim / aggressor class: " + json.dumps(
            {k: {c: round(x, 1) for c, x in v.items()} for k, v in rep.info["coupling_totals_fF"].items()}))
    for key, title in (("reference", "Reference plane coverage"), ("pairs", "Pairs"), ("supply", "Supply"), ("classd", "Class-D outputs"), ("amp_power", "Amplifier power")):
        if rep.info.get(key):
            lines.append("")
            lines.append("## " + title)
            for n, d in rep.info[key].items():
                lines.append("- %s: %s" % (short(n), d))
    txt = "\n".join(lines)
    if a.md:
        open(a.md, "w").write(txt + "\n")
    if a.json:
        def conv(o):
            if isinstance(o, (np.floating,)):
                return float(o)
            if isinstance(o, (np.integer,)):
                return int(o)
            return str(o)
        json.dump({"findings": rep.findings, "info": rep.info}, open(a.json, "w"), indent=1, default=conv)
    if not a.quiet:
        print(txt)
    else:
        print("%d ERROR, %d WARN" % (n_err, n_warn))
    return 1 if n_err else 0


if __name__ == "__main__":
    sys.exit(main())
