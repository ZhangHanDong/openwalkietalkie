#!/usr/bin/env python3
"""Verify Embassy execution layout: Rev A copies to RAM; Rev B/LilyGo use NOR."""
import pathlib
import struct
import argparse

APP_BASE = 0x12020000
APP_END = 0x13000000
BONDS_START, BONDS_END = 0x12700000, 0x12720000
PSRAM_START, PSRAM_SCRATCH = 0x60000000, 0x60700000
SRAM_START, SRAM_END = 0x20000000, 0x2007FC00


def check(path, bootstrap=False):
    data = pathlib.Path(path).read_bytes()
    assert data[:7] == b"\x7fELF\x01\x01\x01", "expected ELF32 little-endian"
    psram = b"OpenWalkieTalkie Rev A (" in data
    phoff, shoff = struct.unpack_from("<II", data, 28)
    phsize, phcount, shsize, shcount, strings = struct.unpack_from("<5H", data, 42)
    section_headers = [struct.unpack_from("<10I", data, shoff + i * shsize) for i in range(shcount)]
    names = section_headers[strings]
    names = data[names[4]:names[4] + names[5]]
    sections = {names[s[0]:].split(b"\0", 1)[0].decode(): s for s in section_headers}
    segments = [struct.unpack_from("<8I", data, phoff + i * phsize) for i in range(phcount)]
    segments = [s for s in segments if s[0] == 1 and s[4]]
    symbols = sections[".symtab"]
    strings = section_headers[symbols[6]]
    strings = data[strings[4]:strings[4] + strings[5]]
    symbol_values = {}
    functions = []
    for at in range(symbols[4], symbols[4] + symbols[5], symbols[9]):
        name, value, size, info, _, _ = struct.unpack_from("<IIIBBH", data, at)
        name = strings[name:].split(b"\0", 1)[0].decode()
        symbol_values[name] = value
        if info & 0xF == 2:
            functions.append((name, value, size))

    def section(name, start, end):
        assert name in sections, f"missing {name}"
        s = sections[name]
        assert s[5] and start <= s[3] < s[3] + s[5] <= end, f"{name} outside required execution region"
        assert s[3] % s[8] == 0, f"{name} alignment"
        return s

    def load_address(s):
        for _, offset, vaddr, paddr, size, _, _, _ in segments:
            if vaddr <= s[3] and s[3] + s[5] <= vaddr + size:
                assert s[4] == offset + s[3] - vaddr, "section/segment offset mismatch"
                return paddr + s[3] - vaddr
        raise AssertionError("section has no flash load image")

    vector = section(".vector_table", APP_BASE, BONDS_START)
    assert vector[3] == APP_BASE, "vector table must start the application"
    sp, reset = struct.unpack_from("<II", data, vector[4])
    assert SRAM_START <= sp <= SRAM_END and sp % 8 == 0 and reset & 1, "invalid reset vectors"
    boot = section(".boot" if psram else ".text", APP_BASE, BONDS_START)
    assert boot[3] <= (reset & ~1) < boot[3] + boot[5], "Reset must execute in NOR"
    if psram:
        for name, stem in ((".text", "text"), (".rodata", "rodata"), (".flash_assets", "assets")):
            s = section(name, PSRAM_START, PSRAM_SCRATCH)
            assert s[3] % 32 == 0 and s[5] % 32 == 0, f"{name} must span whole cache lines"
            for symbol, expected in ((f"__s{stem}", s[3]), (f"__e{stem}", s[3] + s[5]),
                                     (f"__si{stem}", load_address(s))):
                assert symbol_values.get(symbol) == expected, f"invalid copy boundary {symbol}"
            assert load_address(s) % 4 == 0, f"{name} word alignment"
        assert sections[".text"][3] == PSRAM_START, "code must start at PSRAM base"
        assert symbol_values["__etext"] <= symbol_values["__srodata"], "code/constant overlap"
        assert symbol_values["__erodata"] <= symbol_values["__sassets"], "constant/asset overlap"
        assert boot[3] <= (symbol_values.get("__pre_init", 0) & ~1) < boot[3] + boot[5], "copy routine must stay in NOR boot section"
        ram_vectors = section(".vector_table_ram", SRAM_START, SRAM_END)
        assert ram_vectors[1] == 8 and ram_vectors[3] % 1024 == 0, "RAM vectors must be aligned NOLOAD storage"
        assert ram_vectors[5] == vector[5], "RAM vector table size mismatch"
        assert symbol_values.get("__vector_table_ram") == ram_vectors[3], "invalid RAM vector start"
        assert symbol_values.get("__vector_table_ram_end") == ram_vectors[3] + vector[5], "invalid RAM vector end"
        assert symbol_values.get("__vector_table") == APP_BASE, "invalid vector copy source"
        assert symbol_values.get("_stack_end", 0) >= ram_vectors[3] + vector[5], "stack overlaps RAM vectors"
        # Every exception/IRQ after Reset must target RAM, including defaults.
        for at in range(vector[4] + 8, vector[4] + vector[5], 4):
            handler, = struct.unpack_from("<I", data, at)
            assert handler == 0 or (handler & 1 and (
                PSRAM_START <= handler < PSRAM_SCRATCH or SRAM_START <= handler < SRAM_END
            )), "interrupt handler still targets NOR"
        buffers = sections.get(".psram_bss")
        if buffers and buffers[5]:
            assert symbol_values["__eassets"] <= buffers[3] <= buffers[3] + buffers[5] <= PSRAM_SCRATCH, "PSRAM buffers overlap code/assets or scratch"
    else:
        section(".rodata", APP_BASE, BONDS_START)
        section(".flash_assets", APP_BASE if bootstrap else BONDS_END, BONDS_START if bootstrap else APP_END)

    assets = sections[".flash_assets"]
    asset_load = load_address(assets)
    if bootstrap:
        assert APP_BASE <= asset_load < asset_load + assets[5] <= BONDS_START, "bootstrap assets must fit in lower NOR"
    else:
        assert asset_load == BONDS_END, "font load image must use upper NOR"
    stored = loaded = 0
    physical_end = APP_BASE
    for _, offset, vaddr, paddr, size, memory, flags, _ in sorted(segments, key=lambda s: s[3]):
        assert offset + size <= len(data) and size <= memory, "truncated load segment"
        assert APP_BASE <= paddr < paddr + size <= APP_END, "segment exceeds application flash"
        assert paddr + size <= BONDS_START or paddr >= BONDS_END, "segment overlaps Bluetooth storage"
        assert paddr >= physical_end, "overlapping flash load images"
        physical_end = paddr + size
        if PSRAM_START <= vaddr < 0x60800000:
            assert psram, "only Rev A may copy code/fonts into PSRAM"
            assert vaddr + size <= symbol_values["__eassets"], "PSRAM load segment outside startup copy"
        elif APP_BASE <= vaddr < APP_END:
            assert vaddr == paddr, "NOR boot/code must execute in place"
            if psram:
                assert vaddr + size <= boot[3] + boot[5], "Rev A runtime data/code left in NOR"
        else:
            assert SRAM_START <= vaddr < vaddr + size <= SRAM_END, "unknown runtime load region"
        stored = max(stored, paddr + size - APP_BASE)
        loaded += size
    if bootstrap:
        assert APP_BASE + stored <= BONDS_START, "bootstrap must fit protocol-1 recovery"
    # Check the Rust bodies, not linker-generated long-call thunks.
    found = set()
    required = ("3pmu5enter", "10bond_flash11erase_block", "10bond_flash12program_slot")
    if bootstrap:
        required += ("11boot_update6nor_op",)
    for name, value, size in functions:
        if not name.startswith(("_R", "_ZN")):
            continue
        for marker in required:
            if marker in name:
                assert SRAM_START <= value < value + size <= SRAM_END, f"{marker} must stay in SRAM"
                found.add(marker)
    if b"OpenWalkieTalkie Rev " in data:
        assert found == set(required), "missing SRAM flash-writing/shutdown routines"
    mode = "Rev A code/fonts in PSRAM, vectors in SRAM" if psram else "code/fonts in NOR"
    print(f"Execution layout verified: {loaded} load bytes ({stored} byte image span including padding); {mode}, {len(found)} critical routines in SRAM")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf")
    parser.add_argument("--bootstrap", action="store_true", help="check a temporary lower-bank bootloader updater")
    args = parser.parse_args()
    check(args.elf, args.bootstrap)
