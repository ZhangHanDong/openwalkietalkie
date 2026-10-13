# SPDX-License-Identifier: MIT
"""Prepare a board for Freerouting and export a Specctra DSN (KiCad 10.0.4).

  $KPY dsn_export.py IN.kicad_pcb OUT.dsn [--rip NET1,NET2,...] [--lock-rest]
                     [--save-prepped PREP.kicad_pcb] [--unlock-all]

--rip         delete tracks+vias of these nets before export (they become the job)
--lock-rest   SetLocked(True) on every remaining track/via -> exported as
              (wire ... (type protect)) / (via ... (type protect)) so Freerouting
              will not rip them up or move them.
--save-prepped  also save the modified board (needed: import the SES into THIS
              board, not the original, so footprints/zones/locks match).

Notes (measured on r6):
 * pcbnew.ExportSpecctraDSN(board, path) is the KiCad 10 python entry (the
   1-arg form needs a running GUI frame). Returns True; ~0.1 s for r6.
 * Zones with a net (GND on In1 and the B.Cu heat spread) are exported as
   (plane GND (polygon ...)); Freerouting treats the In1 plane as a GND
   connection target and may drop vias onto it.
 * Rule areas are exported as keepout / via_keepout / wire_keepout per layer
   depending on their track/via flags; pad/footprint/pour-only rule areas
   are not exported.
 * Netclasses become (class NAME nets... (circuit (use_via ...)) (rule (width)(clearance))).
"""
import argparse
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import kicad_helpers as kh  # noqa: E402
import pcbnew  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("board")
    ap.add_argument("dsn")
    ap.add_argument("--rip", default="")
    ap.add_argument("--rip-all", action="store_true", help="delete every track and via (full re-route)")
    ap.add_argument("--keep-window", action="append", default=[],
                    help="u0,v0,u1,v1 mm; ripped-net items fully inside are kept (repeatable)")
    ap.add_argument("--lock-rest", action="store_true")
    ap.add_argument("--unlock-all", action="store_true")
    ap.add_argument("--save-prepped")
    ap.add_argument("--keep-soft-keepouts", action="store_true",
                    help="do NOT drop rule areas that allow both tracks and vias (KiCad 10 "
                         "exports those as FULL (keepout ...) areas)")
    ap.add_argument("--drop-signal-zones", action="store_true",
                    help="drop net zones on signal layers (exported as (plane ...) conduction "
                         "areas) from the export copy; refill after import")
    a = ap.parse_args()

    b = kh.load(a.board)
    rip = [n for n in a.rip.split(",") if n]
    if a.rip_all:
        rip = sorted(set(t.GetNetname() for t in b.GetTracks()))
    for n in rip:
        if b.FindNet(n) is None:
            raise SystemExit("no such net: " + n)
    kw = [tuple(float(x) for x in w.split(",")) for w in a.keep_window]
    nrip = kh.delete_tracks(b, rip, keep_windows=kw) if rip else 0
    nlock = 0
    if a.unlock_all:
        kh.set_tracks_locked(b, False)
    if a.lock_rest:
        nlock = kh.set_tracks_locked(b, True)
    st = kh.stats(b)
    if a.save_prepped:
        kh.save(b, a.save_prepped)
    # ---- export-only edits (after the prepped board was saved) ----
    dropped = []
    for z in list(b.Zones()):
        if z.GetIsRuleArea():
            soft = (not z.GetDoNotAllowTracks()) and (not z.GetDoNotAllowVias())
            if soft and not a.keep_soft_keepouts:
                dropped.append("rule area '%s'" % z.GetZoneName())
                b.Delete(z)   # Remove() leaves a dangling SWIG proxy that breaks later calls
        elif a.drop_signal_zones:
            lays = [b.GetLayerType(l) for l in z.GetLayerSet().Seq()]
            if all(t != pcbnew.LT_POWER for t in lays):
                dropped.append("zone '%s' (%s)" % (z.GetZoneName(), z.GetNetname()))
                b.Delete(z)
    for d in dropped:
        print("export copy: dropped", d)
    t0 = time.time()
    ok = pcbnew.ExportSpecctraDSN(b, a.dsn)
    dt = time.time() - t0
    if not ok:
        raise SystemExit("ExportSpecctraDSN failed")
    print("ripped items: %d, locked: %d, unrouted after rip: %d, DSN %s (%.2f s, %d bytes)" % (
        nrip, nlock, st["unrouted_connections"], a.dsn, dt, os.path.getsize(a.dsn)))


if __name__ == "__main__":
    main()
