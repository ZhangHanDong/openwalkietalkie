#!/usr/bin/env python3
"""Flash the Rev A or Rev B application over the deck's own USB-C port, through
the bootloader's recovery mode (native/bootloader). No probe, no test pads,
no driver.

    tools/flash-usb.py target/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
    tools/flash-usb.py firmware.bin            # a raw image for 0x12020000
    tools/flash-usb.py --status                # what is attached, and which mode

Recovery mode is the deck's own USB device ("OpenWalkieTalkie", 1209:5754, the
vendor HID collection the application presents, descriptor for descriptor),
so a host that has accepted the application accepts it too. The image goes
over channel 9 of the same 64-byte reports: one 4 KB sector at a time with a
CRC, written to the application window only; the bootloader writes the
vector-table sector last, checks it, and reboots into the application, whose
version this script then reports.

Flow: a running deck is asked over channel 2 to reboot into recovery
(`sys.reboot`, mode recovery). A deck that cannot run its application
(erased or half-written image) boots into recovery by itself; so does one
powered on with SHIFT and CANCEL (ESC) held. Needs the `hid` Python module.

The application names its board (`sys.version` answers `board`, and the ELF
carries the same name), and the three boards' images share one build path:
an ELF built for another board than the running deck reports is refused
unless --any-board is passed. A deck already in recovery cannot say which
board it is.
"""
import argparse
import base64
import json
import struct
import sys
import time
import zlib

VID, PID, USAGE_PAGE = 0x1209, 0x5754, 0xFF00
APP_BASE = 0x12020000
APP_END = APP_BASE + 7 * 1024 * 1024 - 64 * 1024  # the pairing journal follows
SECTOR = 4096
REPORT_ID, RPC_CHANNEL, RECOVERY_CHANNEL, PAYLOAD_MAX = 6, 2, 9, 61
OP_HELLO, OP_BEGIN, OP_DATA, OP_COMMIT, OP_FINISH, OP_BOOT, OP_READ = 1, 2, 3, 4, 5, 6, 7
STATUS = {0: "ok", 1: "bad arguments", 2: "crc mismatch", 3: "flash error", 4: "verify error"}


def log(message):
    print(f"flash-usb: {message}", flush=True)


def fail(message):
    print(f"flash-usb: {message}", file=sys.stderr)
    sys.exit(1)


# ---- image -------------------------------------------------------------------

def image_from_elf(data, xip=False):
    """The flash contents an ELF describes: its PT_LOAD segments by physical
    address inside the application window, gaps filled with 0xFF (what sftool
    does when it flashes an ELF).

    A Rev A image runs from PSRAM (link-psram.x): its code is a segment whose
    virtual address is in PSRAM with its load image in the NOR. Both boards
    build to the same ELF path, so an image without such a segment is the dev
    board's (flash XIP, no USB task, its own pin map): on this deck it is a
    dark screen and no USB until the recovery chord. Refused unless `xip`."""
    if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
        fail("not a 32-bit little-endian ELF")
    e_phoff = struct.unpack_from("<I", data, 0x1C)[0]
    e_phentsize, e_phnum = struct.unpack_from("<HH", data, 0x2A)
    segments = []
    psram_code = False
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_paddr, p_filesz, _memsz, _flags, _align = struct.unpack_from("<IIIIIIII", data, off)
        if p_type != 1 or p_filesz == 0:
            continue
        if 0x60000000 <= p_vaddr < 0x70000000 and APP_BASE <= p_paddr < APP_END:
            psram_code = True
        if p_paddr < APP_BASE or p_paddr + p_filesz > APP_END:
            if 0x20000000 <= p_paddr < 0x70000000:
                continue  # RAM/PSRAM-only segment
            fail(f"segment at 0x{p_paddr:08X} ({p_filesz} bytes) is outside the application window")
        segments.append((p_paddr, data[p_offset:p_offset + p_filesz]))
    if not segments:
        fail("no flash segments in the ELF")
    if not psram_code and not xip:
        fail("this ELF runs from the NOR (no code segment loaded into PSRAM): a dev-board build, not a Rev A or "
             "Rev B image (`cargo build --release --features hw-a|hw-b`, built last); pass --xip if that is really wanted")
    segments.sort()
    base = segments[0][0]
    if base != APP_BASE:
        fail(f"the image starts at 0x{base:08X}, expected the vector table at 0x{APP_BASE:08X}")
    end = max(addr + len(body) for addr, body in segments)
    image = bytearray(b"\xff" * (end - base))
    for addr, body in segments:
        image[addr - base:addr - base + len(body)] = body
    return bytes(image)


def image_board(data):
    """The board an application image names in its boot banner (src/board.rs
    `NAME`), or None for a raw image that carries none."""
    boards = [name for name in ("Rev A", "Rev B") if b"OpenWalkieTalkie " + name.encode() + b" (" in data]
    return boards[0] if len(boards) == 1 else None


def load_image(path, xip=False):
    data = open(path, "rb").read()
    image = image_from_elf(data, xip) if data[:4] == b"\x7fELF" else data
    sp, pc = struct.unpack_from("<II", image, 0)
    if not (0x20000000 <= sp <= 0x20080000) or not (pc & 1) or not (APP_BASE <= pc < APP_END):
        fail(f"the image's vector table looks wrong (SP 0x{sp:08X}, reset 0x{pc:08X})")
    if len(image) > APP_END - APP_BASE:
        fail("image larger than the application window")
    return image


# ---- the deck's USB device --------------------------------------------------------

def hid_module():
    try:
        import hid  # noqa: PLC0415
    except ImportError:
        fail("the 'hid' Python module is missing for this interpreter "
             f"({sys.executable}); with the SiFli SDK environment sourced, run /usr/bin/python3 tools/flash-usb.py")
    return hid


def device_paths(serial=None):
    """The deck's vendor collections on USB, optionally one deck by serial.

    A deck paired over Bluetooth shows up in the same enumeration (bus type 2,
    no serial, interface -1) and must never be used: recovery is USB-only, so
    a reboot request sent over that road would strand a deck that is not on
    the cable in recovery mode.
    """
    hid = hid_module()
    paths = []
    for d in hid.enumerate(VID, PID):
        if d.get("usage_page") != USAGE_PAGE:
            continue
        if d.get("bus_type", 1) != 1 or d.get("interface_number", 0) < 0:
            continue
        if serial is not None and d.get("serial_number") != serial:
            continue
        paths.append(d["path"])
    return paths


class Link:
    """One open vendor HID collection: reports of [id 6][channel][len][payload]."""

    def __init__(self, path):
        hid = hid_module()
        self.dev = hid.device()
        self.dev.open_path(path)
        self.dev.set_nonblocking(0)

    def close(self):
        self.dev.close()

    def send(self, channel, payload):
        if len(payload) > PAYLOAD_MAX:
            raise ValueError("payload too long")
        report = bytes([REPORT_ID, channel, len(payload)]) + bytes(payload)
        self.dev.write(report + b"\0" * (64 - len(report)))

    def recv(self, channel, timeout_s):
        """The next payload on `channel`, or None; other channels are skipped."""
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            data = bytes(self.dev.read(64, timeout_ms=100))
            if not data:
                continue
            body = data[1:] if data[0] == REPORT_ID else data
            if len(body) < 2 or body[0] != channel:
                continue
            n = body[1]
            return body[2:2 + n]
        return None

    def rpc(self, method, params, timeout_s=3.0):
        request = json.dumps({"method": method, "params": params, "id": 1}, separators=(",", ":")).encode() + b"\n"
        for off in range(0, len(request), PAYLOAD_MAX):
            self.send(RPC_CHANNEL, request[off:off + PAYLOAD_MAX])
        line = b""
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            chunk = self.recv(RPC_CHANNEL, deadline - time.monotonic())
            if chunk is None:
                break
            line += chunk
            if b"\n" in line:
                try:
                    return json.loads(line.split(b"\n", 1)[0])
                except json.JSONDecodeError:
                    return {"raw": line.decode(errors="replace")}
        return None

    def hello(self, timeout_s=1.0):
        """(protocol, image valid) from a deck in recovery, else None."""
        self.send(RECOVERY_CHANNEL, [OP_HELLO])
        reply = self.recv(RECOVERY_CHANNEL, timeout_s)
        if reply and reply[:5] == bytes([OP_HELLO]) + b"OWTB" and len(reply) >= 7:
            return reply[5], bool(reply[6])
        return None

    def command(self, op, payload, timeout_s=5.0):
        self.send(RECOVERY_CHANNEL, [op] + list(payload))
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            reply = self.recv(RECOVERY_CHANNEL, deadline - time.monotonic())
            if reply is None:
                break
            if reply[0] == op:
                return reply[1:]
        fail(f"no reply to recovery command {op}")


def wait_for(predicate, what, timeout_s):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.3)
    fail(f"timed out waiting for {what}")


def open_recovery(known_paths, timeout_s):
    """A link to the deck in recovery mode, once it has re-enumerated."""
    def fresh():
        for path in device_paths():
            if path in known_paths:
                continue
            try:
                link = Link(path)
            except OSError:
                return None
            if link.hello():
                return link
            link.close()
        return None
    return wait_for(fresh, "the deck's recovery mode", timeout_s)


def download(link, image):
    status = link.command(OP_BEGIN, [])
    if status[0] != 0:
        fail(f"begin failed: {STATUS.get(status[0], status[0])}")
    sectors = range(0, len(image), SECTOR)
    started = time.monotonic()
    for index, offset in enumerate(sectors):
        chunk = image[offset:offset + SECTOR]
        for off in range(0, len(chunk), PAYLOAD_MAX - 4):
            piece = chunk[off:off + PAYLOAD_MAX - 4]
            link.send(RECOVERY_CHANNEL, bytes([OP_DATA, off & 0xFF, off >> 8, len(piece)]) + piece)
        addr = APP_BASE + offset
        reply = link.command(OP_COMMIT, struct.pack("<IHI", addr, len(chunk), zlib.crc32(chunk) & 0xFFFFFFFF), timeout_s=10)
        if reply[0] != 0 or struct.unpack_from("<I", reply, 1)[0] != addr:
            fail(f"sector 0x{addr:08X}: {STATUS.get(reply[0], reply[0])}")
        if index % 32 == 0 or offset + SECTOR >= len(image):
            done = min(offset + SECTOR, len(image))
            log(f"{done * 100 // len(image):3d}% ({done}/{len(image)} bytes)")
    reply = link.command(OP_FINISH, [], timeout_s=10)
    elapsed = time.monotonic() - started
    if reply[0] != 0 or not reply[1]:
        fail(f"finish: {STATUS.get(reply[0], reply[0])}, image valid: {bool(reply[1])}")
    log(f"download done in {elapsed:.1f} s ({len(image) / 1024 / max(elapsed, 0.001):.0f} KB/s); the bootloader checked the vector table and reboots")


def update_bootloader(link, path):
    """Bench only: hand a bootloader image to an application built with the
    `bootloader-update` feature, which writes it to its slot at 0x12010000
    (src/link/boot_update.rs). Not in --help on purpose: a power loss while
    the slot is rewritten leaves only the UART pads to put one back."""
    image = open(path, "rb").read()
    if not 8 <= len(image) <= 0x10000:
        fail(f"{path}: {len(image)} bytes; a bootloader is 8 B .. 64 KiB")
    sp, pc = struct.unpack_from("<II", image)
    if not (0x20000000 <= sp <= 0x20080000 and pc & 1 and 0x20000000 <= pc <= 0x20080000):
        fail(f"{path}: vector table SP 0x{sp:08X} PC 0x{pc:08X} does not point into SRAM; not a bootloader")
    crc = zlib.crc32(image)
    reply = link.rpc("boot.begin", {"n": len(image), "crc": crc})
    if not reply or "result" not in reply:
        fail(f"boot.begin refused: {reply} (is the application built with --features hw-a,bootloader-update?)")
    started = time.monotonic()
    for off in range(0, len(image), 128):
        reply = link.rpc("boot.data", {"o": off, "d": base64.b64encode(image[off:off + 128]).decode()})
        if not reply or reply.get("result", {}).get("o") != min(off + 128, len(image)):
            fail(f"boot.data at {off} failed: {reply}")
    log(f"bootloader: {len(image)} bytes staged in RAM in {time.monotonic() - started:.1f} s; writing the slot")
    reply = link.rpc("boot.commit", {}, timeout_s=30)
    result = (reply or {}).get("result")
    if not result:
        fail(f"boot.commit failed: {reply}")
    if result["crc"] != crc:
        fail(f"the slot reads back CRC 0x{result['crc']:08X}, not 0x{crc:08X}: rewrite it over UART before rebooting")
    if result["written"] == 0:
        log("bootloader: the slot already holds this image; nothing written")
    else:
        log(f"bootloader: {result['written']} bytes written and read back (CRC 0x{crc:08X})")
    return result["written"]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("image", nargs="?", help="application ELF (load segments) or raw .bin for 0x12020000")
    parser.add_argument("--status", action="store_true", help="say what is attached and in which mode, then exit")
    parser.add_argument("--no-request", action="store_true", help="the deck is already in recovery (buttons, or no valid image)")
    parser.add_argument("--no-wait-app", action="store_true", help="do not wait for the application to come back")
    parser.add_argument("--xip", action="store_true", help="flash an ELF that runs from the NOR (not the Rev A/B PSRAM link)")
    parser.add_argument("--any-board", action="store_true",
                        help="flash although the image names another board than the running application does")
    parser.add_argument("--bootloader", metavar="BIN", help=argparse.SUPPRESS)
    parser.add_argument("--serial", help="the deck to use when more than one is on USB (the USB serial number)")
    args = parser.parse_args()

    if args.bootloader:
        paths = device_paths(args.serial)
        if not paths:
            fail("the deck is not attached")
        link = Link(paths[0])
        if link.hello():
            link.close()
            fail("the deck is in recovery mode; the bootloader is replaced from the running application")
        written = update_bootloader(link, args.bootloader)
        if written:
            log(f"rebooting so the new bootloader runs: {link.rpc('sys.reboot', {})}")
        link.close()
        return

    paths = device_paths(args.serial)
    if args.status:
        if not paths:
            print("deck: not attached" + (f" (serial {args.serial})" if args.serial else ""))
            return
        link = Link(paths[0])
        hello = link.hello()
        if hello:
            print(f"deck: recovery mode (protocol {hello[0]}, application image {'valid' if hello[1] else 'INVALID'})")
        else:
            print("deck: application,", link.rpc("sys.version", {}))
        link.close()
        return
    if not args.image:
        parser.error("an image is required (or --status)")
    image = load_image(args.image, args.xip)
    board = image_board(image)
    log(f"image: {len(image)} bytes for 0x{APP_BASE:08X}" + (f", a {board} build" if board else ""))

    if not paths:
        fail("the deck is not attached")
    link = Link(paths[0])
    if not link.hello():
        if args.no_request:
            link.close()
            fail("the deck is running its application, not recovery mode")
        # The running application knows its board; an image for the other one
        # boots, with its speaker silent and its charger unread.
        version = link.rpc("sys.version", {}) or {}
        deck_board = (version.get("result") or {}).get("board") if isinstance(version, dict) else None
        if board and deck_board and deck_board != board:
            if not args.any_board:
                link.close()
                fail(f"the deck is a {deck_board} board and this image is a {board} build; "
                     "build with the matching feature, or pass --any-board")
            log(f"WARNING: writing a {board} build to a {deck_board} board (--any-board)")
        elif board and not deck_board:
            log("the running application does not name its board (older firmware); not checked")
        reply = link.rpc("sys.reboot", {"mode": "recovery"})
        link.close()
        log(f"asked the application to reboot into recovery: {reply}")
        link = open_recovery(set(paths), 30)
    log("deck is in recovery mode; downloading")
    download(link, image)
    seen = set(device_paths())
    link.close()
    if args.no_wait_app:
        return
    log("waiting for the application to boot")

    def application():
        for path in device_paths():
            if path in seen:
                continue
            try:
                probe = Link(path)
            except OSError:
                return None
            version = probe.rpc("sys.version", {}, timeout_s=1.5)
            probe.close()
            if version:
                return version
        return None
    version = wait_for(application, "the application's HID device", 40)
    log(f"application is back: {version}")


if __name__ == "__main__":
    main()
