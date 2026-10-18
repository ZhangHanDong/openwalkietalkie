"""Exercise the production bootloader renderer/state without board hardware."""
from pathlib import Path
import os
import re
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RecoveryUiTests(unittest.TestCase):
    def test_usb_endpoints_match_sf32lb52x_fixed_fifo_directions(self):
        source = (ROOT / "native/bootloader/owt/recovery.c").read_text()
        endpoints = {name: int(value, 16) for name, value in re.findall(
            r"#define\s+((?:HID|BULK)_(?:IN|OUT)_EP)\s+(0x[0-9a-fA-F]+)", source)}
        self.assertEqual(len(endpoints), 4)
        self.assertEqual(len(set(endpoints.values())), 4)
        for name, address in endpoints.items():
            if "_IN_" in name:
                self.assertTrue(address & 0x80)
                self.assertIn(address & 15, (1, 5, 6, 7))
            else:
                self.assertFalse(address & 0x80)
                self.assertIn(address, (1, 2, 3, 4))

    def test_production_report_handler(self):
        # Compile the actual handler unchanged. Hardware USB descriptors, MMIO
        # and flash primitives are replaced by the harness; no device is used.
        source = (ROOT / "native/bootloader/owt/recovery.c").read_text()
        extracted = re.findall(r"^#define .*$", source, re.MULTILINE)
        start = source.index("static uint8_t sector_buf[")
        extracted.append(source[start:source.index("static void usbd_event_handler", start)])
        for name in ("crc32_of", "rd32", "wr32", "all_ff", "sector_matches", "begin_transfer", "commit_sector", "finish_transfer", "handle_bulk", "handle_report"):
            match = re.search(rf"^static [^\n]+ {name}\(", source, re.MULTILINE)
            self.assertIsNotNone(match, name)
            start = match.start()
            extracted.append(source[start:source.index("\n}\n", start) + 3])
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            (directory / "recovery_handler.inc").write_text("\n".join(extracted))
            binary = directory / "recovery-protocol"
            subprocess.run([
                *shlex.split(os.environ.get("CC", "cc")), "-std=c11", "-O2",
                "-Wall", "-Wextra", "-Werror", "-I", str(directory),
                "-I", str(ROOT / "native/bootloader/owt"),
                str(ROOT / "tests/recovery_protocol.c"),
                str(ROOT / "native/bootloader/owt/recovery_ui.c"),
                "-o", str(binary),
            ], check=True)
            subprocess.run([str(binary)], check=True)

    def test_progress_and_renderer(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "recovery-ui"
            subprocess.run([
                *shlex.split(os.environ.get("CC", "cc")), "-std=c11", "-O2",
                "-Wall", "-Wextra", "-Werror",
                str(ROOT / "tests/recovery_ui.c"),
                str(ROOT / "native/bootloader/owt/recovery_ui.c"),
                "-o", str(binary),
            ], check=True)
            subprocess.run([str(binary)], check=True)
