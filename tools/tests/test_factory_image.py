import importlib.util
from pathlib import Path
import struct
import unittest

from test_release import image, recovery, bootstrap

SPEC = importlib.util.spec_from_file_location("factory_image", Path(__file__).resolve().parents[1] / "factory-image.py")
factory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(factory)


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
