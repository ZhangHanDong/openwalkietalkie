#!/usr/bin/env python3
"""Package full-chip Rev A/B factory images from validated release components.

Generate a fresh ftab with the pinned SDK's gen_ftab.py first. Never use a
device dump: the full image intentionally starts with empty unit storage.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("factory_release", ROOT / "tools/release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

FLASH_BASE = 0x12000000
FLASH_SIZE = 0x1000000
BOOT_OFFSET = 0x10000
APP_OFFSET = 0x20000


def validate_ftab(data):
    if len(data) != 0x2C10 or struct.unpack_from("<I", data)[0] != 0x53454346:
        raise ValueError("expected a fresh SDK SF32LB52 flash table (11280 bytes)")
    # Match the project's pinned SDK layout, including its 128 KiB boot
    # partition descriptor. The load header is limited to our 64 KiB slot.
    for index, expected in {
        0: (FLASH_BASE, 0x8000, 0, 0),
        3: (FLASH_BASE + BOOT_OFFSET, 0x20000, 0x20020000, 0),
        4: (FLASH_BASE + APP_OFFSET, 0x6E0000, FLASH_BASE + APP_OFFSET, 0),
        7: (FLASH_BASE + BOOT_OFFSET, 0x20000, 0x20020000, 0),
        8: (FLASH_BASE + APP_OFFSET, 0x6E0000, FLASH_BASE + APP_OFFSET, 0),
    }.items():
        if struct.unpack_from("<4I", data, 4 + index * 16) != expected:
            raise ValueError(f"unexpected flash table partition {index}")
    for index in (1, 2, 5, 6, 9, 10, 11, 12, 13, 14, 15):
        if any(data[4 + index * 16:4 + (index + 1) * 16]):
            raise ValueError("unexpected extra flash table partition")
    if any(data[0x104:0x1000]):
        raise ValueError("factory flash table must not contain keys or provisioned data")
    for index in range(14):
        offset = 0x1000 + index * 512
        expected = {1: (0x10000, 512, 2), 2: (0x6E0000, 512, 2),
                    6: (0x6E0000, 512, 2)}.get(index, (0xFFFFFFFF, 0, 0))
        if struct.unpack_from("<IHH", data, offset) != expected or any(data[offset + 8:offset + 512]):
            raise ValueError(f"unexpected flash table image header {index}")
    if struct.unpack_from("<4I", data, 0x2C00) != (0xFFFFFFFF, 0x12001200, 0x12001400, 0xFFFFFFFF):
        raise ValueError("unexpected flash table boot pointers")


def merge_image(ftab, recovery, application, board):
    validate_ftab(ftab)
    release.validate_recovery(recovery, board)
    release.validate_image(application, board)
    merged = bytearray(b"\xff" * FLASH_SIZE)
    for offset, data in ((0, ftab), (BOOT_OFFSET, recovery), (APP_OFFSET, application)):
        merged[offset:offset + len(data)] = data
    if len(merged) != FLASH_SIZE or any(b != 255 for b in merged[0x700000:0x720000]):
        raise ValueError("factory image must have empty Bluetooth/settings storage")
    return bytes(merged)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def package(firmware_dir, ftab_path, version, output):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("version must be MAJOR.MINOR.PATCH")
    if output.exists():
        raise ValueError("output directory already exists; use a fresh destination")
    archive = output.with_name(output.name + ".zip")
    if archive.exists():
        raise ValueError("output archive already exists")
    ftab = ftab_path.read_bytes()
    images, variants = {}, {}
    for board in ("hw-a", "hw-b"):
        app_path = firmware_dir / f"OpenWalkieTalkie-{version}-{board}.bin"
        recovery_path = firmware_dir / f"OpenWalkieTalkie-{version}-recovery-{board}.bin"
        app, recovery = app_path.read_bytes(), recovery_path.read_bytes()
        if (version + "-openwalkietalkie").encode() not in app:
            raise ValueError(f"application does not contain firmware version {version}")
        # Require the raw release image to agree with its checked ELF.
        elf = app_path.with_suffix(".elf").read_bytes()
        if release.image_from_elf(elf, board) != app:
            raise ValueError(f"{board} application differs from its ELF")
        merged = merge_image(ftab, recovery, app, board)
        name = f"Rev-{board[-1].upper()}/OpenWalkieTalkie-{version}-factory-{board}.bin"
        images[name] = merged
        variants[board] = {
            "file": name, "size": len(merged), "sha256": digest(merged),
            "components": [{"kind": kind, "offset": offset, "size": len(data), "sha256": digest(data)}
                           for kind, offset, data in (("flash_table", 0, ftab),
                               ("recovery_2", BOOT_OFFSET, recovery), ("application", APP_OFFSET, app))],
        }
    manifest = {
        "schema": 1, "product": "OpenWalkieTalkie", "firmware_version": version,
        "purpose": "factory_full_chip", "flash_size": FLASH_SIZE,
        "soc_address": FLASH_BASE, "spi_chip_offset": 0, "fill_byte": 255,
        "clears_unit_storage": True, "recovery_protocol": 2,
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
        "sdk_revision": "006d03293e0621d694335cd4031773279fa2f19a", "variants": variants,
    }
    output.mkdir(parents=True)
    for name, data in images.items():
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    readme = (ROOT / "docs/factory-flashing-zh.md").read_text(encoding="utf-8").replace("@VERSION@", version)
    (output / "README.md").write_text(readme, encoding="utf-8")
    shutil.copyfile(ROOT / "tools/verify-factory-package.py", output / "verify.py")
    files = sorted(p for p in output.rglob("*") if p.is_file())
    (output / "SHA256SUMS").write_text("".join(
        f"{digest(p.read_bytes())}  {p.relative_to(output).as_posix()}\n" for p in files), encoding="utf-8")
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for path in sorted(p for p in output.rglob("*") if p.is_file()):
            z.write(path, path.relative_to(output.parent))
    archive.with_name(archive.name + ".sha256").write_text(f"{digest(archive.read_bytes())}  {archive.name}\n")
    print(archive)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--firmware-dir", type=Path, required=True)
    parser.add_argument("--ftab", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    package(args.firmware_dir, args.ftab, args.version, args.output)
