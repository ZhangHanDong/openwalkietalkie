#!/usr/bin/env python3
"""Check application flash bounds and the SRAM-only clock transition helpers."""
import pathlib
import struct
import sys

image = pathlib.Path(sys.argv[1]).read_bytes()
if image[:7] != b"\x7fELF\x01\x01\x01":
    sys.exit("Expected a 32-bit little-endian ELF")
header = struct.unpack_from("<16sHHIIIIIHHHHHH", image)
if header[2] != 40:
    sys.exit("Expected an ARM ELF")
entry, offset, stride, count = header[4], header[5], header[9], header[10]
if not 0x12020000 <= (entry & ~1) < 0x12700000 or stride < 32:
    sys.exit("Invalid application entry point or program headers")
total = 0
for index in range(count):
    kind, source, virtual, address, size, memory_size, flags, alignment = struct.unpack_from(
        "<IIIIIIII", image, offset + index * stride
    )
    if kind != 1 or size == 0:
        continue
    if source + size > len(image) or not (0x12020000 <= address < address + size <= 0x12700000):
        sys.exit(f"Unsafe load segment at 0x{address:08x} ({size} bytes)")
    total += size
if not total:
    sys.exit("ELF has no application load segments")

# The SDK's placement rules depend on object filenames. A wrapper renamed from
# bsp_init.c to clock.c silently moved divider accessors into the NOR being
# reconfigured, risking cache-dependent HardFaults on 48 -> 240 MHz wake. Rust's
# weak memset also evades the SDK's *memset*.o selector. Check the linked code,
# including symbol size and its loadable RAM section, rather than source names.
section_offset, section_stride, section_count = header[6], header[11], header[12]
if section_stride < 40:
    sys.exit("Invalid ELF section headers")
sections = [struct.unpack_from("<IIIIIIIIII", image, section_offset + i * section_stride)
            for i in range(section_count)]
required = {
    "sifli_pm_run", "HAL_PreInit", "HAL_RCC_HCPU_ConfigHCLK",
    "HAL_RCC_HCPU_EnableDLL2", "HAL_RCC_HCPU_ClockSelect",
    "BSP_GetFlash1DIV", "BSP_GetFlash2DIV", "BSP_SetFlash1DIV", "BSP_SetFlash2DIV",
    "rt_psram_init", "bsp_psramc_init", "BSP_Flash_hw2_init", "BSP_Flash_Init_WithID",
    "memset", "rt_memset",
}
found = set()
for section in sections:
    if section[1] != 2:  # SHT_SYMTAB
        continue
    strings = sections[section[6]]
    names = image[strings[4]:strings[4] + strings[5]]
    for pos in range(section[4], section[4] + section[5], section[9]):
        name_at, value, size, info, other, index = struct.unpack_from("<IIIBBH", image, pos)
        name = names[name_at:names.find(b"\0", name_at)].decode("utf-8")
        if name not in required or index == 0:
            continue
        address = value & ~1  # Thumb function pointer
        if not 0 < index < len(sections):
            sys.exit(f"Clock transition helper has no code section: {name}")
        code = sections[index]
        if (info & 15 != 2 or not size or code[1] != 1 or code[2] & 6 != 6
                or not 0x20000000 <= code[3] <= address < address + size
                <= code[3] + code[5] <= 0x20080000):
            sys.exit(f"Clock transition helper must execute from internal SRAM: {name} at 0x{address:08x}")
        found.add(name)
if missing := required - found:
    sys.exit("Missing clock transition helpers: " + ", ".join(sorted(missing)))
print(f"Application verified: {total:,} load bytes; bootloader and bond stores preserved")
print(f"Clock transition verified: {len(found)} required helpers execute from internal SRAM")
