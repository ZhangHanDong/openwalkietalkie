#!/bin/sh
# Managed by OpenWalkieTalkie: Codex Desktop shared-daemon launcher.
set -eu

fail() {
    printf '%s\n' "$1" >&2
    /usr/bin/osascript - "$1" <<'APPLESCRIPT' || true
on run argv
    display alert "Codex with OpenWalkieTalkie" message (item 1 of argv) as critical
end run
APPLESCRIPT
    exit 1
}

app=
for candidate in @DESKTOP_APPS@; do
    if [ -x "$candidate/Contents/Resources/codex-cli/bin/codex" ]; then
        app=$candidate
        break
    fi
done
[ -n "$app" ] || fail 'Install ChatGPT.app or Codex.app in Applications first. Its bundled Codex CLI is required.'
codex="$app/Contents/Resources/codex-cli/bin/codex"
for marker in CODEX_APP_SERVER_WS_URL ws+unix:; do
    /usr/bin/grep -a -q -F "$marker" "$app/Contents/Resources/app.asar" 2>/dev/null || fail 'This Codex Desktop build does not expose the local WebSocket connection mode. Update Codex Desktop before trying again.'
done

# An existing process will ignore the environment passed by open. Do not
# terminate it: the user may have an active turn or approval in that window.
running=$(/usr/bin/osascript -e 'application id "com.openai.codex" is running') || fail 'Could not check whether Codex is running.'
[ "$running" = false ] || fail 'Quit Codex Desktop / ChatGPT first, then open Codex with OpenWalkieTalkie again. Running sessions are not interrupted automatically.'

export CODEX_HOME=@CODEX_HOME@
# start reuses a running daemon; never restart/update it behind an active CLI.
"$codex" -c features.code_mode_host=true app-server daemon start || fail 'Codex could not start its local app-server daemon. Check Codex settings and authentication, then try again.'
# Desktop adds plugin overrides even with default settings, which makes its
# USE_LOCAL_DAEMON branch silently fall back to a private stdio server. Select
# its explicit WebSocket transport instead, over the same local Unix socket.
# Resolve Codex's socket symlink: its short target also avoids URL escaping of
# spaces and punctuation in CODEX_HOME. Fail rather than open an unshared UI.
socket="$CODEX_HOME/app-server-control/app-server-control.sock"
hops=0
while [ -L "$socket" ]; do
    [ "$hops" -lt 40 ] || fail 'The shared Codex daemon socket has a symlink loop.'
    target=$(/usr/bin/readlink "$socket") || fail 'Could not resolve the shared Codex daemon socket.'
    case "$target" in
        /*) socket=$target ;;
        *) socket="${socket%/*}/$target" ;;
    esac
    hops=$((hops + 1))
done
socket_dir=$(CDPATH= cd -P "${socket%/*}" && pwd -P) || fail 'Could not resolve the shared Codex daemon socket directory.'
socket="$socket_dir/${socket##*/}"
[ -S "$socket" ] || fail 'The shared Codex daemon socket is unavailable. Try opening this launcher again.'
case "$socket" in
    *[!A-Za-z0-9/._~-]*) fail 'The resolved Codex socket path contains characters unsupported by the desktop WebSocket URL.' ;;
esac
# localhost prevents Desktop from selecting its external-host proxy. ws+unix
# connects directly to the socket and never opens or uses a TCP listener.
/usr/bin/open -a "$app" \
    --env "CODEX_HOME=$CODEX_HOME" \
    --env CODEX_APP_SERVER_USE_LOCAL_DAEMON=1 \
    --env CODEX_APP_SERVER_FORCE_CLI=0 \
    --env CODEX_CLI_PATH= \
    --env "CODEX_APP_SERVER_WS_URL=ws+unix://localhost$socket:/rpc" || fail 'Could not open Codex Desktop. Check that ChatGPT.app or Codex.app is installed correctly.'
