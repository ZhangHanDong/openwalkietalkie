#!/usr/bin/env python3
"""Verify that a built Embassy ELF executes and reads fonts directly from NOR."""
import pathlib
import struct
import argparse

APP_BASE = 0x12020000
APP_END = 0x13000000
BONDS_START, BONDS_END = 0x12700000, 0x12720000


def check(path, bootstrap=False):
    data = pathlib.Path(path).read_bytes()
    assert data[:7] == b"\x7fELF\x01\x01\x01", "expected ELF32 little-endian"
    phoff, shoff = struct.unpack_from("<II", data, 28)
    phsize, phcount, shsize, shcount, strings = struct.unpack_from("<5H", data, 42)
    sections = [struct.unpack_from("<10I", data, shoff + i * shsize) for i in range(shcount)]
    names = sections[strings]
    names = data[names[4]:names[4] + names[5]]
    sections = {names[s[0]:].split(b"\0", 1)[0].decode(): s for s in sections}
    for name in (".vector_table", ".text", ".rodata", ".flash_assets"):
        section = sections[name]
        assert section[5] and APP_BASE <= section[3] < section[3] + section[5] <= APP_END, f"{name} must stay in application NOR"
        assert section[3] % section[8] == 0, f"{name} alignment"
        assert section[3] + section[5] <= BONDS_START or section[3] >= BONDS_END, f"{name} overlaps Bluetooth storage"
    if bootstrap:
        assets = sections[".flash_assets"]
        assert assets[3] + assets[5] <= BONDS_START, "bootstrap assets must fit in lower NOR"
    else:
        assert sections[".flash_assets"][3] == BONDS_END, "font assets must use upper NOR"
    vector = sections[".vector_table"]
    sp, reset = struct.unpack_from("<II", data, vector[4])
    code = sections[".text"]
    assert 0x20000000 <= sp <= 0x20080000 and reset & 1, "invalid reset vectors"
    assert code[3] <= (reset & ~1) < code[3] + code[5], "Reset must execute in NOR"
    stored = loaded = 0
    for i in range(phcount):
        kind, offset, vaddr, paddr, size, memory, flags, _ = struct.unpack_from("<8I", data, phoff + i * phsize)
        if kind != 1:
            continue
        assert not size or not 0x60000000 <= vaddr < 0x60800000, "startup must not copy code/fonts into PSRAM"
        if APP_BASE <= vaddr < APP_END and size:
            assert vaddr == paddr, "NOR code/constants must execute in place"
        if APP_BASE <= paddr < APP_END and size:
            assert paddr + size <= APP_END, "segment exceeds physical flash"
            assert paddr + size <= BONDS_START or paddr >= BONDS_END, "segment overlaps Bluetooth storage"
            stored = max(stored, paddr + size - APP_BASE)
            loaded += size
    if bootstrap:
        assert APP_BASE + stored <= BONDS_START, "bootstrap must fit protocol-1 recovery"
    # These function names are identifiable in Rust's symbol strings without
    # requiring an ARM binutils installation. Their bodies must stay in SRAM.
    symbols = sections[".symtab"]
    string_section = struct.unpack_from("<10I", data, shoff + symbols[6] * shsize)
    strings = data[string_section[4]:string_section[4] + string_section[5]]
    found = set()
    required = ("3pmu5enter", "10bond_flash11erase_block", "10bond_flash12program_slot")
    if bootstrap:
        required += ("11boot_update6nor_op",)
    for at in range(symbols[4], symbols[4] + symbols[5], symbols[9]):
        name, value, size, info, other, section = struct.unpack_from("<IIIBBH", data, at)
        name = strings[name:].split(b"\0", 1)[0].decode()
        # Long-call thunks in NOR include the callee's mangled name. They only
        # jump into SRAM; check the Rust function body rather than those stubs.
        if info & 0xF != 2 or not name.startswith(("_R", "_ZN")):
            continue
        for marker in required:
            if marker in name:
                assert 0x20000000 <= value < value + size <= 0x20080000, f"{marker} must stay in SRAM"
                found.add(marker)
    # Bond storage and PMU shutdown are main-board features, absent on LilyGo.
    if b"OpenWalkieTalkie Rev " in data:
        assert found == set(required), "missing SRAM flash-writing/shutdown routines"
    print(f"XIP verified: {loaded} load bytes ({stored} byte image span including padding); code/fonts in NOR, {len(found)} critical routines in SRAM")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf")
    parser.add_argument("--bootstrap", action="store_true", help="check a temporary lower-bank bootloader updater")
    args = parser.parse_args()
    check(args.elf, args.bootstrap)
