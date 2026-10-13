#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
design_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python3 "$design_dir/validate_release.py"
cohdl_bin=${COHDL_BIN:-cohdl}
expected_version=$(cat "$design_dir/.cohdl-version")
actual_version=$("$cohdl_bin" --version)
case "$actual_version" in
    "cohdl $expected_version"|"cohdl $expected_version "*) ;;
    *) printf 'Expected CoHDL %s, got %s. Set COHDL_BIN to the matching executable.\n' "$expected_version" "$actual_version" >&2; exit 1 ;;
esac
release_dir="$design_dir/pcb-release-bodyv5-20260917-r3"
"$cohdl_bin" check "$release_dir/source/main" --json
"$cohdl_bin" check "$release_dir/source/encoder" --json
