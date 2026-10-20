import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

from test_release import image, recovery, bootstrap, elf

SPEC = importlib.util.spec_from_file_location("factory_image", Path(__file__).resolve().parents[1] / "factory-image.py")
factory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(factory)
VERIFY_SPEC = importlib.util.spec_from_file_location(
    "factory_verify", Path(__file__).resolve().parents[1] / "verify-factory-package.py")
verifier = importlib.util.module_from_spec(VERIFY_SPEC)
VERIFY_SPEC.loader.exec_module(verifier)


def ftab():
    data = bytearray(0x2C10)
    struct.pack_into("<I", data, 0, 0x53454346)
    for index, row in {0: (0x12000000, 0x8000, 0, 0),
                       3: (0x12010000, 0x20000, 0x20020000, 0),
                       4: (0x12020000, 0x6E0000, 0x12020000, 0),
                       7: (0x12010000, 0x20000, 0x20020000, 0),
                       8: (0x12020000, 0x6E0000, 0x12020000, 0)}.items():
        struct.pack_into("<4I", data, 4 + index * 16, *row)
    for index in range(14):
        row = {1: (0x10000, 512, 2), 2: (0x6E0000, 512, 2), 6: (0x6E0000, 512, 2)}.get(index, (0xFFFFFFFF, 0, 0))
        struct.pack_into("<IHH", data, 0x1000 + index * 512, *row)
    struct.pack_into("<4I", data, 0x2C00, 0xFFFFFFFF, 0x12001200, 0x12001400, 0xFFFFFFFF)
    return data


class FactoryImageTests(unittest.TestCase):
    def test_packages_keep_boards_and_chinese_spi_instructions_separate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            table = root / "ftab.bin"
            table.write_bytes(ftab())
            for letter in ("A", "B"):
                stem = f"OpenWalkieTalkie-1.2.3-hw-{letter.lower()}"
                app = image(letter) + b"1.2.3-openwalkietalkie"
                (root / f"{stem}.bin").write_bytes(app)
                (root / f"{stem}.elf").write_bytes(elf(app))
                (root / f"OpenWalkieTalkie-1.2.3-recovery-hw-{letter.lower()}.bin").write_bytes(recovery(letter))
            for letter in ("A", "B"):
                board = "hw-" + letter.lower()
                output = root / f"OpenWalkieTalkie-1.2.3-factory-{board}"
                factory.package(root, table, "1.2.3", output, board)
                manifest = json.loads((output / "manifest.json").read_text())
                self.assertEqual(list(manifest["variants"]), [board])
                verifier.verify(output)
                with zipfile.ZipFile(str(output) + ".zip") as archive:
                    binaries = [name for name in archive.namelist() if name.endswith(".bin")]
                    self.assertEqual(binaries, [f"{output.name}/{manifest['variants'][board]['file']}"])
                    self.assertEqual(len(archive.read(binaries[0])), factory.FLASH_SIZE)
                readme = (output / "README.md").read_text()
                self.assertIn(f"仅用于 Rev {letter}", readme)
                self.assertNotIn("Rev " + ("B" if letter == "A" else "A"), readme)
                self.assertIn("0x00000000", readme)
                self.assertIn("全片读回校验", readme)
                for forbidden in ("@VERSION@", "@BOARD@", "@REV@", "sftool", "UART", "0x12000000"):
                    self.assertNotIn(forbidden, readme)
                # Reusing an extraction directory with another image must fail.
                extra = output / "wrong-board.bin"
                extra.write_bytes(b"other board")
                with self.assertRaisesRegex(ValueError, "额外文件"):
                    verifier.verify(output)
                extra.unlink()
                (output / "README.md").write_text(readme + "\nchanged")
                with self.assertRaisesRegex(ValueError, "SHA256"):
                    verifier.verify(output)

    def test_board_selection_and_archive_name_are_mandatory(self):
        for board, name in (("both", "combined"), ("hw-b", "OpenWalkieTalkie-1.2.3-factory-hw-a")):
            with self.assertRaises(ValueError):
                factory.package(Path("unused"), Path("unused"), "1.2.3", Path(name), board)

    def test_full_chip_merge_exact_placement_and_empty_unit_storage(self):
        for board in ("A", "B"):
            app, boot, table = image(board), recovery(board), ftab()
            merged = factory.merge_image(table, boot, app, "hw-" + board.lower())
            self.assertEqual(len(merged), 16 * 1024 * 1024)
            self.assertEqual(merged[:len(table)], table)
            self.assertEqual(merged[0x10000:0x10000 + len(boot)], boot)
            self.assertEqual(merged[0x20000:0x20000 + len(app)], app)
            for start, end in [(len(table), 0x10000), (0x10000 + len(boot), 0x20000),
                               (0x20000 + len(app), len(merged))]:
                self.assertEqual(merged[start:end], b"\xff" * (end - start))

    def test_mixed_boards_and_temporary_bootstrap_are_rejected(self):
        for boot, app in [(recovery("B"), image()), (recovery(), image("B")), (recovery(), bootstrap())]:
            with self.assertRaises(ValueError):
                factory.merge_image(ftab(), boot, app, "hw-a")

    def test_bad_boot_mapping_or_provisioned_flash_table_is_rejected(self):
        for offset in [0, 0x104, 4 + 3 * 16, 4 + 4 * 16, 4 + 2 * 16, 0x1200, 0x1206, 0x1208, 0x2C04]:
            table = ftab()
            table[offset] ^= 1
            with self.assertRaises(ValueError, msg=hex(offset)):
                factory.merge_image(table, recovery(), image(), "hw-a")

    def test_existing_bonds_and_oversized_images_are_rejected(self):
        app = bytearray(b"\xff" * (0x720010 - 0x20000))
        app[:len(image())] = image()
        app[0x710000 - 0x20000] = 0
        with self.assertRaises(ValueError):
            factory.merge_image(ftab(), recovery(), app, "hw-a")
        with self.assertRaises(ValueError):
            factory.merge_image(ftab(), recovery(), image() + b"\xff" * factory.FLASH_SIZE, "hw-a")
