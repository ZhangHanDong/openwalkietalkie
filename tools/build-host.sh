#!/usr/bin/env bash
# Build the release `deckd` binary for this Mac and print its path.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
target=$(rustc -vV | sed -n 's/^host: //p')
case "$target" in
    *-apple-darwin) ;;
    *) echo 'The host service currently supports macOS.' >&2; exit 1 ;;
esac
cargo build --manifest-path "$root/host/Cargo.toml" -p deckd --release --locked \
    --target "$target" --target-dir "$root/target/host-app" "$@"
echo "$root/target/host-app/$target/release/deckd"
