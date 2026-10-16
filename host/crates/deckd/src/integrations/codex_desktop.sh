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

app=@USER_APP@
if [ ! -d "$app" ]; then app=/Applications/Codex.app; fi
codex="$app/Contents/Resources/codex-cli/bin/codex"
[ -x "$codex" ] || fail 'Install Codex.app in Applications first. Its bundled Codex CLI is required.'
/usr/bin/grep -a -q 'CODEX_APP_SERVER_USE_LOCAL_DAEMON' "$app/Contents/Resources/app.asar" 2>/dev/null || fail 'This Codex Desktop build does not expose the shared-daemon launch mode. Update Codex Desktop before trying again.'

# An existing process will ignore the environment passed by open. Do not
# terminate it: the user may have an active turn or approval in that window.
running=$(/usr/bin/osascript -e 'application id "com.openai.codex" is running') || fail 'Could not check whether Codex is running.'
[ "$running" = false ] || fail 'Quit Codex Desktop first, then open Codex with OpenWalkieTalkie again. Running sessions are not interrupted automatically.'
[ ! -e "$app/Contents/Resources/git/bin/git" ] || fail 'This Codex Desktop build uses bundled Git and does not support the shared-daemon launch mode. Update Codex Desktop before trying again.'

export CODEX_HOME=@CODEX_HOME@
# start reuses a running daemon; never restart/update it behind an active CLI.
"$codex" -c features.code_mode_host=true app-server daemon start || fail 'Codex could not start its local app-server daemon. Check Codex settings and authentication, then try again.'
/usr/bin/open -a "$app" \
    --env "CODEX_HOME=$CODEX_HOME" \
    --env CODEX_APP_SERVER_USE_LOCAL_DAEMON=1 \
    --env CODEX_APP_SERVER_FORCE_CLI=0 \
    --env CODEX_CLI_PATH= \
    --env CODEX_APP_SERVER_WS_URL= || fail 'Could not open Codex Desktop. Check that Codex.app is installed correctly.'
