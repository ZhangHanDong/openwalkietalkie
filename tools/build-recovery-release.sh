#!/usr/bin/env bash
# Build only the SDK's SRAM recovery child, then check board, vectors and size.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
board=${1:?usage: build-recovery-release.sh hw-a|hw-b [OUTPUT]}
case "$board" in hw-a|hw-b) ;; *) echo 'choose hw-a or hw-b' >&2; exit 2 ;; esac
output=${2:-dist/firmware}
[[ "$output" = /* ]] || output="$root/$output"
: "${SIFLI_SDK:?Load the pinned SiFli SDK build environment first}"
[ "$(git -C "$SIFLI_SDK" rev-parse HEAD)" = 006d03293e0621d694335cd4031773279fa2f19a ] || {
    echo 'Recovery requires the pinned SiFli SDK revision.' >&2; exit 1;
}
cd native/project
OWT_BOARD="$board" OWT_RECOVERY_ONLY=1 scons --board=openwalkietalkie_hcpu --board_search_path=../boards -j8
cd "$root"
python3 tools/release.py recovery "$board" native/project/build_openwalkietalkie_hcpu/bootloader/output/bootloader.bin "$output"
