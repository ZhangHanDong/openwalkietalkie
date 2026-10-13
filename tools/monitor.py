#!/usr/bin/env python3
"""Tail the board's debug UART.

Two things make this awkward enough to need a script rather than `stty` + `cat`:

  * macOS termios has no constant for 1000000 baud, so `stty` refuses it with
    "tcsetattr: Invalid argument". The rate has to be set with the IOSSIOSPEED
    ioctl after the port is open.
  * Resetting the board means pulsing RTS, which is what sftool does to drop it
    into the bootloader (assert 100ms, release 100ms). On this board a bare RTS
    pulse only reboots it sometimes -- sftool's full connect-then-reset is the
    reliable way, so `--sftool-reset` shells out to it and attaches after.

Usage:
    tools/monitor.py [PORT] [--reset | --sftool-reset] [--seconds N] [--interactive]

Opening this reset-wired bridge can itself pulse reset. For fault diagnosis,
keep one connection open and use --interactive to send shell commands through
that same connection rather than reopening the port.
"""

import argparse
import array
import fcntl
import glob
import os
import select
import subprocess
import sys
import termios
import time

IOSSIOSPEED = 0x80045402
TIOCMSET = 0x8004746D
TIOCM_DTR = 0x002
TIOCM_RTS = 0x004

BAUD = 1000000


def find_port():
    for pattern in ("/dev/cu.*wchusbserial*", "/dev/cu.*usbserial*", "/dev/cu.*usbmodem*"):
        matches = sorted(glob.glob(pattern))
        if matches:
            return matches[0]
    sys.exit("no USB serial device found")


def configure(fd):
    # Release reset before termios work. Some bridges assert it on open, so
    # waiting until after configuration can produce a valid reset pulse.
    # This shortens the pulse; it cannot guarantee a non-resetting attach.
    set_lines(fd, 0)
    attrs = termios.tcgetattr(fd)
    iflag, oflag, cflag, lflag, _ispeed, _ospeed, cc = attrs
    # Force a standard rate into the termios struct. Once IOSSIOSPEED has put
    # the port at 1000000, tcgetattr reports that literal value back, and
    # handing it to tcsetattr fails with EINVAL -- so re-attaching to a port
    # this script had already configured would blow up. The real rate is set by
    # the ioctl below; this value only has to be legal.
    ispeed = ospeed = termios.B9600
    # 8N1, no flow control, receiver on, ignore modem lines.
    cflag = termios.CS8 | termios.CREAD | termios.CLOCAL
    iflag = 0
    oflag = 0
    lflag = 0
    cc = list(cc)
    cc[termios.VMIN] = 0
    cc[termios.VTIME] = 1
    termios.tcsetattr(fd, termios.TCSANOW, [iflag, oflag, cflag, lflag, ispeed, ospeed, cc])
    # Must come after tcsetattr: tcsetattr would clobber the custom rate.
    fcntl.ioctl(fd, IOSSIOSPEED, array.array("i", [BAUD]), 1)
    # macOS asserts both DTR and RTS on open, and RTS is wired to the chip's
    # reset. Leaving it asserted holds the board in reset and the port stays
    # silent forever -- which looks exactly like firmware that never booted.
    set_lines(fd, 0)


def set_lines(fd, value):
    fcntl.ioctl(fd, TIOCMSET, array.array("i", [value]))


def sftool_reset(port):
    """Reboot via sftool, which resets and then releases the chip into the app.

    More reliable than an RTS pulse here, at the cost of a second or two. The
    trivial read is just a vehicle for sftool's reset-before/reset-after.
    """
    subprocess.run(
        ["sftool", "-c", "SF32LB52", "--port", port, "--connect-attempts", "10",
         "-q", "read_flash", "/dev/null@0x12000000:16"],
        capture_output=True,
    )


def reset(fd):
    """Pulse RTS, the way sftool reboots the board."""
    set_lines(fd, TIOCM_RTS)
    time.sleep(0.1)
    set_lines(fd, 0)
    time.sleep(0.1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("port", nargs="?")
    ap.add_argument("--reset", action="store_true", help="pulse RTS to reboot the board first")
    ap.add_argument("--sftool-reset", action="store_true",
                    help="reboot via sftool first (slower, but reliable on this board)")
    ap.add_argument("--seconds", type=float, help="stop after N seconds (default: run until ctrl-c)")
    ap.add_argument("--interactive", action="store_true",
                    help="forward stdin shell commands without reopening the serial port")
    args = ap.parse_args()

    port = args.port or find_port()
    if args.sftool_reset:
        sftool_reset(port)
    fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    try:
        configure(fd)
        os.set_blocking(fd, True)
        if args.reset:
            reset(fd)
            termios.tcflush(fd, termios.TCIFLUSH)
        print(f"--- {port} @ {BAUD} ---", file=sys.stderr, flush=True)

        deadline = time.time() + args.seconds if args.seconds else None
        stdin = sys.stdin.fileno() if args.interactive else None
        while deadline is None or time.time() < deadline:
            readable, _, _ = select.select([fd] + ([stdin] if stdin is not None else []), [], [], 0.2)
            if stdin is not None and stdin in readable:
                command = os.read(stdin, 4096)
                if command:
                    os.write(fd, command.replace(b"\n", b"\r"))
                else:
                    stdin = None
            if fd not in readable:
                continue
            try:
                chunk = os.read(fd, 4096)
            except OSError:
                break
            if chunk:
                sys.stdout.write(chunk.decode("utf-8", "replace"))
                sys.stdout.flush()
    except KeyboardInterrupt:
        pass
    finally:
        os.close(fd)


if __name__ == "__main__":
    main()
