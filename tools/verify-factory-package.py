#!/usr/bin/env python3
"""Verify an extracted factory package without any third-party dependencies."""
import hashlib
from pathlib import Path
import sys


def verify(directory):
    directory = directory.resolve()
    count = 0
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        path = (directory / name).resolve()
        if directory not in path.parents:
            raise ValueError("checksum path escapes package")
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"SHA256 mismatch: {name}")
        print(f"OK  {name}")
        count += 1
    if count != 5:
        raise ValueError(f"expected 5 checked files, got {count}")
    print("Factory package checksums verified. No device was accessed.")


if __name__ == "__main__":
    try:
        verify(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent)
    except (OSError, ValueError) as error:
        sys.exit(str(error))
