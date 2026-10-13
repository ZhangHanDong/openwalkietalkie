# SPDX-License-Identifier: MIT
"""Import a Freerouting SES into a KiCad 10 board, verify nothing else moved, save.

  $KPY ses_import.py PREPPED.kicad_pcb IN.ses OUT.kicad_pcb [--only-nets N1,N2,...]
                     [--refill] [--report out.json]

PREPPED must be the exact board the DSN was exported from (dsn_export.py
--save-prepped), so footprints and fixed copper match.

KiCad 10.0.4 behaviour (measured):
 * pcbnew.ImportSpecctraSES(board, path) -> True. It first DELETES every
   unlocked track/via of the board and keeps locked ones, then adds the SES
   wires/vias (Freerouting 2.4.1 writes only the wires it created/changed; the
   (type fix) / protect copper is not in the SES). So: lock everything you
   want kept before exporting the DSN (dsn_export.py --lock-rest).
 * Imported items come back unlocked, widths from the SES path width,
   vias as the padstack named in the SES ("Via[0-3]_450:200_um" -> 0.45/0.20).
 * The SES (placement ...) section re-places footprints. Freerouting writes
   `(component` with NO name for footprints whose KiCad lib id is empty (r6:
   MH2, MH4 -> DSN `(component ""`), and KiCad then rejects the whole SES
   (ImportSpecctraSES returns False, no message). This script therefore strips
   the placement section (Freerouting never moves parts) -> IN.noplace.ses.
--only-nets  delete imported items on any other net (Freerouting also
             'repairs' nets it believes are open, e.g. GND stubs to the plane).
"""
import argparse
import collections
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import kicad_helpers as kh  # noqa: E402
import pcbnew  # noqa: E402


def key(t):
    if t.GetClass() == "PCB_VIA":
        p = t.GetPosition()
        return ("V", t.GetNetname(), p.x, p.y, t.GetWidth(pcbnew.F_Cu), t.GetDrillValue())
    s, e = t.GetStart(), t.GetEnd()
    a, b = sorted([(s.x, s.y), (e.x, e.y)])
    return ("T", t.GetNetname(), t.GetLayer(), a, b, t.GetWidth())


def strip_placement(ses_path):
    """Write IN.noplace.ses without the (placement ...) block; return its path."""
    s = open(ses_path).read()
    i = s.find("  (placement")
    if i < 0:
        return ses_path
    depth = 0
    j = i
    while True:
        c = s[j]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                break
        j += 1
    out = os.path.splitext(ses_path)[0] + ".noplace.ses"
    with open(out, "w") as fh:
        fh.write(s[:i] + s[j + 1:].lstrip("\n"))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("board")
    ap.add_argument("ses")
    ap.add_argument("out")
    ap.add_argument("--only-nets", default="")
    ap.add_argument("--refill", action="store_true")
    ap.add_argument("--report")
    a = ap.parse_args()

    b = kh.load(a.board)
    before = collections.Counter(key(t) for t in b.GetTracks())
    fp_before = {f.GetReference(): (f.GetPosition().x, f.GetPosition().y, f.GetOrientationDegrees(), f.GetLayer())
                 for f in b.GetFootprints()}
    t0 = time.time()
    ses = strip_placement(a.ses)
    ok = pcbnew.ImportSpecctraSES(b, ses)
    dt = time.time() - t0
    if not ok:
        raise SystemExit("ImportSpecctraSES failed")
    after_items = list(b.GetTracks())
    after = collections.Counter(key(t) for t in after_items)
    removed = before - after
    added = after - before
    rep = {"import_s": round(dt, 3), "items_before": sum(before.values()), "items_after": sum(after.values()),
           "removed": sum(removed.values()), "added": sum(added.values())}
    rep["removed_by_net"] = collections.Counter(k[1] for k in removed.elements()).most_common()
    rep["added_by_net"] = collections.Counter(k[1] for k in added.elements()).most_common()

    only = [n for n in a.only_nets.split(",") if n]
    if only:
        added_keys = set(added)
        junk = [t for t in after_items if key(t) in added_keys and t.GetNetname() not in only]
        for t in junk:
            b.Delete(t)
        rep["dropped_off_target_items"] = len(junk)

    moved = [r for r, v in fp_before.items()
             if (lambda f: (f.GetPosition().x, f.GetPosition().y, f.GetOrientationDegrees(), f.GetLayer()))(
                 b.FindFootprintByReference(r)) != v]
    rep["footprints_moved"] = moved
    if a.refill:
        ok, fdt = kh.fill_zones(b)
        rep["refill_s"] = round(fdt, 2)
    rep["unrouted_after"] = kh.unrouted_count(b)
    # geometry quality of what was added
    segs = [t for t in b.GetTracks() if t.GetClass() == "PCB_TRACK" and key(t) in set(added)]
    import math
    ang = collections.Counter()
    for t in segs:
        dx = t.GetEnd().x - t.GetStart().x
        dy = t.GetEnd().y - t.GetStart().y
        d = math.degrees(math.atan2(dy, dx)) % 90 if (dx or dy) else 0
        ang["0/90" if min(d, 90 - d) < 0.01 else ("45" if abs(d - 45) < 0.01 else "other")] += 1
    rep["added_segment_angles"] = dict(ang)
    rep["added_length_mm"] = round(sum(kh.to_mm(t.GetLength()) for t in segs), 2)
    kh.save(b, a.out)
    rep["saved"] = a.out
    txt = json.dumps(rep, indent=1, default=str)
    print(txt)
    if a.report:
        open(a.report, "w").write(txt)


if __name__ == "__main__":
    main()
