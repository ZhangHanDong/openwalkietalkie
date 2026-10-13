#!/usr/bin/env python3
"""Flash an SF32LB52x board over its UART debug port when nothing can pulse its reset.

The Rev A board exposes UART1 on test pads (TP6 board RX, TP8 board TX, TP4
GND) and no reset line, so `tools/flash.sh` does not apply. The chip's UART
debug port answers at any time, blank or with firmware running: this script
clears the debug session the port may still be in (an earlier aborted attempt
leaves it answering Enter but ignoring memory commands, which sftool reports as
"Failed to download stub: Timeout"), checks a memory read works, then execs
sftool on the same port with `--before no_reset`. sftool soft-resets the board
into the new image when it is done.

Usage:
    tools/flash-uart.py PORT [sftool arguments after the connection options]

    tools/flash-uart.py /dev/cu.usbmodem3101 write_flash --verify \
        target/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
    tools/flash-uart.py /dev/cu.usbmodem3101 --compat false read_flash /tmp/ftab.bin@0x12000000:4096

`--compat true` is on by default: USB CDC bridges such as a DAPLink's UART side
drop sftool's 64 KiB stub frame, and the 256-byte packets it selects instead
are harmless on any adapter. Pass `--compat false` to turn it off.
"""
import array
import fcntl
import os
import select
import struct
import sys
import termios
import time

IOSSIOSPEED = 0x80045402  # macOS: set a non-standard rate after open
BAUD = 1000000  # the debug port's fixed rate


def frame(payload):
    return bytes([0x7E, 0x79]) + struct.pack("<H", len(payload)) + bytes([0x10, 0x00]) + payload


ENTER = frame(bytes([0x41, 0x54, 0x53, 0x46, 0x33, 0x32, 0x05, 0x21]))
EXIT = frame(bytes([0x41, 0x54, 0x53, 0x46, 0x33, 0x32, 0x18, 0x21]))
MEMREAD = frame(bytes([0x40, 0x72]) + struct.pack("<II", 0x20000000, 1))


def open_port(port):
    fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    attrs = termios.tcgetattr(fd)
    attrs[0] = 0
    attrs[1] = 0
    attrs[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
    attrs[3] = 0
    attrs[6][termios.VMIN] = 0
    attrs[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    fcntl.ioctl(fd, IOSSIOSPEED, array.array("i", [BAUD]))
    return fd


def exchange(fd, data, wait):
    termios.tcflush(fd, termios.TCIFLUSH)
    os.write(fd, data)
    buf = b""
    end = time.monotonic() + wait
    while time.monotonic() < end:
        ready, _, _ = select.select([fd], [], [], max(0, end - time.monotonic()))
        if ready:
            try:
                buf += os.read(fd, 512)
            except BlockingIOError:
                pass
    return buf


def acked(buf, code):
    start = buf.find(b"\x7e\x79")
    return start >= 0 and code in buf[start:]


def main():
    if len(sys.argv) < 2 or sys.argv[1].startswith("-"):
        sys.exit(__doc__.strip())
    port, rest = sys.argv[1], sys.argv[2:]
    compat = "true"
    if "--compat" in rest:
        at = rest.index("--compat")
        compat = rest[at + 1]
        del rest[at : at + 2]
    fd = open_port(port)
    for attempt in range(1, 9):
        exchange(fd, EXIT, 0.2)
        entered = exchange(fd, ENTER, 0.3)
        read = exchange(fd, MEMREAD, 0.5)
        if acked(entered, b"\xd1") and acked(read, b"\xd2"):
            print(f"flash-uart: debug port ready (attempt {attempt})", flush=True)
            break
        print(
            f"flash-uart: attempt {attempt}: Enter {entered.hex(' ') or '-'} / "
            f"memory read {read.hex(' ') or '-'}",
            flush=True,
        )
    else:
        sys.exit(
            "flash-uart: the debug port did not answer; is the board powered and the "
            "adapter wired TX->TP6, RX->TP8, GND->TP4 at 3.3 V?"
        )
    os.close(fd)
    args = ["sftool", "-c", "SF32LB52", "-p", port, "--before", "no_reset",
            "--connect-attempts", "5", "--compat", compat] + rest
    os.execvp("sftool", args)


if __name__ == "__main__":
    main()
