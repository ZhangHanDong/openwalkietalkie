#!/usr/bin/env bash
# Install `deckd run` as the per-user LaunchAgent ai.cohdl.openwalkietalkie.deckd
# (design D16). The agents of an install from before the rename from
# OpenMicroDial (com.conol.openmicrodial.deckd and the older .icon-sync) are
# stopped and removed so there is one HID writer (D15), and its data, logs and
# caches move to the new directories. The plist carries no log paths: deckd
# opens ~/Library/Logs/OpenWalkieTalkie/deckd.launchd.log itself (§9).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
label=ai.cohdl.openwalkietalkie.deckd
legacy_labels=(com.conol.openmicrodial.deckd com.conol.openmicrodial.icon-sync)
domain="gui/$(id -u)"
destination="$HOME/Library/Application Support/OpenWalkieTalkie"
plist="$HOME/Library/LaunchAgents/$label.plist"
logs="$HOME/Library/Logs/OpenWalkieTalkie"
bootout() {
    if launchctl print "$domain/$1" >/dev/null 2>&1; then
        launchctl bootout "$domain/$1"
    fi
}
remove_legacy_agents() {
    for old in "${legacy_labels[@]}"; do
        bootout "$old"
        rm -f "$HOME/Library/LaunchAgents/$old.plist"
    done
}
case "${1:-}" in
    --uninstall)
        bootout "$label"
        remove_legacy_agents
        rm -f "$plist" "$destination/deckd"
        echo 'OpenWalkieTalkie deckd stopped and removed. Logs and state retained.'
        exit 0 ;;
    '') ;;
    *) echo 'Usage: tools/install-host.sh [--uninstall]' >&2; exit 1 ;;
esac
"$root/tools/build-host.sh"
target=$(rustc -vV | sed -n 's/^host: //p')
binary="$root/target/host-app/$target/release/deckd"
bootout "$label"
remove_legacy_agents
for base in "Application Support" Logs Caches; do
    old="$HOME/Library/$base/OpenMicroDial"
    new="$HOME/Library/$base/OpenWalkieTalkie"
    if [ -d "$old" ] && [ ! -e "$new" ]; then
        mv "$old" "$new"
    fi
done
rm -f "$destination/openmicrodial-host"
mkdir -p "$destination" "$(dirname "$plist")" "$logs"
chmod 700 "$destination"
install -m 755 "$binary" "$destination/deckd"
python3 - "$plist" "$destination/deckd" "${CODEX_HOME:-$HOME/.codex}" "$label" <<'PY'
import plistlib, sys
plist, binary, codex, label = sys.argv[1:]
data = {
    'Label': label,
    'ProgramArguments': [binary, 'run'],
    'EnvironmentVariables': {'CODEX_HOME': codex},
    'RunAtLoad': True,
    # Week 6: `KeepAlive: True` restarts on **every** exit, which makes the
    # two exit codes `deckd` names meaningless — `service.stop` (0) came
    # straight back, and so did a second instance standing down (0, the
    # `EXIT_ALREADY_RUNNING` whose comment has always said "so it does not
    # loop"). `{SuccessfulExit: False}` is what those comments describe:
    # a clean 0 stays stopped, and everything else — `service.restart`'s
    # 75, a crash, a signal — is run again.
    'KeepAlive': {'SuccessfulExit': False},
    'ThrottleInterval': 10,
    'ProcessType': 'Background',
}
with open(plist, 'wb') as output:
    plistlib.dump(data, output)
PY
plutil -lint "$plist"
launchctl bootstrap "$domain" "$plist"
echo "Installed OpenWalkieTalkie deckd ($label). Logs: $logs/deckd.log*, $logs/deckd.launchd.log"
