#!/usr/bin/env bash
# OWT_BOARD selects the hardware: unset/"lilygo" is the upright LilyGo
# T-Display SF32 dev board, "hw-a" the Rev A main board and "hw-b" the Rev B
# one (both with the panel turned 180; hw-b differs by three pins).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
board=${OWT_BOARD:-lilygo}
case "$board" in
    lilygo) features=() ;;
    hw-a | hw-b) features=(--features "$board") ;;
    *) echo "Unknown OWT_BOARD '$board' (expected lilygo, hw-a or hw-b)" >&2; exit 1 ;;
esac
: "${SIFLI_SDK:?Initialize SiFli SDK first: source /path/to/SiFli-SDK/export.sh}"
sdk_revision=006d03293e0621d694335cd4031773279fa2f19a
bt_revision=c3f204157265c275b5a9af8b20ef40663d9feab2
[ "$(git -C "$SIFLI_SDK" rev-parse HEAD)" = "$sdk_revision" ] || {
    echo "Expected SiFli-SDK revision $sdk_revision" >&2; exit 1;
}
[ "$(git -C "$SIFLI_SDK/middleware/bluetooth" rev-parse HEAD)" = "$bt_revision" ] || {
    echo "Initialize the SDK's pinned Bluetooth submodule first" >&2; exit 1;
}
echo "build-native: board=$board (rotation $([ "$board" != lilygo ] && echo 180 || echo 0) degrees)"
# Both boards share one staticlib path; cargo refreshes it for the chosen board
# and scons relinks because the archive's content changed.
cargo build --manifest-path "$root/native/core/Cargo.toml" --release --locked \
    --target thumbv8m.main-none-eabihf --target-dir "$root/target/native-core" \
    ${features[@]+"${features[@]}"}
cd "$root/native/project"
# The SDK merges <project>/<board>/proj.conf after proj.conf for the main
# project only; Rev A's configuration deltas (hw-a.conf, which Rev B shares:
# the microphone did not move) are installed there and removed again for the
# dev board, so every build shares one board.
overlay="$root/native/project/openwalkietalkie_hcpu/proj.conf"
if [ "$board" != lilygo ]; then
    mkdir -p "$(dirname "$overlay")"
    cp "$root/native/boards/openwalkietalkie/hcpu/hw-a.conf" "$overlay"
else
    rm -f "$overlay"
fi
scons --board=openwalkietalkie_hcpu --board_search_path=../boards -j8 "$@"
python3 "$root/tools/check-native-image.py" build_openwalkietalkie_hcpu/main.elf
