#!/usr/bin/env python3
"""Validate versions and build release assets consumed by the app updater."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
APP_BASE = 0x12020000
APP_SIZE = 7 * 1024 * 1024 - 64 * 1024
REPOSITORY = "conol-ai/openwalkietalkie"


def versions():
    def package_version(manifest, package):
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--manifest-path", str(ROOT / manifest), "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT))
        return next(p["version"] for p in metadata["packages"] if p["name"] == package)
    host = package_version("host/Cargo.toml", "openwalkietalkie-app")
    firmware = package_version("Cargo.toml", "openwalkietalkie-fw")
    for version in (host, firmware):
        if not re.fullmatch(r"\d+\.\d+\.\d+", version):
            raise ValueError("release versions must be MAJOR.MINOR.PATCH")
    return host, firmware


def image_from_elf(data, board):
    if len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        raise ValueError("expected a 32-bit little-endian ELF")
    phoff = struct.unpack_from("<I", data, 28)[0]
    size, count = struct.unpack_from("<HH", data, 42)
    if size < 32 or phoff + size * count > len(data):
        raise ValueError("invalid ELF program headers")
    segments, psram = [], False
    for i in range(count):
        kind, offset, vaddr, paddr, length, _, flags, _ = struct.unpack_from("<8I", data, phoff + i * size)
        if kind != 1 or not length:
            continue
        if offset + length > len(data):
            raise ValueError("truncated ELF segment")
        if APP_BASE <= paddr < APP_BASE + APP_SIZE:
            if paddr + length > APP_BASE + APP_SIZE:
                raise ValueError("segment exceeds application flash")
            segments.append((paddr, data[offset:offset + length]))
            psram |= 0x60000000 <= vaddr < 0x70000000 and bool(flags & 1)
        elif not 0x20000000 <= paddr < 0x70000000:
            raise ValueError("segment targets bootloader or another flash area")
    segments.sort()
    if not segments or segments[0][0] != APP_BASE or not psram:
        raise ValueError("image must start at application base and execute from PSRAM")
    end = max(addr + len(body) for addr, body in segments)
    image = bytearray(b"\xff" * (end - APP_BASE))
    previous = APP_BASE
    for addr, body in segments:
        if addr < previous:
            raise ValueError("overlapping flash segments")
        image[addr - APP_BASE:addr - APP_BASE + len(body)] = body
        previous = addr + len(body)
    validate_image(image, board)
    return image


def validate_image(image, board):
    if board not in ("hw-a", "hw-b") or not 8 <= len(image) <= APP_SIZE:
        raise ValueError("invalid board or image size")
    sp, pc = struct.unpack_from("<II", image)
    if not 0x20000000 <= sp <= 0x20080000 or not pc & 1 or not APP_BASE <= (pc & ~1) < APP_BASE + len(image):
        raise ValueError("invalid application vector table")
    marker = b"OpenWalkieTalkie Rev " + (b"A" if board == "hw-a" else b"B") + b" ("
    if marker not in image:
        raise ValueError("firmware does not identify the expected board")


def asset(directory, name, tag):
    path = directory / name
    return {"name": name, "url": f"https://github.com/{REPOSITORY}/releases/download/{tag}/{name}",
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "size": path.stat().st_size}


def manifest(directory, tag):
    app, firmware = versions()
    if tag != f"v{app}":
        raise ValueError("tag does not match host workspace version")
    variants = {}
    for board in ("hw-a", "hw-b"):
        name = f"OpenWalkieTalkie-{firmware}-{board}.bin"
        validate_image((directory / name).read_bytes(), board)
        variants[board] = asset(directory, name, tag)
    return {"schema": 1, "product": "openwalkietalkie",
            "release_url": f"https://github.com/{REPOSITORY}/releases/tag/{tag}",
            "app": {"version": app, "macos": {arch: asset(directory, f"OpenWalkieTalkie-{app}-macos-{arch}.dmg", tag)
                                                for arch in ("aarch64", "x86_64")}},
            "firmware": {"version": firmware, "protocol": 1, "app_base": APP_BASE,
                         "app_size": APP_SIZE, "variants": variants}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("tag")
    build = sub.add_parser("image")
    build.add_argument("board", choices=["hw-a", "hw-b"])
    build.add_argument("elf", type=Path)
    build.add_argument("output", type=Path)
    release = sub.add_parser("manifest")
    release.add_argument("tag")
    release.add_argument("directory", type=Path)
    args = parser.parse_args()
    app, firmware = versions()
    if args.command == "validate":
        if args.tag != f"v{app}":
            raise ValueError("release tag must match host/Cargo.toml workspace version")
        subprocess.run(["git", "merge-base", "--is-ancestor", "HEAD", "origin/main"], cwd=ROOT, check=True)
        lines = f"tag={args.tag}\napp_version={app}\nfirmware_version={firmware}\n"
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write(lines)
        print(lines, end="")
    elif args.command == "image":
        args.output.mkdir(parents=True, exist_ok=True)
        name = f"OpenWalkieTalkie-{firmware}-{args.board}"
        data = args.elf.read_bytes()
        (args.output / f"{name}.bin").write_bytes(image_from_elf(data, args.board))
        (args.output / f"{name}.elf").write_bytes(data)
    else:
        value = manifest(args.directory, args.tag)
        (args.directory / "release-manifest.json").write_text(json.dumps(value, indent=2) + "\n")
        files = sorted(p for p in args.directory.iterdir() if p.is_file() and p.name != "SHA256SUMS")
        (args.directory / "SHA256SUMS").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in files))


if __name__ == "__main__":
    main()
