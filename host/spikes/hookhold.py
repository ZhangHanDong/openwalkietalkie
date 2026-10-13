#!/usr/bin/env python3
"""S1/S2 hook endpoint: receives Claude Code http hooks, HOLDS the held events, answers.

Every POST is logged with a wall-clock stamp (event, tool, session, tool_use_id).
`PermissionRequest` and `PreToolUse` are held for --hold seconds before the
response is written; every other event is answered `{}` at once (a
`Notification permission_prompt` arriving ~6 s into a hold is the tell that the
terminal dialog is on screen while the hook is held).

Responses after the hold:
  --permission empty  -> {}                                  (no decision)
  --permission allow  -> decision {behavior: allow}
  --permission deny   -> decision {behavior: deny, message}
  --ask empty         -> {}
  --ask answer        -> permissionDecision allow + updatedInput{questions: <echo>,
                         answers: {<question>: <--answer-label or the first option>}}

A client that hangs up during the hold shows as "client gone" in the log.

Usage:
  host/spikes/hookhold.py --port 47299 --hold 20 --permission empty --log /tmp/hooks.log
"""

import argparse
import json
import sys
import threading
import time
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ARGS = None
LOCK = threading.Lock()
T0 = time.monotonic()


def log(text):
    stamp = f"[hook t={time.monotonic() - T0:7.3f} {datetime.now().strftime('%H:%M:%S.%f')[:-3]}]"
    line = f"{stamp} {text}"
    with LOCK:
        print(line, flush=True)
        if ARGS.log:
            with open(ARGS.log, "a") as fh:
                fh.write(line + "\n")


def summary(payload):
    ev = payload.get("hook_event_name")
    bits = [f"event={ev}"]
    for key in ("tool_name", "session_id", "tool_use_id", "notification_type", "permission_mode"):
        if key in payload:
            bits.append(f"{key}={payload[key]}")
    ti = payload.get("tool_input")
    if isinstance(ti, dict):
        if "command" in ti:
            bits.append(f"command={ti['command']!r}")
        if "questions" in ti:
            bits.append("questions=" + json.dumps([q.get("question") for q in ti["questions"]]))
        if "answers" in ti:
            bits.append("answers=" + json.dumps(ti["answers"]))
    if "permission_suggestions" in payload:
        bits.append(f"suggestions={len(payload['permission_suggestions'])}")
    if "message" in payload:
        bits.append(f"message={payload['message']!r}")
    return " ".join(bits)


def answer_for(payload):
    ev = payload.get("hook_event_name")
    tool = payload.get("tool_name")
    if ev == "PermissionRequest":
        if ARGS.permission == "allow":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "allow"}}}
        if ARGS.permission == "deny":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "deny", "message": "Denied from the deck (spike)"}}}
        return {}
    if ev == "PreToolUse" and tool == "AskUserQuestion" and ARGS.ask == "answer":
        questions = payload.get("tool_input", {}).get("questions", [])
        answers = {}
        for q in questions:
            labels = [o.get("label") for o in q.get("options", []) if isinstance(o, dict)]
            pick = ARGS.answer_label if ARGS.answer_label in labels else (labels[0] if labels else "")
            answers[q.get("question", "")] = pick
        return {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow",
                                       "updatedInput": {"questions": questions, "answers": answers}}}
    if ev == "PreToolUse" and ARGS.pre_allow and tool != "AskUserQuestion":
        return {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow",
                                       "permissionDecisionReason": "spike pre-allow"}}
    return {}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            payload = json.loads(raw.decode() or "{}")
        except ValueError:
            payload = {"hook_event_name": "?", "raw": raw[:200].decode(errors="replace")}
        ev = payload.get("hook_event_name")
        tool = payload.get("tool_name")
        held = ev in ("PermissionRequest", "PreToolUse") and (tool != "AskUserQuestion" or ev == "PreToolUse")
        hold = ARGS.hold if held else 0.0
        if ev == "PermissionRequest" and tool == "AskUserQuestion":
            hold = 0.0  # design §5.1: this one is answered {} at once
        log(f"POST {self.path} {summary(payload)} hold={hold}s")
        if ARGS.dump:
            log("  payload " + json.dumps(payload)[:ARGS.dump])
        if hold:
            time.sleep(hold)
        body = json.dumps(answer_for(payload)).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            self.wfile.flush()
            log(f"RESP {ev}/{tool} after {hold}s: {body.decode()}")
        except (BrokenPipeError, ConnectionResetError, OSError) as err:
            log(f"RESP {ev}/{tool} after {hold}s: client gone ({err})")


def main():
    global ARGS
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=47299)
    ap.add_argument("--hold", type=float, default=20.0, help="seconds to hold PermissionRequest / PreToolUse")
    ap.add_argument("--permission", choices=["empty", "allow", "deny"], default="empty")
    ap.add_argument("--ask", choices=["empty", "answer"], default="empty")
    ap.add_argument("--answer-label", default=None, help="option label to pick for every question")
    ap.add_argument("--pre-allow", action="store_true", help="answer non-AskUserQuestion PreToolUse with allow")
    ap.add_argument("--dump", type=int, default=0, help="also log the first N chars of each payload")
    ap.add_argument("--log", default=None)
    ARGS = ap.parse_args()
    srv = ThreadingHTTPServer(("127.0.0.1", ARGS.port), Handler)
    srv.daemon_threads = True
    log(f"listening on 127.0.0.1:{ARGS.port} hold={ARGS.hold}s permission={ARGS.permission} ask={ARGS.ask}")
    try:
        srv.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
