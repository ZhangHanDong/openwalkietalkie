#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run Freerouting 2.4.1 headless on a DSN and write a SES.

usage: python3 fr_route.py IN.dsn OUT.ses [--passes 30] [--threads 1] [--timeout 900]
                            [--no-optimize] [--extra "--router.via_costs=80 ..."]

Flags verified against freerouting-2.4.1.jar (GlobalSettings.applyCommandLineArguments):
  -de FILE   input .dsn             -do FILE  output .ses (or .dsn/.scr)
  -mp N      max autorouter passes   -mt N     optimizer threads (0/1 = single)
  -oit PCT   optimizer improvement threshold
  -us greedy|global|hybrid  -hr m:n  -is sequential|random|prioritized
  -inc CLASS,CLASS  ignore these net classes -- only honoured by the GUI path, NO effect
             headless in 2.4.1 (verified); use dsn_tweak.py --isolate instead
  -drc FILE  DRC-only mode: write a KiCad-schema DRC JSON and exit (no routing)
  -da        disable analytics (always pass it)
  -ll LEVEL  console log level          -dl  disable file logging
  --<settings.path>=value   any freerouting.json key, e.g. --gui.enabled=false,
                            --router.max_passes=30, --router.optimizer.enabled=false,
                            --router.fanout.enabled=false, --router.automatic_neckdown=false,
                            --router.via_costs=50, --router.max_threads=1
                            (unknown keys only log "Unknown settings property")
  Env overrides: FREEROUTING__<SECTION>__<KEY>=value (e.g. FREEROUTING__GUI__ENABLED=false)
Settings are loaded from ~/Library/Application Support/freerouting/freerouting.json; the
--user_data_path=DIR flag points that (and the log) at a private dir so runs do not
touch the user's config.
"""
import argparse
import os
import re
import shlex
import subprocess
import sys
import time

JAVA = os.path.expanduser("~/.cohdl-dev-runtime/eda-tools/java/jdk-25.0.4.1+1-jre/Contents/Home/bin/java")
JAR = os.path.expanduser("~/.cohdl-dev-runtime/eda-tools/freerouting-2.4.1.jar")
HERE = os.path.dirname(os.path.abspath(__file__))


def run(dsn, ses, passes=30, threads=1, timeout=900, optimize=True, extra=(), userdir=None, drc=None,
        fanout=False, neckdown=False):
    userdir = userdir or os.path.join(HERE, "fr-userdata")
    os.makedirs(userdir, exist_ok=True)
    cmd = [JAVA, "-Djava.awt.headless=true", "-jar", JAR,
           "-de", dsn, "-do", ses, "-mp", str(passes), "-mt", str(threads), "-da",
           "--gui.enabled=false", "--user_data_path=" + userdir]
    if not optimize:
        cmd.append("--router.optimizer.enabled=false")
    if not fanout:
        # the fan-out stage adds stubs+vias to SMD pins of ANY net it thinks is open
        cmd.append("--router.fanout.enabled=false")
    if not neckdown:
        # automatic neck-down narrows traces at pads to 0.09 / 0.072 mm (< r6 min 0.10):
        # 41 KiCad track_width errors in the full-board test
        cmd.append("--router.automatic_neckdown=false")
    if drc:
        cmd += ["-drc", drc]
    cmd += list(extra)
    log = os.path.splitext(ses)[0] + ".fr.log"
    t0 = time.time()
    with open(log, "w") as fh:
        fh.write(" ".join(shlex.quote(c) for c in cmd) + "\n")
        fh.flush()
        try:
            p = subprocess.run(cmd, stdout=fh, stderr=subprocess.STDOUT, timeout=timeout)
            rc = p.returncode
        except subprocess.TimeoutExpired:
            rc = "timeout"
    dt = time.time() - t0
    txt = open(log).read()
    unrouted = re.findall(r"(\d+) unrouted", txt)
    print("freerouting rc=%s in %.1f s; SES %s (%s); log %s" % (
        rc, dt, ses, "%d bytes" % os.path.getsize(ses) if os.path.exists(ses) else "MISSING", log))
    for line in txt.splitlines():
        if re.search(r"Auto-routing was completed|pass #|unrouted|Optimization was completed|"
                     r"incomplete|violation|ERROR|WARN", line):
            print("  " + line[:200])
    return rc, dt, log


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dsn")
    ap.add_argument("ses")
    ap.add_argument("--passes", type=int, default=30)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--no-optimize", action="store_true")
    ap.add_argument("--drc")
    ap.add_argument("--fanout", action="store_true", help="enable Freerouting's SMD fan-out stage")
    ap.add_argument("--neckdown", action="store_true", help="allow automatic neck-down (default off)")
    ap.add_argument("--extra", default="", help='pass as --extra="--router.via_costs=80 ..."')
    a = ap.parse_args()
    rc, _, _ = run(a.dsn, a.ses, a.passes, a.threads, a.timeout, not a.no_optimize,
                   shlex.split(a.extra), drc=a.drc, fanout=a.fanout, neckdown=a.neckdown)
    sys.exit(0 if rc == 0 and os.path.exists(a.ses) else 1)


if __name__ == "__main__":
    main()
