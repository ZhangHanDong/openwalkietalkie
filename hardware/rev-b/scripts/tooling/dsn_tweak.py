#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Post-process a KiCad-exported Specctra DSN before Freerouting (stdlib only).

usage: python3 dsn_tweak.py IN.dsn OUT.dsn --route-nets PTT,ENTER,... [--snap45]
          [--class-width CLASS=um ...]
Prints the comma-separated list of the OTHER net classes, which is what
Freerouting's -inc (ignore net classes) flag wants.

--route-nets  move these nets out of their KiCad class into a twin class
              "FR_<class>" with identical circuit/rule, so every other class
              can be ignored by the autorouter (-inc). Without this Freerouting
              also 'repairs' every net it thinks is open (on r6: 130 nets, incl.
              GND stubs and fan-out vias on FLASH_*, see tooling.md).
--snap45      add (snap_angle fortyfive_degree) to (structure) (Freerouting's
              default is already 45-degree on KiCad DSNs; kept for explicitness).
"""
import argparse
import re
import sys


def tokenize(s):
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c in " \t\r\n":
            i += 1
        elif c in "()":
            yield c
            i += 1
        elif c == '"':
            j = s.index('"', i + 1)
            yield s[i:j + 1]
            i = j + 1
        else:
            j = i
            while j < n and s[j] not in ' \t\r\n()':
                j += 1
            yield s[i:j]
            i = j


def parse(s):
    stack = [[]]
    toks = tokenize(s)
    # the "(string_quote \")" form contains a bare quote char: handle it
    prev = None
    for t in toks:
        if t == "(":
            stack.append([])
        elif t == ")":
            x = stack.pop()
            stack[-1].append(x)
        else:
            stack[-1].append(t)
        prev = t
    return stack[0][0]


def dump(x, ind=0):
    if not isinstance(x, list):
        return x
    if all(not isinstance(e, list) for e in x):
        return "(" + " ".join(x) + ")"
    pad = "  " * (ind + 1)
    head = []
    rest = list(x)
    while rest and not isinstance(rest[0], list):
        head.append(rest.pop(0))
    return "(" + " ".join(head) + "".join("\n" + pad + dump(e, ind + 1) for e in rest) + ")"


def find(x, key):
    return [e for e in x if isinstance(e, list) and e and e[0] == key]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dsn_in")
    ap.add_argument("dsn_out")
    ap.add_argument("--route-nets", default="")
    ap.add_argument("--snap45", action="store_true")
    ap.add_argument("--width", default="",
                    help="override the routed (FR_*) classes' track width, um (e.g. 104 like r6's QFN escapes)")
    ap.add_argument("--clearance", default="", help="override the FR_* classes' clearance, um")
    ap.add_argument("--isolate", action="store_true",
                    help="drop the (pins ...) of every non-target net so Freerouting has nothing "
                         "else to route (its pads become no-net obstacles, its fixed wires stay as "
                         "net-owned obstacles). This is what actually restricts the job; -inc is "
                         "ignored by the headless 2.4.1 runner.")
    a = ap.parse_args()
    raw = open(a.dsn_in).read()
    # KiCad writes (string_quote ") which is not a balanced token: protect it.
    raw = raw.replace('(string_quote ")', "(string_quote QUOTECHAR)")
    pcb = parse(raw)
    targets = set(n for n in a.route_nets.split(",") if n)
    targets_q = targets | set('"%s"' % n for n in targets)
    network = find(pcb, "network")[0]
    classes = find(network, "class")
    other, moved = [], set()
    for cl in list(classes):
        name = cl[1]
        nets = [e for e in cl[2:] if not isinstance(e, list)]
        body = [e for e in cl[2:] if isinstance(e, list)]
        hit = [n for n in nets if n in targets_q]
        keep = [n for n in nets if n not in targets_q]
        if hit:
            moved |= set(hit)
            import copy
            body2 = copy.deepcopy(body)
            for blk in body2:
                if blk[0] == "rule":
                    for r in blk[1:]:
                        if a.width and r[0] == "width":
                            r[1] = a.width
                        if a.clearance and r[0] == "clearance" and len(r) == 2:
                            r[1] = a.clearance
            twin = ["class", "FR_" + name] + hit + body2
            network.insert(network.index(cl) + 1, twin)
            if keep:
                cl[:] = [cl[0], name] + keep + body
            else:
                network.remove(cl)
                continue
        other.append(name)
    missing = targets - set(n.strip('"') for n in moved)
    if missing:
        sys.exit("nets not found in any class: %s" % ",".join(sorted(missing)))
    if a.isolate and targets:
        # 1) non-target nets lose their pins -> their pads become no-net obstacles
        for net in find(network, "net"):
            if net[1] not in targets_q:
                net[:] = [e for e in net if not (isinstance(e, list) and e and e[0] == "pins")]
        # 2) non-target fixed wires/vias lose their (net X) -> anonymous obstacles.
        #    Otherwise Freerouting counts every KiCad junction it cannot see as
        #    connected (T-joins off an endpoint, overlapping stubs) as an open
        #    connection and tries to 'repair' it (r6: 130 nets / 44 items).
        wiring = find(pcb, "wiring")
        nstrip = 0
        for w in (wiring[0] if wiring else []):
            if isinstance(w, list) and w and w[0] in ("wire", "via"):
                nn = find(w, "net")
                if nn and nn[0][1] not in targets_q:
                    w.remove(nn[0])
                    nstrip += 1
        sys.stderr.write("isolate: anonymised %d wires/vias\n" % nstrip)
    if a.snap45:
        st = find(pcb, "structure")[0]
        if not find(st, "snap_angle"):
            st.append(["snap_angle", "fortyfive_degree"])
    out = dump(pcb).replace("(string_quote QUOTECHAR)", '(string_quote ")')
    open(a.dsn_out, "w").write(out + "\n")
    print(",".join(other))


if __name__ == "__main__":
    main()
