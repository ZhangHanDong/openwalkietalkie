# SPDX-License-Identifier: MIT
"""Read-only integrity gate for the current hardware release (stdlib only)."""
from pathlib import Path
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parent
RELEASE = ROOT / "pcb-release-bodyv5-20260917-r3"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return json.loads(path.read_text())


def require(ok, message):
    if not ok:
        raise SystemExit(message)


def validate():
    require(sha(RELEASE / "manifest.json") ==
            "be34d5d505a4d2c5761e981d421488248f099ac47cd11c1c794d3610838e7498",
            "Release manifest changed; review and version the hardware handoff.")
    manifest = load(RELEASE / "manifest.json")
    for rel, info in manifest["files"].items():
        path = RELEASE / rel
        require(path.is_file(), f"Missing release asset: {rel}")
        require(path.stat().st_size == info["bytes"] and sha(path) == info["sha256"],
                f"Release asset differs: {rel}")
    package = RELEASE / "openmicrodial-bodyv5-r3.zip"
    require(sha(package) == "5d073781aa0a40c0eaf8885636fb1d073cbc2669bde5b3f60dc6458ab33c2d37",
            "Complete release ZIP differs.")
    with zipfile.ZipFile(package) as archive:
        require(archive.testzip() is None, "Release ZIP CRC check failed.")
        expected = set(manifest["files"]) | {"manifest.json"}
        require(len(archive.namelist()) == len(expected) and set(archive.namelist()) == expected,
                "Unexpected or missing release ZIP members.")
        for rel in expected:
            require(hashlib.sha256(archive.read(rel)).hexdigest() == sha(RELEASE / rel),
                    f"ZIP member differs: {rel}")
    for board, counts in [("main", (4, 100)), ("encoder", (2, 3))]:
        verification = RELEASE / board / "verification"
        drc = load(verification / "export-drc.json")
        require(not drc["violations"] and not drc["unconnected_items"], f"Recorded {board} DRC failed.")
        parity = load(verification / "source-parity.json")
        copper = load(verification / "independent-copper-audit.json")
        board_hash = sha(RELEASE / manifest["boards"][board]["file"])
        require(parity["passed"] and copper["passed"] and
                parity["routed_sha256"] == copper["sha256"] == board_hash and
                (copper["copper_layers"], copper["nets_checked"]) == counts,
                f"Recorded {board} verification does not match the board.")
    inputs = ROOT / "mechanical-source"
    for rel, info in load(inputs / "manifest.json")["files"].items():
        require(sha(inputs / rel) == info["sha256"], f"Mechanical input differs: {rel}")
    case = load(RELEASE / "verification/case-fit-audit.json")
    require(sha(inputs / "case/BodyV5-USB-face-battery-fit.step") == case["case_sha256"],
            "Case STEP is not the source used by the r3 fit audit.")
    print(f"BodyV5 r3 verified: {len(manifest['files'])} assets, matched ZIP, both boards and current mechanical inputs.")
    print("Recorded CAD/DRC evidence checked; this command does not rerun KiCad or certify physical fit.")


if __name__ == "__main__":
    validate()
