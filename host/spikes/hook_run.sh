#!/bin/sh
# One S1/S2 run: start hookhold.py with HOLD_ARGS, drive claude in a pty with the
# remaining arguments (claude_pty.py options), stop the hook server, and print a
# merged timeline (hook POST/RESP + screen WATCH/SEND events, by wall clock).
#
#   host/spikes/hook_run.sh NAME OUTDIR "HOLD_ARGS" [claude_pty.py args...]
#   e.g. host/spikes/hook_run.sh run1c /tmp/spk "--hold 20 --permission allow" \
#          --cwd /tmp/spk/proj --settings /tmp/spk/settings-ask.json --seconds 70 \
#          --prompt 'Use the Bash tool to run: date' --watch 'Do\s*you\s*want\s*to\s*proceed' \
#          -- --permission-mode default
set -u
NAME=$1; OUT=$2; HOLD_ARGS=$3; shift 3
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
pkill -f "hookhold.py" 2>/dev/null; sleep 0.5
# shellcheck disable=SC2086
python3 "$HERE/hookhold.py" $HOLD_ARGS --log "$OUT/$NAME-hooks.log" > "$OUT/$NAME-hooks.out" 2>&1 &
HP=$!
sleep 1
python3 "$HERE/claude_pty.py" --log "$OUT/$NAME-pty.log" "$@" > "$OUT/$NAME-pty.out" 2>&1
kill "$HP" 2>/dev/null
python3 - "$OUT/$NAME-hooks.log" "$OUT/$NAME-pty.log" <<'EOF'
import re, sys
rows = []
for path in sys.argv[1:]:
    for line in open(path, errors="replace"):
        m = re.match(r"\[(\w+) t=\s*([\d.]+) (\d\d:\d\d:\d\d\.\d\d\d)\] (.*)", line.rstrip("\n"))
        if not m:
            continue
        src, _, wall, text = m.groups()
        if re.match(r"(POST|RESP|WATCH|SEND|listening|spawned|watch counts|done)", text):
            rows.append((wall, src, text))
rows.sort()
print("== timeline (wall clock) ==")
for wall, src, text in rows:
    print(f"{wall} {src:4s} {text[:220]}")
EOF
