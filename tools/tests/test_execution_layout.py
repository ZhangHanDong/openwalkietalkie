"""Exercise execution-layout checks with small ELF fixtures and unsafe mutations."""
import contextlib
import importlib.util
import io
from pathlib import Path
import struct
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("layout", Path(__file__).resolve().parents[1] / "check-xip-image.py")
layout = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(layout)


class Image:
    def __init__(self, board="A", psram=True, bootstrap=False):
        self.sections = {}
        self.symbols = {}
        base = layout.APP_BASE
        text = layout.PSRAM_START if psram else base + 0x100
        assets = text + 128 if psram else (base + 0x300 if bootstrap else layout.BONDS_END)
        self.add(".vector_table", base, base, struct.pack("<16I", 0x2007FC00, base + 0x101, text + 1, *([0] * 13)))
        if psram:
            self.add(".boot", base + 0x100, base + 0x100, bytes(64))
        self.add(".text", text, base + (0x200 if psram else 0x100), bytes(64))
        self.add(".rodata", text + 64, base + 0x240, f"OpenWalkieTalkie Rev {board} (fixture)".encode().ljust(64, b"\0"))
        # XIP constants must have equal virtual/physical addresses.
        if not psram:
            self.sections[".rodata"]["addr"] = base + 0x240
        self.add(".flash_assets", assets, base + 0x300 if bootstrap else layout.BONDS_END, bytes(64))
        self.add(".data", layout.SRAM_START, base + 0x400, bytes(16))
        for i, name in enumerate(("3pmu5enter", "10bond_flash11erase_block", "10bond_flash12program_slot") + (("11boot_update6nor_op",) if bootstrap else ())):
            self.symbols["_ZN" + name] = (layout.SRAM_START + i * 4 + 1, 2, 2)
        if psram:
            for section, stem in ((".text", "text"), (".rodata", "rodata"), (".flash_assets", "assets")):
                s = self.sections[section]
                self.symbols["__s" + stem] = (s["addr"], 0, 0)
                self.symbols["__e" + stem] = (s["addr"] + len(s["body"]), 0, 0)
                self.symbols["__si" + stem] = (s["load"], 0, 0)
            self.add(".vector_table_ram", layout.SRAM_START + 1024, None, bytes(64), align=1024)
            for name, value in (("__pre_init", base + 0x121), ("__vector_table", base),
                                ("__vector_table_ram", layout.SRAM_START + 1024),
                                ("__vector_table_ram_end", layout.SRAM_START + 1088),
                                ("_stack_end", layout.SRAM_START + 1088)):
                self.symbols[name] = (value, 0, 0)

    def add(self, name, addr, load, body, align=32):
        self.sections[name] = dict(addr=addr, load=load, body=body, align=align)

    def bytes(self):
        count = sum(s["load"] is not None for s in self.sections.values())
        data = bytearray(52 + 32 * count)
        data[:7] = b"\x7fELF\x01\x01\x01"
        headers = [bytes(40)]
        names = bytearray(b"\0")
        program = 52
        for name, s in self.sections.items():
            offset = len(data)
            body, addr = s["body"], s["addr"]
            kind = 8 if s["load"] is None else 1
            if kind != 8:
                data += body
                struct.pack_into("<8I", data, program, 1, offset, addr, s["load"], len(body), len(body), 7, s["align"])
                program += 32
            headers.append(struct.pack("<10I", len(names), kind, 2, addr, offset, len(body), 0, 0, s["align"], 0))
            names += name.encode() + b"\0"
        strings = bytearray(b"\0")
        symbols = bytearray(16)
        for name, (value, size, kind) in self.symbols.items():
            symbols += struct.pack("<IIIBBH", len(strings), value, size, kind, 0, 1)
            strings += name.encode() + b"\0"
        for name, body, kind, link, entsize in ((".symtab", symbols, 2, len(headers) + 1, 16),
                                               (".strtab", strings, 3, 0, 0)):
            headers.append(struct.pack("<10I", len(names), kind, 0, 0, len(data), len(body), link, 0, 1, entsize))
            names += name.encode() + b"\0"
            data += body
        index = len(headers)
        name_offset = len(names)
        names += b".shstrtab\0"
        headers.append(struct.pack("<10I", name_offset, 3, 0, 0, len(data), len(names), 0, 0, 1, 0))
        data += names
        struct.pack_into("<II", data, 28, 52, len(data))
        struct.pack_into("<5H", data, 42, 32, count, 40, len(headers), index)
        return data + b"".join(headers)


class ExecutionLayoutTests(unittest.TestCase):
    def check(self, image, bootstrap=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image.elf"
            path.write_bytes(image.bytes())
            with contextlib.redirect_stdout(io.StringIO()):
                layout.check(path, bootstrap)

    def test_normal_and_bootstrap_board_layouts(self):
        for bootstrap in (False, True):
            for board, psram in (("A", True), ("B", False)):
                with self.subTest(board=board, bootstrap=bootstrap):
                    self.check(Image(board, psram, bootstrap), bootstrap)

    def test_board_features_cannot_silently_choose_wrong_layout(self):
        for board, psram in (("A", False), ("B", True)):
            with self.subTest(board=board), self.assertRaises(AssertionError):
                self.check(Image(board, psram))

    def test_font_copy_must_fit_and_match_startup_bounds(self):
        for field, value in (("addr", layout.BONDS_END), ("addr", layout.PSRAM_SCRATCH - 32),
                             ("load", layout.BONDS_START)):
            image = Image()
            image.sections[".flash_assets"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                self.check(image)
        image = Image()
        image.symbols["__eassets"] = (image.symbols["__eassets"][0] - 32, 0, 0)
        with self.assertRaisesRegex(AssertionError, "copy boundary"):
            self.check(image)

    def test_vector_copy_must_be_outside_stack_and_handlers_in_ram(self):
        image = Image()
        image.symbols["_stack_end"] = (layout.SRAM_START + 1024, 0, 0)
        with self.assertRaisesRegex(AssertionError, "stack overlaps"):
            self.check(image)
        image = Image()
        body = bytearray(image.sections[".vector_table"]["body"])
        struct.pack_into("<I", body, 8, layout.APP_BASE + 0x121)
        image.sections[".vector_table"]["body"] = body
        with self.assertRaisesRegex(AssertionError, "handler still targets NOR"):
            self.check(image)

    def test_sram_flash_writer_cannot_move_to_psram(self):
        image = Image()
        image.symbols["_ZN10bond_flash11erase_block"] = (layout.PSRAM_START + 1, 2, 2)
        with self.assertRaisesRegex(AssertionError, "must stay in SRAM"):
            self.check(image)
