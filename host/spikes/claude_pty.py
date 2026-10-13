#!/usr/bin/env python3
"""S1/S2 driver: runs an interactive `claude` in a pty and records what it draws.

Standard library only (no pexpect): forks `claude` on a pty of --cols x --rows,
strips ANSI/OSC sequences from everything it draws and appends it to --log with
wall-clock stamps (raw bytes go to --log.raw). It accepts the workspace trust
dialog and "Press Enter to continue" screens by itself, types --prompt once the
input box is up, then watches the screen for --watch regexes and logs the first
time each one appears (and how often). `--at T:KEYS` sends KEYS (python escapes,
e.g. '\\r' or '\\x1b') T seconds after the prompt was sent, to answer a dialog
locally while a hook is held. After --seconds it sends Ctrl-C twice and kills
the process group.

`--settings` is passed straight to `claude --settings` (a file or inline JSON):
that is how the hooks are installed without touching ~/.claude/settings.json.

Usage:
  host/spikes/claude_pty.py --cwd /tmp/spk/proj --settings /tmp/spk/settings.json \\
      --prompt 'Use the Bash tool to run: date' --seconds 90 \\
      --watch 'Do you want to proceed' --watch 'Yes' --log /tmp/spk/pty.log
"""

import argparse
import codecs
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time
from datetime import datetime

OSC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
CURSOR_FWD = re.compile(rb"\x1b\[(\d*)C")
# Ink draws gaps between words as cursor moves, so movement sequences become a space and
# SGR/mode sequences vanish; everything else non-printable is dropped.
CSI_MOVE = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[A-HJKSTdfr]")
CSI_OTHER = re.compile(rb"\x1b[\[\?][0-9;?<=>]*[ -/]*[@-~]|\x1b[()][A-Za-z0-9]|\x1b[@-Z\\-_]|[\x00-\x08\x0b-\x1f\x7f]")


def strip(data):
    data = OSC.sub(b"", data)
    data = CURSOR_FWD.sub(lambda m: b" " * int(m.group(1) or b"1"), data)
    data = CSI_MOVE.sub(b" ", data)
    text = CSI_OTHER.sub(b"", data).decode("utf-8", errors="replace")
    return re.sub(r"[ \t]+", " ", text)


class Session:
    def __init__(self, args):
        self.args = args
        self.log = open(args.log, "a")
        self.raw = open(args.log + ".raw", "ab")
        self.t0 = time.monotonic()
        self.screen = ""
        self.seen = {}
        self.prompt_sent_at = None
        self.trusted = False
        self.pending_at = sorted((float(t), codecs.decode(k, "unicode_escape")) for t, k in
                                 (item.split(":", 1) for item in args.at))

    def out(self, text):
        stamp = f"[pty t={time.monotonic() - self.t0:7.3f} {datetime.now().strftime('%H:%M:%S.%f')[:-3]}]"
        line = f"{stamp} {text}"
        self.log.write(line + "\n")
        self.log.flush()
        if self.args.echo:
            print(line, flush=True)

    def spawn(self):
        env = dict(os.environ)
        for key in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"):
            env.pop(key, None)
        env["TERM"] = "xterm-256color"
        if self.args.config_dir:
            env["CLAUDE_CONFIG_DIR"] = self.args.config_dir
        argv = ["claude"] + self.args.claude_args
        if self.args.settings:
            argv += ["--settings", self.args.settings]
        if self.args.model:
            argv += ["--model", self.args.model]
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(self.args.cwd)
            os.execvpe(argv[0], argv, env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", self.args.rows, self.args.cols, 0, 0))
        self.pid, self.fd = pid, fd
        self.out(f"spawned pid={pid} argv={argv} cwd={self.args.cwd}")

    def send(self, keys, why):
        os.write(self.fd, keys.encode())
        self.out(f"SEND {keys!r} ({why})")

    def run(self):
        self.spawn()
        deadline = self.t0 + self.args.seconds
        last_activity = time.monotonic()
        while time.monotonic() < deadline:
            r, _, _ = select.select([self.fd], [], [], 0.1)
            now = time.monotonic()
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    self.out("pty closed")
                    break
                if not data:
                    self.out("EOF")
                    break
                last_activity = now
                self.raw.write(data)
                self.raw.flush()
                text = strip(data)
                self.screen = (self.screen + text)[-20000:]
                compact = re.sub(r"\s+", " ", text).strip()
                if compact:
                    self.out(f"DRAW {compact[:self.args.draw_chars]}")
                self.check(text)
            self.automate(now, last_activity)
        self.finish()

    def check(self, text):
        for pat in self.args.watch:
            if re.search(pat, text):
                first = pat not in self.seen
                self.seen[pat] = self.seen.get(pat, 0) + 1
                if first:
                    rel = "" if self.prompt_sent_at is None else f" (+{time.monotonic() - self.prompt_sent_at:.3f}s after prompt)"
                    self.out(f"WATCH first match {pat!r}{rel}")

    def automate(self, now, last_activity):
        recent = self.screen[-3000:]
        if not self.trusted and re.search(r"trust\s*this\s*folder|Yes,\s*proceed|Yes,\s*I\s*trust", recent, re.I):
            self.trusted = True
            time.sleep(0.5)
            if re.search(r"❯\s*No", recent):
                self.send("\x1b[B", "move to Yes in the trust dialog")
                time.sleep(0.3)
            self.send("\r", "accept trust dialog")
            self.screen = ""
            return
        if re.search(r"Press Enter to continue", recent):
            time.sleep(0.3)
            self.send("\r", "press enter to continue")
            self.screen = ""
            return
        if self.prompt_sent_at is None:
            ready = re.search(r"\? for shortcuts|Try \"|bypass permissions|>\s*$", recent)
            if (ready and now - last_activity > 1.0) or now - self.t0 > self.args.start_after:
                self.send(self.args.prompt, "prompt")
                time.sleep(0.3)
                self.send("\r", "enter")
                self.prompt_sent_at = time.monotonic()
                self.screen = ""
            return
        while self.pending_at and now - self.prompt_sent_at >= self.pending_at[0][0]:
            t, keys = self.pending_at.pop(0)
            self.send(keys, f"scheduled at +{t}s")

    def reaped(self):
        try:
            pid, _ = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            return True
        return pid != 0

    def drain_output(self, seconds):
        # Keep reading so the child never blocks on a full pty buffer while it exits.
        end = time.monotonic() + seconds
        while time.monotonic() < end and not self.reaped():
            r, _, _ = select.select([self.fd], [], [], 0.1)
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if data:
                    self.raw.write(data)
                    compact = re.sub(r"\s+", " ", strip(data)).strip()
                    self.out(f"DRAW {compact[:self.args.draw_chars]}")

    def finish(self):
        self.out(f"watch counts: {self.seen}")
        signal.signal(signal.SIGALRM, lambda *_: os._exit(3))
        signal.alarm(15)  # never hang around after the session is over
        for keys in (b"\x03", b"\x03", b"/exit\r"):
            if self.reaped():
                break
            try:
                os.write(self.fd, keys)
            except OSError:
                break
            self.drain_output(1.0)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            if self.reaped():
                break
            try:
                os.killpg(os.getpgid(self.pid), sig)
            except OSError:
                pass
            self.drain_output(1.0)
        self.out(f"done exited={self.reaped()}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cwd", required=True)
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--settings", default=None, help="passed to claude --settings")
    ap.add_argument("--config-dir", default=None, help="CLAUDE_CONFIG_DIR for the child")
    ap.add_argument("--model", default=None)
    ap.add_argument("--seconds", type=float, default=90.0)
    ap.add_argument("--start-after", type=float, default=15.0, help="type the prompt at the latest after this many s")
    ap.add_argument("--watch", action="append", default=[], help="regex to time on the screen (repeatable)")
    ap.add_argument("--at", action="append", default=[], help="T:KEYS to send KEYS T s after the prompt (repeatable)")
    ap.add_argument("--log", required=True)
    ap.add_argument("--cols", type=int, default=140)
    ap.add_argument("--rows", type=int, default=45)
    ap.add_argument("--draw-chars", type=int, default=400)
    ap.add_argument("--echo", action="store_true")
    ap.add_argument("claude_args", nargs="*", help="extra args for claude (after --)")
    args = ap.parse_args()
    Session(args).run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
