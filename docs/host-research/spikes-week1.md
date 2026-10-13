# Week-1 spikes: held Claude hooks, `updatedInput.answers`, non-seizing HID clients, hidapi exclusivity

Facts measured on 2026-09-18 for `docs/host-service.md` §16 Q1 and Q3 (and the §6.7 latency budget).
Probes live in `host/spikes/` (usage in `host/spikes/README.md`); raw logs quoted below were produced by them.

Environment: macOS 27.0 (Darwin 27.0.0), Claude Code 2.1.275 at the start (it auto-updated itself to
**2.1.276** during the first session; runs 1b onwards ran 2.1.276), codex 0.153.4 (not needed), python 3.9 with
python-hidapi 0.15.0, hidapi crate 2.6.7 (bundled hidapi C 0.15.0), deck firmware `0.1.0` over BLE HID
(VID 303A / PID 8360), the user's icon-sync launch agent (`openmicrodial-host`) running, ChatGPT.app running.
The user's `~/.claude/settings.json`, `~/.codex/*` and `~/.claude.json` hooks were not edited: hooks were injected per
session with `claude --settings <file>`; a temp `CLAUDE_CONFIG_DIR` is not usable for this on macOS (see S1 method).

## Verdicts

| Spike | One line | §16 |
|---|---|---|
| S1 | **Holding the `PermissionRequest` http hook does not suppress the terminal dialog**: it is drawn 13–20 ms after the hook POST, `Notification permission_prompt` fires 6 s into the hold, and a decision returned after 20 s (allow or deny) is still honoured within 50–500 ms. `{}` after the hold changes nothing (dialog stays). A local answer during the hold wins immediately and does **not** close the hook connection; a hook timeout closes the connection and leaves the dialog live | Q1 first half: **no** → "first answer wins" |
| S2 | **`PreToolUse` allow + `updatedInput.answers` answers `AskUserQuestion` interactively with no dialog**, and while that hook is held nothing is drawn (spinner "running PreToolUse hook"). Releasing with `{}` fires `PermissionRequest{tool_name:"AskUserQuestion"}` 3 ms later and the question dialog 10 ms after that | Q1 second half: **yes** |
| S3 | **IOKit delivers every input report to every non-seizing client**: 2 and 3 clients counted byte-identical totals over 60 s (363/364 reports each; ch5 360, ch2 3, ch3 0–1) and over a 3000-report reply storm (3005 each). No starvation, no drop. Output side: a host write is a synchronous `IOHIDDeviceSetReport` that returns only when the BLE write is acknowledged — **≈360 ms per report while the deck is idle, ≈60 ms while it is transmitting**; a "500 reports at 8 ms" burst is therefore impossible from macOS (500 silent reports took 179.7 s, 500 hello lines 31.4 s) and neither burst dropped the link | Q3: **yes, every report to every client**; the 8 ms pacer is moot on macOS, the ack latency is the pacer |
| S4 | hidapi 2.6.7 on macOS: `hid_init()` sets exclusive (seize) for backward compatibility; `HidApi::set_open_exclusive(false)` flips a process-global flag consumed by every later `open`/`open_path` (`kIOHIDOptionsTypeNone` instead of `kIOHIDOptionsTypeSeizeDevice`); the `macos-shared-device` cargo feature does the same in `HidApi::new()`. Today's host (`host/src/main.rs:144` before phase A) and `deckd::hid::open_device` (used by `iconsync::run_standalone` too) call it right before opening; `deckd doctor` only enumerates | supports §6.6 |

## S1 — held `PermissionRequest` http hook vs the terminal dialog

### Method

* `host/spikes/hookhold.py --port 47299 --hold N --permission empty|allow|deny`: a `ThreadingHTTPServer` that logs every
  hook POST with a wall clock, sleeps N s on `PermissionRequest` / `PreToolUse`, then writes the configured body; every
  other event is answered `{}` at once. A client that hangs up during the hold shows as "client gone (Broken pipe)".
* `host/spikes/claude_pty.py`: forks `claude --permission-mode default --settings <file> --model sonnet` on a 140×45 pty
  (stdlib `pty`; pexpect is not installed), accepts the workspace-trust dialog, types the prompt, strips ANSI/OSC and
  logs everything drawn with wall-clock stamps, times `--watch` regexes, and can send keys at a scheduled offset.
* Hooks for the session come from `--settings` (file in `host/spikes/README.md`): http `PermissionRequest` (timeout
  130; 30 in run 1f), http `PreToolUse` matcher `AskUserQuestion` (130), http `Notification`/`PostToolUse`/
  `UserPromptSubmit`/`Stop` (timeout 2), `allowedHttpHookUrls: ["http://127.0.0.1:47299/*"]`, and
  `permissions.ask: ["Bash(date)", "Bash(date:*)"]` so the prompt is forced whatever the user's allow rules say
  (run 1 without the ask rule ran `date` with no `PermissionRequest` at all).
* Prompt: `Use the Bash tool to run the command: date   Then tell me its output in one line. Do not ask me anything.`
* Why the real config dir: with `CLAUDE_CONFIG_DIR=<tmp>` the CLI reports `Not logged in · Please run /login`
  (credentials live in the keychain under the real config dir), so `--settings` was the only way to add hooks without
  touching `~/.claude/settings.json`. Side effects on the user's state: the trust dialog for the scratch cwd was accepted
  (a `projects` entry in `~/.claude.json`); transcripts were **not** written — the child printed "Transcript saving is
  off — inherited CLAUDE_CODE_CHILD_SESSION marker" because the spike ran from inside a Claude Code session.

### Runs and raw observations (wall clock, from the merged timelines)

| Run | Hook | Timeline |
|---|---|---|
| 1b | hold 20 s → `{}` (2.1.276) | prompt 22:58:04.34 · `PermissionRequest{Bash,"date"}` POST 22:58:07.766 · **dialog drawn 22:58:07.779** ("Bash command · date · Permission rule Bash(date) requires confirmation … Do you want to proceed? ❯ 1. Yes 2. No · Esc to cancel · Tab to amend") · `Notification permission_prompt` 22:58:13.781 (+6.0 s) · `{}` written 22:58:27.773 · screen unchanged afterwards (only the pulsing `⏺`), dialog still up when the session was killed 80 s later |
| 1c | hold 20 s → allow | POST 23:08:00.368 · dialog 23:08:00.385 (+17 ms) · `permission_prompt` +6.0 s · allow written 23:08:20.372 · **"⎿ Allowed by PermissionRequest hook" 23:08:20.845**, `PostToolUse` 23:08:20.828, `Stop` 23:08:23.886, output `Fri Sep 18 23:08:20 CST 2026` on screen 23:08:23.894 |
| 1d | hold 20 s → deny `{message:"Denied from the deck (spike)"}` | POST 23:12:26.166 · dialog +5 ms · `permission_prompt` +6.0 s · deny written 23:12:46.170 · **"⎿ Denied by PermissionRequest hook" 23:12:46.213** (+43 ms) · Claude: "The date command was denied by permission settings" · `Stop` 23:12:48.534 |
| 1e | hold 40 s → allow; operator presses Enter (Yes) at +12 s | POST 23:13:20.982 · dialog +22 ms · `permission_prompt` +6.0 s · **Enter sent 23:13:29.515 → `PostToolUse` 23:13:29.988** (+0.47 s), output on screen 23:13:32.977, `Stop` 23:13:32.970 · the hook's allow was written at 23:14:00.985 **without error** (Claude kept the connection open for the full 40 s and discarded the answer; nothing on screen) |
| 1f | hook `timeout: 30`, hold 45 s → allow | POST 23:14:24.277 · dialog +20 ms · `permission_prompt` +6.0 s · at 45 s the response write failed: **client gone ([Errno 32] Broken pipe)** — Claude closed the socket at its 30 s timeout · no visible message on the stripped screen · the dialog stayed live: the driver's teardown `\r` at 23:15:36 selected "Yes" and `PostToolUse{Bash}` fired 23:15:37.121 |

Timing of the dialog relative to the hook: 13, 17, 5, 22 and 20 ms after the POST arrived, i.e. Claude Code draws the
dialog and starts the `PermissionRequest` hooks together; the hook does not gate the dialog. `permission_prompt`
arrived 6.00–6.02 s after the POST in every run (the documented "about six seconds").

### What this means for §5.1

* The held hook is still worth it: a deck decision after 20 s is applied within ≈50 ms (deny) / ≈450 ms (allow, the
  tool then runs), the dialog disappears and Claude logs "Allowed/Denied by PermissionRequest hook".
* "First answer wins" is the real contract: a local keypress ends the request at once. The daemon learns it only from
  `PostToolUse` (allow) or the next `Stop`/`PermissionDenied`, **not** from the hold's connection — Claude keeps the
  http request open until the hook timeout, so `HoldGone` never fires for a locally answered card. The card must be
  cleared from those events, and the `hold_s` timer then only bounds a card nobody answered anywhere.
* `{}` after a hold is harmless (the dialog simply stays); the degrade-to-`⌨ Focus terminal` re-push is right.
* A hook timeout (Claude closing the socket) leaves the dialog live, so the daemon's `HoldGone` on a reset connection
  is a valid "release" and the timeout must stay above `hold_s` as designed (130 > 120).

## S2 — `PreToolUse` allow + `updatedInput.answers` for `AskUserQuestion`

Same driver; prompt: `Use the AskUserQuestion tool to ask me one question: "Which colour do you prefer?" with exactly
the options Red, Green and Blue. After I answer, reply with exactly the colour I chose and nothing else.`
`hookhold.py --hold 20 --ask answer --answer-label Green` returns
`{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"questions":<echo>,"answers":{"Which colour do you prefer?":"Green"}}}}`.

| Run | Timeline |
|---|---|
| 2a hold 20 s → answers | `PreToolUse{AskUserQuestion}` POST 23:15:49.870 (tool_input `questions[0] = {question, header:"Colour", options:[Red,Green,Blue], multiSelect:false}`) · **nothing drawn during the hold except "Proofing… running PreToolUse hook"** (no question, no options) · response 23:16:09.876 · `PostToolUse{AskUserQuestion}` 23:16:09.888 with `answers` present · screen 23:16:09.925 **"⏺ User answered Claude's questions: ⎿ · Which colour do you prefer? → Green"** · Claude's reply `Green` 23:16:12.119 · `Stop` 23:16:12.111. No `PermissionRequest` fired |
| 2b hold 20 s → `{}` | POST 23:16:51.300 · nothing drawn during the hold · `{}` 23:17:11.305 · **`PermissionRequest{tool_name:"AskUserQuestion"}` 23:17:11.308** (answered `{}` at once, as §5.1 prescribes) · **question dialog drawn 23:17:11.315** ("☐ Colour · Which colour do you prefer? ❯ 1. Red 2. Green 3. Blue 4. Type something. 5. Chat about this · Enter to select · ↑/↓ to navigate · Esc to cancel") · `permission_prompt` 23:17:17.318 (+6.0 s) · the dialog stayed until the teardown Enter answered it (`PostToolUse` 23:17:43 with `answers`) |

So, unlike a permission, an AskUserQuestion **is** fully suppressed while its `PreToolUse` hook is held (the dialog is
only built after the hook chain returns), the programmatic answer works interactively, and the
`PermissionRequest{AskUserQuestion}`-after-release row of §5.1 is confirmed (3 ms after the release, before the dialog).
Open point for §5.1's sequential question groups: `hold_s` bounds the whole group with nothing on the terminal in the
meantime, so a snoozed group should be released quickly rather than at the 120 s budget.

## S3 — IOKit input-report delivery to several non-seizing clients, and bursts

### Method

`host/spikes/hid_probe.py` opens the deck non-exclusively (python-hidapi exposes no such call; the probe runs
`hid.enumerate()` — which calls `hid_init()`, which resets the flag — then `hid_darwin_set_open_exclusive(0)` through
`ctypes` on the module's own symbol, then `hid.device().open()`), polls `read(64)` non-blocking at 1 ms, counts input
reports per channel byte, reassembles channel-5 lines with a copy of `deck-sim.py`'s assembler and counts them by `m`,
times every `write` and every hello→reply, and prints a JSON summary. Probe A sends `{"m":"hello","v":1,"sid":314159,
"hm":…}` every second, probe B only listens. The icon-sync launch agent was stopped with `launchctl bootout` only for
the two-client case and restored with `launchctl bootstrap` right after (verified `state = running`, new pid 47105).
The ChatGPT app was running throughout (it is the channel-2 client; 3 ch-2 reports/min were seen by everyone).
Device-side: `tools/monitor.py` on the USB console printed one `HID: write attr=14 length=64` per report received and
no error during a 100-report burst.

### Delivery (60 s each)

| Case | Clients | A `reports_in` (ch5/ch2/ch3) | B `reports_in` (ch5/ch2/ch3) | hello sent/replies | hello RTT ms min/med/max |
|---|---|---|---|---|---|
| two clients (icon-sync stopped) | A + B (+ ChatGPT.app) | 363 (360/3/0) | 363 (360/3/0) | 60/60 | 30.7 / 46.0 / 61.2 |
| three clients (icon-sync running) | A + B + icon-sync (+ ChatGPT.app) | 364 (360/3/1) | 364 (360/3/1) | 60/60 | 43.8 / 45.8 / 61.2 |
| reply storm: 500 hellos back-to-back | A + B + icon-sync | 3005 (3000/3/2) | 3005 (3000/3/2) | 500/500 in 31.4 s | 50.7 / 60.0 / 104.7 |
| silent storm: 500 title lines back-to-back | A + B + icon-sync | 15 (0/11/4) | 15 (0/11/4) | — | — |

Every input report (six per hello reply, plus the other channels' traffic) reached every client, in every case, with
zero `bad_lines`, `read_errors` or `write_errors`; the per-second rows match one to one. The link stayed up through
both storms and answered the next hello normally (RTT 45 ms). No starvation of the icon-sync or ChatGPT clients was
observable from the outside (their channel-2/3 reports kept flowing during the storms).

### Output side: write latency is the real pacer

hidapi's macOS `hid_write` is `IOHIDDeviceSetReport` with no timeout (sampled stack of a "stuck" probe:
`__pyx_pw_3hid_6device_7write → set_report → IOHIDDeviceSetReport → IOConnectCallMethod → mach_msg2_trap`); it returns
when the BLE write is acknowledged by the peripheral, which happens at the peripheral's next attended connection event:

| Condition | write() latency (median / max, n) |
|---|---|
| deck idle (silent `agent` title lines, 1 probe alone, pace 0) | **359.9 ms** / 720 ms, n = 502 (500 lines in 179.7 s) |
| deck idle, 3 clients, 100 title lines "at 8 ms" | 359.8 ms / 392 ms, n = 107 (34.6 s) |
| deck transmitting (hello lines, each answered by 6 reports) | **59.8 ms** / 361 ms, n = 502 (500 lines in 31.4 s) |
| single hello while idle | 100–360 ms (first write of a run: 200–265 ms) |

The 360 ms idle figure is consistent with a slave-latency of ≈23 on the 15 ms interval the Mac negotiates
(`docs/idle-power-audit.md:486`; the firmware logs the negotiated parameters only at connect time, which was not
observed during the spike); 60 ms while transmitting is two intervals plus bluetoothd. The client count does not change
it (single-client and three-client runs are identical).

Consequences for §6.2 / §6.7 / §14:

* The "≤1 report / 8 ms" rule is satisfied by construction on macOS; `pace_ms` never triggers. The firmware's 64-message
  queue cannot be overflowed from this host at all (the ack gates every report), so the overflow back-off signature of
  §6.2 is unreachable on macOS (still relevant on Linux hidraw, where writes are not acked).
* `hid_write` blocks the hid thread for up to 360 ms (720 ms observed once): the 2 ms read poll and the
  `channel → focus{ack}` ≤30 ms budget cannot hold while a write is in flight. Input is not lost (hidapi's own read
  thread queues reports, which is why the counts above are complete), only delayed. Writes should live on their own
  thread (or `IOHIDDeviceSetReportWithCallback`, which hidapi does not expose) so reads and the `focus{ack}` reply are
  not queued behind a title update.
* Budget corrections: full sync of ≈80 reports ≈ 5–8 s when the deck is replying (`hello` traffic keeps it at 60 ms),
  up to ≈29 s if it stays silent; "hook → toast ≤100 ms" becomes ≈60–360 ms + the reports of the line; the 30-min title
  flood soak of §14 tops out at ≈2.8 lines/s idle or ≈16/s active, whatever the flood asks for. The keep-latest
  coalescing in the outbox matters more than pacing: at 4 spinner updates/s/slot the link can carry one slot's titles
  when idle.
* A device-side `err` was never provoked; the 500 silent `agent` updates for slot 3 were accepted (the device had an
  `agent` line for the slot first).

## S4 — hidapi 2.6.7 `set_open_exclusive(false)` on macOS 27 (source read, `~/.cargo/registry`)

* `etc/hidapi/mac/hid.c:469` — `hid_init()` calls `hid_darwin_set_open_exclusive(1)` ("Backward compatibility"), so the
  default after any `HidApi::new()` is `kIOHIDOptionsTypeSeizeDevice`.
* `hid.c:1501-1504` — `hid_darwin_set_open_exclusive(0)` sets the process-global `device_open_options =
  kIOHIDOptionsTypeNone`; `hid.c:155` copies it into each new `hid_device` at open time; `hid.c:1055` passes it to
  `IOHIDDeviceOpen`. It is therefore a global switch that affects **every later open in the process** (any thread), not
  a per-device option, and it must be set before `open`/`open_path` of the device in question.
* `src/macos.rs:14-21` — `HidApi::set_open_exclusive(&self, bool)` / `get_open_exclusive()` are thin wrappers;
  `HidDevice::is_open_exclusive()` reads the per-device copy. `src/lib.rs:181` — the `macos-shared-device` feature calls
  `hid_darwin_set_open_exclusive(0)` once inside `HidApi::new()`; the workspace does not enable it (`hidapi = "=2.6.7"`).
* Today's host (`host/src/main.rs:142-144` at commit c0e4d1e) does `HidApi::new()` then `api.set_open_exclusive(false)`
  before opening — this is the production behaviour §6.6 relies on, and `deckd::hid::open_device` (`hid.rs:82`) keeps it;
  `deckd::iconsync::run_standalone` opens through that same function (`iconsync/mod.rs:113`), so every `deckd` open is
  non-seizing. Because the flag is process-global, one `HidApi` per process that never flips it back is the safe shape;
  a second `HidApi::new()` in the same process does not reset it (`hid_init()` is a no-op once the manager exists).
* Non-exclusive semantics observed: a seized open by another process would make `IOHIDDeviceOpen` fail with
  `kIOReturnExclusiveAccess` for us (not tested against the real ChatGPT app, which was running the whole time and did
  not block any of our non-seizing opens); several non-seizing opens coexist and each gets every input report (S3).
  The python probe confirms the flag through `hid_darwin_get_open_exclusive()` after setting it.

## Files

* `host/spikes/hookhold.py`, `host/spikes/claude_pty.py`, `host/spikes/hook_run.sh`, `host/spikes/hid_probe.py`,
  `host/spikes/README.md` (usage and the settings file).
* Raw logs of the runs quoted above are in the session scratchpad (`spk/s1`, `spk/s2`, `spk/s3`), not committed.
