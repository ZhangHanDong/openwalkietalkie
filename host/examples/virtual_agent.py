#!/usr/bin/env python3
"""A virtual agent over `POST /v1/rpc` (design §14): register, ask, wait.

    python3 virtual_agent.py --port $(cat ~/.../port) --token dk_... --name bot

Registers a `poll`/`poll` agent, sets a `q` card with Yes/No, then parks in
`agent.wait` (heartbeating every 10 s) until the deck or an operator
answers. Prints the answer as JSON and exits 0; exits 1 on timeout or error.
"""
import argparse, json, sys, time, urllib.request

p = argparse.ArgumentParser()
p.add_argument("--port", type=int, required=True)
p.add_argument("--token", required=True)
p.add_argument("--name", default="virtual")
p.add_argument("--text", default="Deploy to staging?")
a = p.parse_args()
URL, HEAD = f"http://127.0.0.1:{a.port}/v1/rpc", {
    "Authorization": f"Bearer {a.token}", "Content-Type": "application/json"}


def call(method, params, _id=[0]):
    _id[0] += 1
    params = dict(params, client={"name": a.name, "version": "1"})
    body = json.dumps({"jsonrpc": "2.0", "id": _id[0], "method": method, "params": params})
    req = urllib.request.Request(URL, body.encode(), HEAD, method="POST")
    with urllib.request.urlopen(req, timeout=90) as r:
        answer = json.load(r)
    if "error" in answer:
        raise SystemExit(f"{method}: {json.dumps(answer['error'])}")
    return answer["result"]


call("session.hello", {})
agent = call("agent.register", {"kind": "other", "name": a.name, "key": f"virtual:{a.name}",
             "lease_ttl_s": 30, "caps": {"answer": "poll", "order": "poll", "focus": False}})["agent"]
call("agent.card.set", {"agent": agent, "type": "q", "text": a.text, "free_text": False,
     "deliver": "poll", "choices": [{"label": "Yes"}, {"label": "No"}],
     "origin": {"kind": "custom"}})
print(json.dumps({"agent": agent, "waiting": True}), flush=True)
deadline, answer = time.monotonic() + 60, None
while answer is None and time.monotonic() < deadline:
    got = call("agent.wait", {"agent": agent, "timeout_s": 10})
    if got.get("kind") in ("answer", "order"):
        answer = got
    else:
        call("agent.heartbeat", {"agents": [agent]})
print(json.dumps(answer), flush=True)
call("agent.release", {"agent": agent, "reason": "done"})
sys.exit(0 if answer else 1)
