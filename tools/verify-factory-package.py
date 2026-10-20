#!/usr/bin/env python3
"""Verify an extracted factory package without any third-party dependencies."""
import hashlib
import json
from pathlib import Path
import sys


def verify(directory):
    directory = directory.resolve()
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    variants = manifest["variants"]
    if len(variants) != 1 or next(iter(variants)) not in ("hw-a", "hw-b"):
        raise ValueError("工厂包必须且只能包含一种硬件版本")
    board, variant = next(iter(variants.items()))
    name = f"Rev-{board[-1].upper()}/OpenWalkieTalkie-{manifest['firmware_version']}-factory-{board}.bin"
    if variant["file"] != name or variant["size"] != 16 * 1024 * 1024:
        raise ValueError("镜像文件名或大小与主板版本不符")
    required = {"README.md", "manifest.json", "verify.py", name}
    actual = {p.relative_to(directory).as_posix() for p in directory.rglob("*") if p.is_file()}
    if actual != required | {"SHA256SUMS"}:
        raise ValueError("工厂包文件不完整或含有额外文件，请使用全新目录解压")
    checked = set()
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        if name not in required or name in checked:
            raise ValueError(f"校验清单包含额外或重复文件：{name}")
        path = (directory / name).resolve()
        if directory not in path.parents:
            raise ValueError("校验路径超出本包目录")
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"SHA256 校验失败：{name}")
        print(f"OK  {name}")
        checked.add(name)
    if checked != required:
        raise ValueError("校验清单必须包含全部 4 个文件")
    data = (directory / variant["file"]).read_bytes()
    if len(data) != variant["size"] or hashlib.sha256(data).hexdigest() != variant["sha256"]:
        raise ValueError("镜像与组件清单不符")
    print(f"Rev {board[-1].upper()} 工厂包校验通过，共 4 个文件。未访问任何设备。")


if __name__ == "__main__":
    try:
        verify(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent)
    except (OSError, ValueError, KeyError, TypeError) as error:
        sys.exit(str(error))
