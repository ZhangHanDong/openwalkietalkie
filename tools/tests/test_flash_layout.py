import contextlib
import io
import struct
import unittest
import zlib

from test_flash_usb import flash
from test_release import image, release


def large_image():
    data = bytearray(b"\xff" * (flash.BONDS_END - flash.APP_BASE + 17))
    data[:len(image())] = image()
    data[-17:] = bytes(range(17))
    return data


class Recovery:
    def __init__(self, protocol=2):
        self.protocol = protocol
        self.calls = []
        self.sector = bytearray(4096)
        self.written = []

    def hello(self):
        return self.protocol, True

    def send(self, channel, payload):
        self.assert_channel = channel
        op, low, high, size = payload[:4]
        assert op == flash.OP_DATA
        offset = low | high << 8
        self.sector[offset:offset + size] = payload[4:]

    def command(self, op, payload, timeout_s=5):
        self.calls.append(op)
        if op == flash.OP_COMMIT:
            address, length, crc = struct.unpack("<IHI", payload)
            assert zlib.crc32(self.sector[:length]) & 0xffffffff == crc
            assert not flash.BONDS_START <= address < flash.BONDS_END
            self.written.append((address, bytes(self.sector[:length])))
            return bytes([0]) + struct.pack("<I", address)
        return bytes([0, 1])


class FlashLayoutTests(unittest.TestCase):
    def test_extended_download_preserves_both_bond_stores(self):
        data = large_image()
        link = Recovery()
        with contextlib.redirect_stdout(io.StringIO()):
            flash.download(link, data)
        self.assertEqual(link.calls[0], flash.OP_BEGIN)
        self.assertEqual(link.calls[-1], flash.OP_FINISH)
        self.assertEqual(link.written[-1], (flash.BONDS_END, bytes(range(17))))
        for address, chunk in link.written:
            offset = address - flash.APP_BASE
            self.assertEqual(data[offset:offset + len(chunk)], chunk)
        self.assertEqual(sum(len(chunk) for _, chunk in link.written),
                         len(data) - (flash.BONDS_END - flash.BONDS_START))

    def test_old_bootloader_and_bad_padding_fail_before_any_erase(self):
        data = large_image()
        for protocol, corrupt in ((1, False), (3, False), (2, True)):
            payload = bytearray(data)
            if corrupt:
                payload[flash.BONDS_START - flash.APP_BASE] = 0
            link = Recovery(protocol)
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                flash.download(link, payload)
            self.assertEqual(link.calls, [])

    def test_release_span_padding_and_reset_vector_bounds(self):
        data = large_image()
        release.validate_image(data, "hw-a")
        data[flash.BONDS_START - flash.APP_BASE] = 0
        with self.assertRaisesRegex(ValueError, "storage"):
            release.validate_image(data, "hw-a")
        data[flash.BONDS_START - flash.APP_BASE] = 255
        struct.pack_into("<I", data, 4, flash.BONDS_START + 1)
        with self.assertRaisesRegex(ValueError, "storage"):
            release.validate_image(data, "hw-a")
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            flash.validate_padding(b"\xff" * (flash.APP_END - flash.APP_BASE + 1))

    def test_elf_high_bank_is_loaded_but_bond_segment_is_rejected(self):
        for address, virtual in (
            (address, virtual)
            for address in (flash.BONDS_END, flash.BONDS_START, flash.APP_END)
            for virtual in (address, 0x60150000)
        ):
            low = image()
            data = bytearray(116) + low + b"asset"
            data[:7] = b"\x7fELF\x01\x01\x01"
            struct.pack_into("<I", data, 28, 52)
            struct.pack_into("<HH", data, 42, 32, 2)
            struct.pack_into("<8I", data, 52, 1, 116, flash.APP_BASE, flash.APP_BASE, len(low), len(low), 5, 4)
            struct.pack_into("<8I", data, 84, 1, 116 + len(low), virtual, address, 5, 5, 4, 4)
            if address == flash.BONDS_END:
                expected = release.image_from_elf(data, "hw-a")
                self.assertEqual(flash.image_from_elf(data), expected)
                self.assertEqual(expected[-5:], b"asset")
            else:
                with self.assertRaises(ValueError):
                    release.image_from_elf(data, "hw-a")
                with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                    flash.image_from_elf(data)

    def test_protocol_one_still_accepts_small_legacy_images(self):
        flash.check_recovery(image(), (1, True))
        self.assertEqual(flash.APP_END - flash.APP_BASE, release.APP_SIZE)
        self.assertEqual((flash.BONDS_START, flash.BONDS_END),
                         (release.BONDS_START, release.BONDS_END))
