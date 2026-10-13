# Week-1 spike probes

Throwaway-grade but reusable probes behind `docs/host-research/spikes-week1.md`.
Python 3.9 standard library only, plus `hid` (python hidapi, `pip install hidapi`)
for the device probe. Nothing here is part of the `deckd` build.

| File | Spike | What it does |
|---|---|---|
| `hookhold.py` | S1, S2 | Local HTTP hook endpoint that logs every Claude Code hook POST, holds `PermissionRequest` / `PreToolUse` for `--hold` seconds and answers `{}`, allow, deny or an `updatedInput.answers` block |
| `claude_pty.py` | S1, S2 | Runs an interactive `claude` in a pty (stdlib `pty`, no pexpect), accepts the trust dialog, types the prompt, times `--watch` regexes on the stripped screen, sends scheduled keys (`--at T:KEYS`), logs everything with wall-clock stamps |
| `hook_run.sh` | S1, S2 | One run = hook server + pty session + merged wall-clock timeline |
| `hid_probe.py` | S3 | One non-seizing hidapi client of the deck: hello heartbeat, listen-only, or bursts; counts input reports per channel and lines per `m`; times every write |

## S1/S2 — held hooks vs the terminal dialog

Hooks are installed for one session only through `claude --settings <file>` (the user's
`~/.claude/settings.json` is never edited; a temp `CLAUDE_CONFIG_DIR` cannot be used on macOS
because the OAuth credentials live in the keychain under the real config dir). The file
`settings-ask.json` used in the doc:

```json
{
  "allowedHttpHookUrls": ["http://127.0.0.1:47299/*"],
  "permissions": {"ask": ["Bash(date)", "Bash(date:*)"]},
  "hooks": {
    "PermissionRequest": [{"hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 130}]}],
    "PreToolUse": [{"matcher": "AskUserQuestion", "hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 130}]}],
    "Notification": [{"hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 2}]}],
    "PostToolUse": [{"hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 2}]}],
    "UserPromptSubmit": [{"hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 2}]}],
    "Stop": [{"hooks": [{"type": "http", "url": "http://127.0.0.1:47299/claude", "timeout": 2}]}]
  }
}
```

The `permissions.ask` rule forces the dialog for `date` whatever the user's own allow rules say.

```sh
mkdir -p /tmp/spk/proj
# hook held 20 s, then {} (no decision):
host/spikes/hook_run.sh run1 /tmp/spk "--hold 20 --permission empty" \
  --cwd /tmp/spk/proj --settings settings-ask.json --model sonnet --seconds 60 \
  --prompt 'Use the Bash tool to run the command: date   Then tell me its output in one line.' \
  --watch 'Do\s*you\s*want\s*to\s*proceed' --watch 'CST' -- --permission-mode default
# hook held 20 s, then allow / deny:            "--hold 20 --permission allow"   /   "... deny"
# operator answers the dialog 8 s into the hold: add   --at 12:'\r'
# AskUserQuestion answered from the hook:       "--hold 20 --ask answer --answer-label Green"
#   with --prompt 'Use the AskUserQuestion tool to ask me which colour I prefer (Red, Green, Blue) ...'
```

`hook_run.sh` prints a merged timeline; the full screen trace is in `<out>/<name>-pty.log`
(`DRAW` lines) and the raw bytes in `<name>-pty.log.raw`.

## S3 — non-seizing IOKit clients

```sh
# two probes for 60 s: A sends hello every second, B only listens; diff the summaries
host/spikes/hid_probe.py --name A --seconds 60 --hello 1 --log /tmp/A.log &
host/spikes/hid_probe.py --name B --seconds 60 --hello 0 --log /tmp/B.log
# burst: 500 hello lines as fast as the link acknowledges them (each answered by 6 reports)
host/spikes/hid_probe.py --name A --seconds 75 --hello 0 --burst 500 --burst-kind hello --burst-ms 0
# burst: silent title updates (the spinner-flood shape), paced at 8 ms if the link allowed it
host/spikes/hid_probe.py --name A --seconds 60 --hello 1 --burst 100 --burst-ms 8 --slow-write-ms 30
```

The probe opens the device non-exclusively by calling `hid_darwin_set_open_exclusive(0)` through
`ctypes` on the python module (the cython wrapper does not expose it; `hid.enumerate()` must run
first because `hid_init()` resets the flag to exclusive). If another process holds the device in
a way that makes the open fail, the probe waits and retries (`--retries`, `--retry-s`); it never
kills anything. The last log line is a JSON summary (`reports_in`, `per_channel`, `lines_by_m`,
`rtt_ms`, `write_ms`, `write_errors`, `device_present_after`, `per_second`).

Device-side confirmation of a run: `python3 tools/monitor.py /dev/cu.usbmodem… --seconds N`
(one process at a time) prints one `HID: write attr=14 length=64` per output report received.
