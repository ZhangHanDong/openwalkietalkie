#!/usr/bin/env python3
"""S3 probe: one non-seizing IOKit client of the deck that counts what it receives.

Opens the deck (VID 1209 / PID 5754) through python hidapi in NON-exclusive mode
(the cython module exports `hid_darwin_set_open_exclusive`, reached via ctypes;
hidapi's `hid_init` resets the flag to exclusive, so `hid.enumerate()` runs first),
then for --seconds:

  * optionally sends a channel-5 `hello` every --hello seconds (0 = never),
  * optionally sends a title burst (--burst N reports at --burst-ms pace, starting
    at --burst-at seconds) the way a spinner flood would,
  * counts every input report by channel, reassembles channel-5 lines and counts
    them by `m`, measures hello round trips, and records read/write errors.

Run two or three copies at once (different --name, one with --hello, the others
listen-only) and diff the per-second counts: identical channel-5/-3 counts mean
IOKit delivers every input report to every non-seizing client.

Usage:
  host/spikes/hid_probe.py --name A --seconds 60 --hello 1 --log /tmp/A.log
  host/spikes/hid_probe.py --name B --seconds 60 --hello 0 --log /tmp/B.log
  host/spikes/hid_probe.py --name A --seconds 30 --hello 1 --burst 500 --burst-ms 8 --burst-at 5

The last line of the log (and stdout) is a JSON summary.
"""

import argparse
import ctypes
import json
import statistics
import sys
import time
from datetime import datetime

VID, PID = 0x1209, 0x5754
REPORT_ID = 6
CHANNEL = 5
PAYLOAD_MAX = 61
LINE_MAX = 1024
SPINNER = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"


def encode(msg):
    return json.dumps(msg, ensure_ascii=False, separators=(",", ":")).encode() + b"\n"


def frames(line):
    for off in range(0, len(line), PAYLOAD_MAX):
        chunk = line[off:off + PAYLOAD_MAX]
        yield bytes([REPORT_ID, CHANNEL, len(chunk)]) + chunk + bytes(64 - 3 - len(chunk))


class LineAssembler:
    """Copy of tools/deck-sim.py's assembler (LF-delimited, 1024 B cap)."""

    def __init__(self):
        self.buf = bytearray()
        self.discarding = False

    def push(self, report):
        body = report[1:] if len(report) >= 2 and report[0] == REPORT_ID and report[1] == CHANNEL else report
        if len(body) < 2 or body[0] != CHANNEL:
            return
        for byte in bytes(body[2:2 + min(body[1], PAYLOAD_MAX)]):
            if byte == 0x0A:
                if self.discarding:
                    yield None
                elif self.buf:
                    yield bytes(self.buf)
                self.buf.clear()
                self.discarding = False
            elif len(self.buf) + 1 >= LINE_MAX:
                self.discarding = True
            elif not self.discarding:
                self.buf.append(byte)


def open_shared(vid, pid, retries, retry_s):
    import hid  # noqa: WPS433 (pip install hidapi)

    hid.enumerate()  # forces hid_init(), which resets the flag to exclusive
    lib = ctypes.CDLL(hid.__file__)
    lib.hid_darwin_set_open_exclusive(0)
    if lib.hid_darwin_get_open_exclusive() != 0:
        sys.exit("could not switch hidapi to non-exclusive opens")
    for attempt in range(retries + 1):
        try:
            dev = hid.device()
            dev.open(vid, pid)
            dev.set_nonblocking(True)
            return dev, lib
        except OSError as err:
            if attempt == retries:
                raise
            print(f"open failed ({err}); device busy? retry {attempt + 1}/{retries} in {retry_s}s", flush=True)
            time.sleep(retry_s)
    raise AssertionError("unreachable")


class Probe:
    def __init__(self, args):
        self.args = args
        self.log = open(args.log, "a") if args.log else None
        self.dev, self.lib = open_shared(VID, PID, args.retries, args.retry_s)
        self.assembler = LineAssembler()
        self.t0 = time.monotonic()
        self.per_channel = {}
        self.per_m = {}
        self.reports_in = 0
        self.bad_lines = 0
        self.hello_sent = 0
        self.hello_replies = 0
        self.last_hello_at = None
        self.rtts = []
        self.write_errors = 0
        self.write_ms = []
        self.read_errors = 0
        self.reports_out = 0
        self.burst_done = False
        self.burst_started = None
        self.burst_ended = None
        self.seconds = []  # per-second rows

    def out(self, text):
        stamp = f"[{self.args.name} t={time.monotonic() - self.t0:7.3f} {datetime.now().strftime('%H:%M:%S.%f')[:-3]}]"
        line = f"{stamp} {text}"
        print(line, flush=True)
        if self.log:
            self.log.write(line + "\n")
            self.log.flush()

    def write(self, report):
        # hidapi's macOS write is a synchronous IOHIDDeviceSetReport with no timeout: it
        # returns only when the BLE write-with-response completes, so its latency IS the
        # link's back-pressure (and a stall here is a link that stopped acknowledging).
        started = time.monotonic()
        try:
            n = self.dev.write(report)
            if n != 64:
                self.write_errors += 1
                self.out(f"short write {n}")
            self.reports_out += 1
        except OSError as err:
            self.write_errors += 1
            self.out(f"write error: {err}")
        took = (time.monotonic() - started) * 1000.0
        self.write_ms.append(took)
        if took > self.args.slow_write_ms:
            self.out(f"slow write #{self.reports_out}: {took:.1f} ms")

    def send(self, msg):
        for report in frames(encode(msg)):
            self.write(report)

    def hello(self):
        now = datetime.now()
        self.send({"m": "hello", "v": 1, "sid": self.args.sid, "hm": f"{now.hour:02}:{now.minute:02}"})
        self.hello_sent += 1
        self.last_hello_at = time.monotonic()

    def burst(self):
        n, pace = self.args.burst, self.args.burst_ms / 1000.0
        self.out(f"burst: {n} title reports at {self.args.burst_ms} ms")
        self.send({"m": "agent", "id": 3, "kind": "codex", "name": "firmware", "state": "working",
                   "title": "Compiling openmicro"})
        self.burst_started = time.monotonic()
        next_at = self.burst_started
        sent = 0
        for i in range(n):
            if self.args.burst_kind == "hello":
                line = encode({"m": "hello", "v": 1, "sid": self.args.sid, "hm": "12:00"})
                self.hello_sent += 1
                self.last_hello_at = time.monotonic()
            else:
                line = encode({"m": "agent", "id": 3, "title": f"Compiling openmicro {SPINNER[i % len(SPINNER)]} {i}"})
            for report in frames(line):
                while time.monotonic() < next_at:
                    time.sleep(0.0005)
                self.write(report)
                sent += 1
                next_at += pace
                if sent % 50 == 0:
                    self.out(f"burst progress {sent}/{n}: {(time.monotonic() - self.burst_started):.3f}s, "
                             f"in={self.reports_in} lines={self.per_m}")
            self.drain()
        self.burst_ended = time.monotonic()
        self.out(f"burst done in {self.burst_ended - self.burst_started:.3f}s, write errors so far {self.write_errors}")

    def drain(self):
        while True:
            try:
                report = self.dev.read(64)
            except OSError as err:
                self.read_errors += 1
                self.out(f"read error: {err}")
                return
            if not report:
                return
            report = bytes(report)
            self.reports_in += 1
            ch = report[1] if report[0] == REPORT_ID else report[0]
            self.per_channel[ch] = self.per_channel.get(ch, 0) + 1
            if ch != CHANNEL:
                continue
            for got in self.assembler.push(report):
                if got is None:
                    self.bad_lines += 1
                    continue
                try:
                    msg = json.loads(got.decode())
                except (ValueError, UnicodeDecodeError):
                    self.bad_lines += 1
                    continue
                m = str(msg.get("m"))
                self.per_m[m] = self.per_m.get(m, 0) + 1
                if m == "hello":
                    self.hello_replies += 1
                    if self.last_hello_at is not None:
                        self.rtts.append((time.monotonic() - self.last_hello_at) * 1000.0)
                elif m == "err":
                    self.out(f"device err: {msg.get('why')}")
                elif self.args.verbose:
                    self.out(f"<- {got.decode(errors='replace')}")

    def run(self):
        a = self.args
        self.out(f"opened non-exclusive={self.lib.hid_darwin_get_open_exclusive() == 0} hello={a.hello}s burst={a.burst}")
        next_hello = self.t0
        next_row = self.t0 + 1.0
        row_prev = (0, {}, 0)
        while True:
            now = time.monotonic()
            if now - self.t0 >= a.seconds:
                break
            if a.hello > 0 and now >= next_hello:
                self.hello()
                next_hello += a.hello
            if a.burst and not self.burst_done and now - self.t0 >= a.burst_at:
                self.burst_done = True
                self.burst()
            self.drain()
            if now >= next_row:
                sec = int(round(next_row - self.t0))
                d_in = self.reports_in - row_prev[0]
                d_ch = {k: self.per_channel.get(k, 0) - row_prev[1].get(k, 0) for k in self.per_channel}
                d_hello = self.hello_replies - row_prev[2]
                self.seconds.append({"s": sec, "in": d_in, "ch": d_ch, "hello_replies": d_hello})
                self.out(f"s={sec:3d} in={d_in:4d} ch={d_ch} hello_replies={d_hello} total_in={self.reports_in}")
                row_prev = (self.reports_in, dict(self.per_channel), self.hello_replies)
                next_row += 1.0
            time.sleep(0.001)
        self.dev.close()
        import hid

        present = any(d["vendor_id"] == VID and d["product_id"] == PID for d in hid.enumerate())
        summary = {
            "name": a.name,
            "seconds": a.seconds,
            "reports_in": self.reports_in,
            "per_channel": {str(k): v for k, v in sorted(self.per_channel.items())},
            "lines_by_m": self.per_m,
            "bad_lines": self.bad_lines,
            "hello_sent": self.hello_sent,
            "hello_replies": self.hello_replies,
            "rtt_ms": {
                "n": len(self.rtts),
                "min": round(min(self.rtts), 1) if self.rtts else None,
                "median": round(statistics.median(self.rtts), 1) if self.rtts else None,
                "max": round(max(self.rtts), 1) if self.rtts else None,
            },
            "reports_out": self.reports_out,
            "write_ms": {
                "n": len(self.write_ms),
                "median": round(statistics.median(self.write_ms), 1) if self.write_ms else None,
                "max": round(max(self.write_ms), 1) if self.write_ms else None,
                "over_8ms": sum(1 for x in self.write_ms if x > 8.0),
            },
            "write_errors": self.write_errors,
            "read_errors": self.read_errors,
            "burst": None if not a.burst else {
                "count": a.burst, "pace_ms": a.burst_ms,
                "duration_s": round(self.burst_ended - self.burst_started, 3) if self.burst_ended else None,
            },
            "device_present_after": present,
            "per_second": self.seconds,
        }
        self.out("SUMMARY " + json.dumps(summary, ensure_ascii=False))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--name", default="probe")
    ap.add_argument("--seconds", type=float, default=60.0)
    ap.add_argument("--hello", type=float, default=1.0, help="hello period in s (0 = listen only)")
    ap.add_argument("--sid", type=int, default=314159)
    ap.add_argument("--burst", type=int, default=0, help="send N title reports (one line each)")
    ap.add_argument("--burst-ms", type=float, default=8.0)
    ap.add_argument("--burst-at", type=float, default=5.0)
    ap.add_argument("--slow-write-ms", type=float, default=100.0, help="log writes slower than this")
    ap.add_argument("--burst-kind", choices=["title", "hello"], default="title",
                    help="title = silent agent updates; hello = every report gets a 6-report reply")
    ap.add_argument("--log", default=None)
    ap.add_argument("--retries", type=int, default=10)
    ap.add_argument("--retry-s", type=float, default=3.0)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()
    Probe(args).run()


if __name__ == "__main__":
    main()
