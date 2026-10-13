import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parents[1] / "release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def image(board="A"):
    return struct.pack("<II", 0x20040000, release.APP_BASE + 9) + b"\0" * 12 + f"OpenWalkieTalkie Rev {board} (test)".encode()


def elf(payload):
    data = bytearray(84)
    data[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<I", data, 28, 52)
    struct.pack_into("<HH", data, 42, 32, 1)
    struct.pack_into("<8I", data, 52, 1, 84, 0x60000000, release.APP_BASE, len(payload), len(payload), 5, 4)
    return data + payload


class ReleaseTests(unittest.TestCase):
    def test_real_application_extraction_preserves_bytes(self):
        self.assertEqual(release.image_from_elf(elf(image()), "hw-a"), image())

    def test_other_board_is_refused(self):
        with self.assertRaisesRegex(ValueError, "board"):
            release.image_from_elf(elf(image()), "hw-b")

    def test_truncated_elf_and_bad_vector_are_refused(self):
        with self.assertRaises(ValueError):
            release.image_from_elf(elf(image())[:-5], "hw-a")
        payload = bytearray(image())
        struct.pack_into("<I", payload, 4, release.APP_BASE + 100001)
        with self.assertRaisesRegex(ValueError, "vector"):
            release.image_from_elf(elf(payload), "hw-a")

    def test_bootloader_and_xip_segments_cannot_be_released(self):
        for offset, value in [(64, 0x12010000), (60, release.APP_BASE)]:
            data = elf(image())
            struct.pack_into("<I", data, offset, value)
            with self.assertRaises(ValueError):
                release.image_from_elf(data, "hw-a")

    def test_manifest_binds_both_boards_to_release_assets(self):
        app, firmware = release.versions()
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            for board in ("a", "b"):
                (directory / f"OpenWalkieTalkie-{firmware}-hw-{board}.bin").write_bytes(image(board.upper()))
            for arch in ("aarch64", "x86_64"):
                (directory / f"OpenWalkieTalkie-{app}-macos-{arch}.dmg").write_bytes(b"fixture")
            manifest = release.manifest(directory, f"v{app}")
            for board, asset in manifest["firmware"]["variants"].items():
                self.assertIn(board, asset["name"])
                self.assertEqual(len(asset["sha256"]), 64)
                self.assertTrue(asset["url"].startswith(f"https://github.com/conol-ai/openwalkietalkie/releases/download/v{app}/"))
            with self.assertRaises(ValueError):
                release.manifest(directory, "v999.0.0")
