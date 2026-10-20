#!/usr/bin/env bash
# The merge gate: every test crate and every firmware build, for every board
# (default = upright LilyGo dev board, --features hw-a = half-turned Rev A
# hardware, --features hw-b = Rev B). Prints one summary line per step and
# exits non-zero on any failure.
#
#   tools/test-all.sh                  tests, Embassy + thumb builds, C checks
#   OWT_GOLDEN_DIR=<dir> ...           also renders every <dir>/<scene>.ppm with
#                                      ui-preview and compares it byte for byte
#   tools/test-all.sh --native-link    also links the native firmware for every
#                                      board and scans both firmwares for
#                                      oversized stack frames; OWT_SDK_ENV names
#                                      the script that exports the SiFli SDK
#                                      environment
#
# Nothing but --native-link needs the SiFli SDK or the network.
set -uo pipefail

native_link=false
for arg in "$@"; do
    case "$arg" in
        --native-link) native_link=true ;;
        -h | --help) sed -n '2,15s/^# \{0,1\}//p' "$0"; exit 0 ;;
        *) echo "test-all: unknown argument '$arg'" >&2; exit 2 ;;
    esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
# cargo reads .cargo/config.toml from the cwd: the Embassy build needs the repo
# root, and host-side crates override its thumb target explicitly.
cd "$root" || exit 2
host=$(rustc -vV | sed -n 's/^host: //p')
logs="$root/target/test-all"
rm -rf "$logs"
mkdir -p "$logs"

# Test-count ratchet: a merge may add tests, never lose them.
min_host=${OWT_MIN_HOST_TESTS:-405}
min_native=${OWT_MIN_NATIVE_TESTS:-453}

steps=0
failures=0

report() { # status, label, detail
    printf '%-4s  %-34s %s\n' "$1" "$2" "$3"
    steps=$((steps + 1))
    [ "$1" = ok ] || failures=$((failures + 1))
}

slug() { printf '%s' "$1" | tr -c 'A-Za-z0-9' '-'; }

# run_tests <label> <minimum passed> <cargo test arguments...>
run_tests() {
    local label=$1 min=$2 log passed failed
    shift 2
    log="$logs/$(slug "$label").log"
    cargo test "$@" >"$log" 2>&1
    local status=$?
    passed=$(awk '/^test result:/ { n += $4 } END { print n + 0 }' "$log")
    failed=$(awk '/^test result:/ { n += $6 } END { print n + 0 }' "$log")
    if [ $status -ne 0 ] || [ "$failed" -ne 0 ]; then
        report FAIL "$label" "$passed passed, $failed failed; see ${log#"$root"/}"
        grep -E '^test .* FAILED$|^error' "$log" | head -20 | sed 's/^/        /'
    elif [ "$passed" -lt "$min" ]; then
        report FAIL "$label" "$passed passed, fewer than the ratchet of $min"
    else
        report ok "$label" "$passed passed"
    fi
}

# run_step <label> <success detail, or @file the command writes it to> <command...>
run_step() {
    local label=$1 detail=$2 log
    shift 2
    log="$logs/$(slug "$label").log"
    if "$@" >"$log" 2>&1; then
        case "$detail" in @*) detail=$(cat "${detail#@}") ;; esac
        report ok "$label" "$detail"
    else
        report FAIL "$label" "see ${log#"$root"/}"
        tail -n 15 "$log" | sed 's/^/        /'
    fi
}

c_check() { # source, binary name
    cc -std=c11 -Wall -Wextra -Werror "$root/$1" -o "$logs/$2" && "$logs/$2"
}

for board in lilygo hw-a hw-b; do
    features=()
    [ "$board" != lilygo ] && features=(--features "$board")
    run_tests "host tests [$board]" "$min_host" \
        --manifest-path tests/host/Cargo.toml --target "$host" \
        --target-dir target/host-tests --locked --offline ${features[@]+"${features[@]}"}
    run_tests "native core tests [$board]" "$min_native" \
        --manifest-path native/core/Cargo.toml --lib --target "$host" \
        --target-dir target/native-core-tests --locked --offline ${features[@]+"${features[@]}"}
done

# The three images share one ELF path: Rev B is built last, and
# tools/flash-usb.py checks an image's board against the deck's before writing.
for board in lilygo hw-a hw-b; do
    features=()
    [ "$board" != lilygo ] && features=(--features "$board")
    run_step "embassy release build [$board]" built \
        cargo build --release --locked --offline ${features[@]+"${features[@]}"}
    run_step "execution layout [$board]" verified \
        python3 tools/check-xip-image.py target/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw
done

for board in lilygo hw-a hw-b; do
    features=()
    [ "$board" != lilygo ] && features=(--features "$board")
    run_step "thumb staticlib [$board]" built \
        cargo build --manifest-path native/core/Cargo.toml --release --locked --offline \
        --target thumbv8m.main-none-eabihf --target-dir target/native-core \
        ${features[@]+"${features[@]}"}
done

run_step "C panel wake sequence" "1 passed" c_check tests/panel.c panel-test
run_step "C speaker PCM" "1 passed" c_check tests/speaker_pcm.c speaker-pcm-test
run_step "C recovery flash bounds" "1 passed" c_check tests/flash_layout.c flash-layout-test
run_step "C recovery progress and screen" "passed" \
    python3 -m unittest discover -s tools/tests -p test_recovery_ui.py

goldens() {
    local preview="$root/target/native-core-preview/$host/debug/examples/ui-preview"
    local total=0 same=0 different="" golden scene
    cargo build --manifest-path native/core/Cargo.toml --example ui-preview --target "$host" \
        --target-dir target/native-core-preview --locked --offline || return 1
    mkdir -p "$logs/golden"
    for golden in "$OWT_GOLDEN_DIR"/*.ppm; do
        [ -e "$golden" ] || continue
        scene=$(basename "$golden" .ppm)
        total=$((total + 1))
        if "$preview" "$logs/golden/$scene.ppm" "$scene" && cmp -s "$logs/golden/$scene.ppm" "$golden"; then
            same=$((same + 1))
        else
            different="$different $scene"
        fi
    done
    echo "$same of $total byte-identical" >"$logs/golden/summary"
    [ "$total" -gt 0 ] || { echo "no *.ppm scenes in $OWT_GOLDEN_DIR"; return 1; }
    [ -z "$different" ] || { echo "differs from golden:$different"; return 1; }
}
if [ -n "${OWT_GOLDEN_DIR:-}" ]; then
    run_step "ui-preview goldens" "@$logs/golden/summary" goldens
fi

# native_link <board> <rotation the linked core must report>
native_link() {
    # `set -e` is ignored under run_step's `if`, so every step checks itself.
    (
        # shellcheck disable=SC1090
        source "$OWT_SDK_ENV" || exit 1
        OWT_BOARD=$1 "$root/tools/build-native.sh" || exit 1
        # One archive path serves both boards: prove this ELF linked this core.
        code=$(arm-none-eabi-objdump -d --disassemble=owt_core_rotation \
            "$root/native/project/build_openwalkietalkie_hcpu/main.elf") || exit 1
        grep -Eq "mov[a-z.]*[[:space:]]+r0, #$2([^0-9]|\$)" <<<"$code" ||
            { echo "main.elf does not report rotation $2"; exit 1; }
        # The app thread has 24 KB: no Rust function (names with `::`, and the
        # owt_core_* ABI) may move a kilobyte-sized state through its frame.
        "$root/tools/check-stack-frames.py" --match '::|^owt_core_' \
            "$root/native/project/build_openwalkietalkie_hcpu/main.elf" || exit 1
    )
}

# The Embassy main task owns the UI state: it must stay in its static. (The
# SDK environment provides the objdump; the ELF is the board built last, the
# frames do not depend on the board.)
embassy_frames() {
    (
        # shellcheck disable=SC1090
        source "$OWT_SDK_ENV" || exit 1
        "$root/tools/check-stack-frames.py" \
            --match '____embassy_main_task|::app::|::ui::|::deck::' \
            "$root/target/thumbv8m.main-none-eabihf/release/openwalkietalkie-fw"
    )
}
if $native_link; then
    if [ -z "${OWT_SDK_ENV:-}" ] || [ ! -f "$OWT_SDK_ENV" ]; then
        report FAIL "native SDK link" "set OWT_SDK_ENV to the SDK environment script"
    else
        # LilyGo last, so the ELF left in the build directory suits the dev board.
        run_step "native SDK link [hw-b]" "linked, rotation 180" native_link hw-b 180
        run_step "native SDK link [hw-a]" "linked, rotation 180" native_link hw-a 180
        run_step "native SDK link [lilygo]" "linked, rotation 0" native_link lilygo 0
        run_step "embassy stack frames" "main task, app, ui, deck <= 2048 B" embassy_frames
    fi
fi

echo "test-all: $steps steps, $failures failed (logs in ${logs#"$root"/})"
[ "$failures" -eq 0 ]
