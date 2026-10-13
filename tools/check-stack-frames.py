#!/usr/bin/env python3
"""Fail when a function of ours reserves a large stack frame.

  tools/check-stack-frames.py ELF --match REGEX [--limit BYTES] [--objdump PATH]

Disassembles ELF and reads every function's prologue: the immediates of
`sub sp, #n` (LLVM and GCC split big frames into several), or the constant a
register was loaded with before `sub sp, rX`. A function whose demangled name
matches REGEX and whose frame exceeds the limit (default 2048) is an error:
`UiState`, `Deck`, `HostMsg` and `Signature` are kilobytes, and the rule is that
firmware paths fill them in place instead of moving them through the stack
(the native main thread has 24 KB in all). Everything else over the limit is
listed for information only (SDK audio codecs, the BLE pairing crypto).
"""
import argparse
import re
import subprocess
import sys

FUNCTION = re.compile(r"^[0-9a-f]+ <(.+)>:$")
SUB_IMMEDIATE = re.compile(r"\bsubw?(?:\.w)?\s+sp,\s*(?:sp,\s*)?#(\d+)")
SUB_REGISTER = re.compile(r"\bsub(?:\.w)?\s+sp,\s*(?:sp,\s*)?(r\d+|ip|lr)\b")
LOAD_CONSTANT = re.compile(r"\bmovw?(?:\.w)?\s+(r\d+|ip|lr),\s*#(\d+)")
LOAD_HIGH = re.compile(r"\bmovt\s+(r\d+|ip|lr),\s*#(\d+)")
# The prologue ends at the first branch or call.
BRANCH = re.compile(
    r"^\s*[0-9a-f]+:\s+(?:b|bl|blx|bx|cbz|cbnz|"
    r"b(?:eq|ne|cs|cc|hs|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le))(?:\.[nw])?\s"
)
PROLOGUE_INSTRUCTIONS = 24


def frames(disassembly):
    name, size, constants, seen = None, 0, {}, 0
    for line in disassembly.splitlines():
        start = FUNCTION.match(line)
        if start:
            if name is not None:
                yield name, size
            name, size, constants, seen = start.group(1), 0, {}, 0
            continue
        if name is None or seen >= PROLOGUE_INSTRUCTIONS or ":\t" not in line:
            continue
        if BRANCH.match(line):
            seen = PROLOGUE_INSTRUCTIONS
            continue
        seen += 1
        if found := SUB_IMMEDIATE.search(line):
            size += int(found.group(1))
        elif found := SUB_REGISTER.search(line):
            size += constants.get(found.group(1), 0)
        elif found := LOAD_CONSTANT.search(line):
            constants[found.group(1)] = int(found.group(2))
        elif found := LOAD_HIGH.search(line):
            register = found.group(1)
            constants[register] = constants.get(register, 0) | int(found.group(2)) << 16
    if name is not None:
        yield name, size


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("elf")
    parser.add_argument("--match", required=True, help="regex of the functions held to the limit")
    parser.add_argument("--limit", type=int, default=2048)
    parser.add_argument("--objdump", default="arm-none-eabi-objdump")
    args = parser.parse_args()
    ours = re.compile(args.match)
    dump = subprocess.run(
        [args.objdump, "-d", "-C", "--no-show-raw-insn", args.elf],
        check=True, capture_output=True, text=True,
    ).stdout
    sizes = {}
    for name, size in frames(dump):
        sizes[name] = max(size, sizes.get(name, 0))
    checked = [name for name in sizes if ours.search(name)]
    if not checked:
        print(f"check-stack-frames: no function matches {args.match!r} in {args.elf}")
        return 1
    failed = False
    for name, size in sorted(sizes.items(), key=lambda item: -item[1]):
        if size <= args.limit:
            break
        mine = bool(ours.search(name))
        failed |= mine
        print(f"{'ERROR' if mine else 'info '} {size:6d} B  {name}")
    biggest = max(checked, key=sizes.get)
    print(
        f"check-stack-frames: {len(checked)} checked functions, largest "
        f"{sizes[biggest]} B ({biggest}), limit {args.limit} B"
    )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
