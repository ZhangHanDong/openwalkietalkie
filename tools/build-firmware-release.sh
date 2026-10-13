#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
board=${1:?usage: build-firmware-release.sh hw-a|hw-b [OUTPUT]}
case "$board" in hw-a|hw-b) ;; *) echo 'choose hw-a or hw-b' >&2; exit 2 ;; esac
cargo build --release --locked --features "$board" --target thumbv8m.main-none-eabihf --target-dir target/firmware-release
python3 tools/release.py image "$board" target/firmware-release/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw "${2:-dist/firmware}"
