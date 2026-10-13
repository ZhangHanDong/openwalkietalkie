#!/usr/bin/env bash
# Apple's ticket delivery can briefly fail after notarization is accepted.
# Retry the same archive; never treat acceptance alone as a stapled release.
set -euo pipefail
dmg=${1:?DMG path required}
for delay in 0 10 20 30 60 60; do
    if (( delay > 0 )); then
        printf 'Retrying Apple ticket attachment in %s seconds...\n' "$delay"
        sleep "$delay"
    fi
    if xcrun stapler staple "$dmg" && xcrun stapler validate "$dmg"; then
        exit 0
    fi
done
echo 'Apple ticket attachment or validation failed after six attempts.' >&2
exit 1
