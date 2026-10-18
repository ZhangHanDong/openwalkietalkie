import contextlib
import importlib.util
import io
from pathlib import Path
import struct
import tempfile
import unittest
import zlib
from collections import deque

from test_release import elf, image

SPEC = importlib.util.spec_from_file_location("flash_usb", Path(__file__).resolve().parents[1] / "flash-usb.py")
flash = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(flash)


class FlashImageTests(unittest.TestCase):
    def test_bulk_begin_retries_old_fragment_and_drains_stale_ack(self):
        class Bulk:
            def __init__(self):
                self.pending, self.ops = deque(), []
                self.timeout, self.stale = True, True
            def send(self, frame):
                self.pending.append(frame)
                self.ops.append(frame[4])
            def recv(self):
                f = self.pending.popleft()
                if self.timeout:
                    self.timeout = False
                    raise TimeoutError("previous incomplete frame consumed BEGIN")
                token = f[16:20]
                if self.stale:
                    self.stale = False
                    self.pending.appendleft(f)
                    token = bytes([token[0] ^ 1]) + token[1:]
                r = b"OWTA" + bytes([f[4], 0, 1, 0]) + f[12:16] + token + f[8:12]
                return r + struct.pack("<I", zlib.crc32(r))
        bulk = Bulk()
        with contextlib.redirect_stderr(io.StringIO()):
            flash.download_bulk(bulk, b"\xa5" * 4200)
        self.assertEqual(bulk.ops, [flash.OP_BEGIN, flash.OP_BEGIN, flash.OP_COMMIT, flash.OP_COMMIT, flash.OP_FINISH])

    def test_bulk_framing_pipeline_sparse_padding_and_bond_gap(self):
        test = self
        class Bulk:
            def __init__(self):
                self.frames, self.pending, self.max_pending = [], deque(), 0
            def send(self, frame):
                test.assertEqual(len(frame) % 64, 32)
                test.assertEqual(frame[:4], b"OWTF")
                test.assertEqual(struct.unpack_from("<I", frame, 24)[0], zlib.crc32(frame[:24]))
                self.frames.append(frame)
                self.pending.append(frame)
                self.max_pending = max(self.max_pending, len(self.pending))
            def recv(self):
                f = self.pending.popleft()
                reply = b"OWTA" + bytes([f[4], 0, 1, 0]) + f[12:20] + f[8:12]
                return reply + struct.pack("<I", zlib.crc32(reply))
        image = bytearray(b"\xff" * (flash.BONDS_END - flash.APP_BASE + 32))
        image[:4096] = b"\xa5" * 4096
        image[-32:] = b"\x5a" * 32
        bulk = Bulk()
        with contextlib.redirect_stderr(io.StringIO()):
            flash.download_bulk(bulk, image)
        self.assertEqual(bulk.max_pending, 2)
        self.assertEqual([f[4] for f in bulk.frames].count(flash.OP_COMMIT), 2)
        for f in bulk.frames[1:-1]:
            address = struct.unpack_from("<I", f, 8)[0]
            self.assertFalse(flash.BONDS_START <= address < flash.BONDS_END)
        self.assertEqual(bulk.frames[-2][32:64], b"\x5a" * 32)
        self.assertEqual(len(bulk.frames[-2]), 96)  # short packet even for a 32-byte tail
        self.assertEqual(bulk.frames[-1][4], flash.OP_FINISH)

    def test_bulk_rejects_corrupt_stale_and_failed_acknowledgments(self):
        frame = flash.bulk_frame(flash.OP_COMMIT, flash.APP_BASE, 1, 123, b"\xa5" * 32)
        prefix = b"OWTA" + bytes([flash.OP_COMMIT, 0, 1, 0]) + frame[12:20] + frame[8:12]
        good = prefix + struct.pack("<I", zlib.crc32(prefix))
        flash.check_bulk_reply(good, frame)
        for at in (4, 5, 6, 8, 12, 16, 20):
            bad = bytearray(good)
            bad[at] ^= 1
            if at != 20:
                struct.pack_into("<I", bad, 20, zlib.crc32(bad[:20]))
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                flash.check_bulk_reply(bad, frame)

    def test_sparse_hid_is_capability_gated_and_bulk_failure_does_not_finish(self):
        test = self
        class Link:
            capabilities = 1
            def __init__(self): self.operations = []
            def hello(self): return 2, True
            def send(self, *args): test.fail("empty sector data must not be transmitted")
            def command(self, op, data, **kwargs):
                self.operations.append(op)
                if op == 8:
                    addr, length, crc = struct.unpack("<IHI", data)
                    test.assertEqual(crc, zlib.crc32(b"\xff" * length))
                    return b"\x00" + struct.pack("<I", addr)
                return b"\x00\x01" if op == flash.OP_FINISH else b"\x00"
        link = Link()
        with contextlib.redirect_stderr(io.StringIO()):
            flash.download(link, b"\xff" * 4200)
        self.assertEqual(link.operations, [flash.OP_BEGIN, 8, 8, flash.OP_FINISH])
        class BrokenBulk:
            def __init__(self): self.frames = []
            def send(self, f): self.frames.append(f)
            def recv(self): raise OSError("connection lost")
        broken = BrokenBulk()
        with self.assertRaises(OSError): flash.download_bulk(broken, b"\xa5" * 4200)
        self.assertNotIn(flash.OP_FINISH, [f[4] for f in broken.frames])

    def test_download_announces_size_and_keeps_legacy_protocol_compatible(self):
        payload = bytes([0xa5]) * 4200
        for protocol in (1, 2):
            class Link:
                offset = 0
                staged = bytearray()
                finished = False

                def hello(self):
                    return protocol, True

                def send(self, channel, data):
                    self_test.assertEqual(channel, flash.RECOVERY_CHANNEL)
                    op, offset, count = struct.unpack_from("<BHB", data)
                    self_test.assertEqual(op, flash.OP_DATA)
                    self_test.assertEqual(offset, len(self.staged))
                    self_test.assertEqual(count, len(data) - 4)
                    self.staged.extend(data[4:])

                def command(self, op, data, **kwargs):
                    if op == flash.OP_BEGIN:
                        self_test.assertEqual(data, struct.pack("<I", len(payload)))
                        return bytes([0])  # Legacy bootloaders ignore the size.
                    if op == flash.OP_COMMIT:
                        addr, count, crc = struct.unpack("<IHI", data)
                        self_test.assertEqual(addr, flash.APP_BASE + self.offset)
                        self_test.assertEqual(count, len(self.staged))
                        self_test.assertEqual(crc, zlib.crc32(self.staged))
                        self_test.assertEqual(self.staged, payload[self.offset:self.offset + count])
                        self.offset += count
                        self.staged.clear()
                        return bytes([0]) + struct.pack("<I", addr)
                    self_test.assertEqual(op, flash.OP_FINISH)
                    self_test.assertEqual(self.offset, len(payload))
                    self.finished = True
                    return bytes([0, 1])

            self_test = self
            link = Link()
            with contextlib.redirect_stderr(io.StringIO()):
                flash.download(link, payload)
            self.assertTrue(link.finished)

    def load(self, data, **kwargs):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "firmware"
            path.write_bytes(data)
            return flash.load_image(path, **kwargs)

    def test_current_xip_and_legacy_psram_images_preserve_bytes_and_board(self):
        for board in ("A", "B"):
            payload = image(board)
            for data in (payload, elf(payload), elf(payload, flash.APP_BASE)):
                self.assertEqual(self.load(data), payload)
                self.assertEqual(flash.image_board(self.load(data)), f"Rev {board}")

    def test_legacy_xip_flag_cannot_bypass_board_identity(self):
        for data in (image().replace(b"Rev A", b"Other"), image() + image("B")):
            for payload in (data, elf(data, flash.APP_BASE)):
                with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                    self.load(payload, xip=True)

    def test_bad_vectors_truncation_and_non_executable_images_are_refused(self):
        bad_vector = bytearray(image())
        struct.pack_into("<I", bad_vector, 4, flash.APP_BASE + 100001)
        non_executable = elf(image(), flash.APP_BASE)
        struct.pack_into("<I", non_executable, 76, 4)
        bad_header = elf(image(), flash.APP_BASE)
        struct.pack_into("<H", bad_header, 42, 16)
        for data in (b"", b"\x7fELF", elf(image())[:-2], bad_vector, non_executable, bad_header):
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                self.load(data)

    def test_bootloader_and_overlapping_segments_are_refused(self):
        data = elf(image(), flash.APP_BASE)
        struct.pack_into("<I", data, 64, 0x12010000)
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            self.load(data)
        # Two load records targeting the same flash bytes.
        data = bytearray(116) + image()
        data[:52] = elf(image())[:52]
        struct.pack_into("<H", data, 44, 2)
        for at in (52, 84):
            struct.pack_into("<8I", data, at, 1, 116, flash.APP_BASE, flash.APP_BASE, len(image()), len(image()), 5, 4)
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            self.load(data)
