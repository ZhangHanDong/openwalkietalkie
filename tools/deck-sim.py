#!/usr/bin/env python3
"""Stand-in deck host for channel-5 testing against real hardware.

Speaks the wire contract in docs/deck-protocol.md over the vendor HID
collection (report id 6, channel byte 5): says hello and keeps the 5 s
heartbeat going, performs the full sync of the mockup roster when the device
asks for one, optionally floods an agent title the way a spinner would, and
prints every device event it receives, decoded. It also answers like a daemon
would: an `answer` clears the card and puts the agent to work, `refresh`
nudges the usage bars, `ptt` types a transcript and replies, `channel` is
acknowledged with `focus`.

Usage:
    tools/deck-sim.py                      run against the first OpenWalkieTalkie found
    tools/deck-sim.py --dry-run            print the reports instead (no hardware, no hidapi)
    tools/deck-sim.py --flood 200          after the sync, 200 title updates at --pace
    tools/deck-sim.py --send '{"m":"set","k":"brightness","v":2}'   extra lines after the sync
    tools/deck-sim.py --seconds 30         stop after 30 s (default: until Ctrl-C)

Needs the `hid` module (pip install hidapi) unless --dry-run. Reports are
paced at --pace ms (default 8, the contract's minimum) so the device's queue
never overflows.
"""

import argparse
import json
import random
import sys
import time
from datetime import datetime

VID, PID = 0x1209, 0x5754
REPORT_ID = 6
CHANNEL = 5
PAYLOAD_MAX = 61
LINE_MAX = 1024

# The mockup roster, verbatim (src/deck/demo.rs is the reference).
ROSTER = [
    {"m": "agent", "id": 1, "kind": "codex", "name": "cohdl-api", "proj": "cohdl", "state": "question",
     "title": "Run migrations on staging?", "since": 120},
    {"m": "card", "agent": 1, "id": 1, "type": "q",
     "text": "Tests fail on main. Run pending migrations on staging first?",
     "choices": ["Run on staging", "Skip migrations", "Abort task"], "rec": 0},
    {"m": "reply", "agent": 1,
     "text": "Tests fail on main: 3 pending migrations are not applied on staging.", "rev": 1},
    {"m": "agent", "id": 2, "kind": "claude", "name": "web-ui", "proj": "cohdl", "state": "ready",
     "title": "Header refactor done · 34 files"},
    {"m": "card", "agent": 2, "id": 2, "type": "r", "text": "Header refactor done · 34 files",
     "choices": ["Run the test suite", "Open a pull request"]},
    {"m": "reply", "agent": 2, "text": "Header refactor done, 34 files changed", "rev": 1},
    {"m": "agent", "id": 3, "kind": "codex", "name": "firmware", "proj": "cohdl", "state": "working",
     "title": "Compiling openmicro", "since": 120},
    {"m": "reply", "agent": 3, "text": "Building the native image for both boards.", "rev": 1},
    {"m": "agent", "id": 4, "kind": "gemini", "name": "research", "proj": "openmicro", "state": "idle",
     "title": "Idle"},
    {"m": "agent", "id": 5, "kind": "claude", "name": "infra", "proj": "openmicro", "state": "error",
     "title": "Deploy failed · exit 1"},
    {"m": "reply", "agent": 5, "text": "terraform apply exited with status 1.", "rev": 1},
    {"m": "ws", "active": 2, "count": 4, "sel": 0, "windows": [
        {"c": "blue", "t": "term · claude — header refactor"},
        {"c": "amber", "t": "firefox — github · pulls"},
        {"c": "green", "t": "term · codex — compiling..."}]},
    {"m": "macro", "f": 3, "src": "deck.toml", "keys": [
        {"l": "build", "b": "left"}, {"l": "test", "b": "right"}, {"l": "deploy", "b": "enter"},
        {"l": "git push", "b": "shift+left"}, {"l": "screenshot"}, {"l": "lock"},
        {"l": "mute", "b": "shift+right"}, {"l": "term", "b": "shift+enter"}]},
    {"m": "usage", "period": "SEP 2026", "items": [
        {"n": "Claude Pro", "p": 62, "r": "resets in 4 days"},
        {"n": "Codex · OpenAI", "p": 81, "r": "resets tomorrow"},
        {"n": "Gemini", "p": 12, "r": "resets in 11 days"}]},
]
DEFAULT_FACES = [
    {"f": 1, "t": "agents", "n": "Agents"}, {"f": 2, "t": "omarchy", "n": "Omarchy"},
    {"f": 3, "t": "macropad", "n": "Macropad"}, {"f": 4, "t": "usage", "n": "Usage"},
    {"f": 6, "t": "arcade", "n": "Arcade"},
    {"f": 5, "t": "settings", "n": "Settings"},
]
SPINNER = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"


def encode(msg):
    """One compact line, raw UTF-8 (ensure_ascii would inflate non-ASCII 6x)."""
    line = json.dumps(msg, separators=(",", ":"), ensure_ascii=False).encode() + b"\n"
    if len(line) > LINE_MAX:
        raise ValueError(f"line of {len(line)} bytes exceeds {LINE_MAX}")
    return line


def frames(line):
    """Report bodies (with the report id) carrying `line`."""
    for at in range(0, len(line), PAYLOAD_MAX):
        chunk = line[at:at + PAYLOAD_MAX]
        yield bytes([REPORT_ID, CHANNEL, len(chunk)]) + chunk + bytes(PAYLOAD_MAX - len(chunk))


class LineAssembler:
    """The device's line assembler, mirrored for the host side."""

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


class Link:
    def __init__(self, args):
        self.dry = args.dry_run
        self.pace = args.pace / 1000.0
        self.dev = None
        self.assembler = LineAssembler()
        self.sent = 0
        if not self.dry:
            try:
                import hid
            except ImportError:
                sys.exit("the hid module is missing: pip install hidapi, or use --dry-run")
            self.dev = hid.device()
            self.dev.open(args.vid, args.pid)
            self.dev.set_nonblocking(True)
            print(f"opened {self.dev.get_manufacturer_string()} {self.dev.get_product_string()}")

    def send(self, msg):
        line = encode(msg)
        for report in frames(line):
            if self.dry:
                print(f"  > {report[:3].hex()} {report[3:3 + report[2]]!r}")
                # Prove the framing by running it back through the assembler.
                for got in self.assembler.push(report):
                    print(f"  = {got.decode(errors='replace') if got else '<oversized>'}")
            else:
                self.dev.write(report)
            self.sent += 1
            time.sleep(self.pace)
        print(f"-> {line[:-1].decode()}")

    def poll(self, timeout_s):
        """Decoded device messages that arrived within `timeout_s`."""
        deadline = time.monotonic() + timeout_s
        while True:
            report = self.dev.read(64) if self.dev else []
            if report:
                for got in self.assembler.push(bytes(report)):
                    if got is None:
                        print("<- <oversized line dropped>")
                        continue
                    try:
                        yield json.loads(got.decode())
                    except (ValueError, UnicodeDecodeError) as err:
                        print(f"<- undecodable {got!r}: {err}")
            elif time.monotonic() >= deadline:
                return
            else:
                time.sleep(0.002)


class Host:
    def __init__(self, link, args):
        self.link = link
        self.args = args
        self.sid = args.sid if args.sid else random.randrange(1, 2**32)
        self.hm = args.hm
        self.seq = 0
        self.working = {}
        self.usage_nudge = 0
        self.last_hello = 0.0
        self.synced = False

    def hello(self):
        now = datetime.now()
        hm = self.hm or f"{now.hour:02}:{now.minute:02}"
        self.link.send({"m": "hello", "v": 1, "sid": self.sid, "hm": hm})
        self.last_hello = time.monotonic()

    def sync(self, faces=None):
        print("== full sync")
        for msg in ROSTER:
            self.link.send(msg)
        if faces is not None:
            self.link.send({"m": "faces", "list": faces})
        self.link.send({"m": "synced"})
        self.synced = True

    def flood(self, count):
        print(f"== title flood: {count} updates")
        for i in range(count):
            self.link.send({"m": "agent", "id": 3, "title": f"Compiling openmicro {SPINNER[i % len(SPINNER)]} {i}"})

    def on_event(self, msg):
        m = msg.get("m")
        print(f"<- {json.dumps(msg, ensure_ascii=False)}")
        if m == "hello":
            # The device holds our session only after `synced`; 0 means resync.
            if msg.get("sid") != self.sid or not self.synced:
                self.sync(DEFAULT_FACES if msg.get("boot") else None)
        elif m == "answer":
            agent, card = msg.get("agent"), msg.get("card")
            self.link.send({"m": "card", "agent": agent, "id": card, "clear": 1})
            self.link.send({"m": "agent", "id": agent, "state": "working",
                            "title": f"Working on choice {msg.get('choice')}"})
        elif m == "channel":
            self.link.send({"m": "focus", "agent": msg.get("agent"), "ack": msg.get("seq")})
        elif m == "refresh":
            self.usage_nudge += 1
            items = [dict(item, p=min(100, item["p"] + self.usage_nudge)) for item in ROSTER[-1]["items"]]
            self.link.send({"m": "usage", "period": ROSTER[-1]["period"], "items": items})
        elif m == "ptt" and msg.get("act") == 1:
            self.working["ptt"] = msg
        elif m == "ptt" and msg.get("act") == 0:
            press = self.working.pop("ptt", msg)
            if msg.get("abort"):
                return
            if press.get("ctx") == "name":
                self.link.send({"m": "name", "text": "Deploy"})
            elif press.get("agent"):
                text = "Refactor the auth module, then run the test suite."
                for end in range(8, len(text) + 8, 8):
                    self.link.send({"m": "transcript", "agent": press["agent"], "text": text[:end]})
                self.link.send({"m": "transcript", "agent": press["agent"], "text": text, "final": 1})
                self.link.send({"m": "reply", "agent": press["agent"], "text": "On it.", "rev": 1})
                self.link.send({"m": "agent", "id": press["agent"], "state": "working", "title": "Refactoring auth"})
        elif m == "err":
            print(f"!! device error: {msg.get('why')}")

    def run(self):
        self.hello()
        if self.link.dry:
            self.sync(DEFAULT_FACES)
        if self.args.flood:
            if not self.link.dry:
                self.drain(2.0)
            self.flood(self.args.flood)
        for line in self.args.send:
            self.link.send(json.loads(line))
        if self.link.dry:
            print(f"dry run: {self.link.sent} reports")
            return
        stop = time.monotonic() + self.args.seconds if self.args.seconds else None
        try:
            while stop is None or time.monotonic() < stop:
                self.drain(0.25)
                if time.monotonic() - self.last_hello >= 5.0:
                    self.hello()
        except KeyboardInterrupt:
            pass
        print(f"{self.link.sent} reports sent")

    def drain(self, seconds):
        for msg in self.link.poll(seconds):
            self.on_event(msg)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--dry-run", action="store_true", help="print the reports instead of sending them")
    parser.add_argument("--vid", type=lambda s: int(s, 0), default=VID)
    parser.add_argument("--pid", type=lambda s: int(s, 0), default=PID)
    parser.add_argument("--pace", type=float, default=8.0, help="ms between reports (default 8)")
    parser.add_argument("--flood", type=int, default=0, metavar="N", help="send N spinner title updates after the sync")
    parser.add_argument("--send", action="append", default=[], metavar="JSON", help="extra line(s) to send after the sync")
    parser.add_argument("--seconds", type=float, default=0, help="run time (default: until Ctrl-C)")
    parser.add_argument("--sid", type=int, default=0, help="session id (default: random); fixed for the deckd parity diff")
    parser.add_argument("--hm", default=None, metavar="HH:MM", help="header clock (default: now); fixed for the parity diff")
    args = parser.parse_args()
    if args.dry_run:
        args.pace = 0
    Host(Link(args), args).run()


if __name__ == "__main__":
    main()
