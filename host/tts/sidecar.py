#!/usr/bin/env python3
"""The daemon's local text-to-speech sidecar: an MLX model (Qwen3-TTS by
default) kept loaded in a Python process, spoken to over stdin/stdout.

deckd starts one of these when `[voice.tts] engine = "mlx"`, from the
virtual environment `deckd tts install` creates, and keeps it for the life
of the daemon so the model loads once. Nothing here touches the network
after the model is cached: the model runs on the Mac's GPU through MLX.

Protocol (`deckd::audio::sidecar` is the other side):

  stdin, one JSON object per line:
    {"op":"say","id":1,"text":"…","voice":"Ryan","instruct":null,"lang":"auto"}
    {"op":"cancel","id":1}
    {"op":"quit"}
  stdout, binary frames: one tag byte, a little-endian u32 length, payload:
    R  {"model":…,"rate":24000,"channels":1,"voices":[…],"load_ms":…}   once, when the model is loaded
    H  {"id":1}                                                        the job's audio starts
    P  raw PCM, 16-bit little-endian mono at `rate`                   zero or more
    E  {"id":1,"ms":…,"samples":…}                                    the job is over
    X  {"id":1,"error":"…"}                                           instead of E (or of H)
  stderr: log lines.

`--warm` loads the model (downloading it on first use), prints the R
frame's JSON on stdout as text, and exits: what `deckd tts install` runs
so the first announcement does not wait on a download.
"""
import argparse
import json
import os
import struct
import sys
import threading
import time

PROTOCOL = 1
DEFAULT_MODEL = "mlx-community/Qwen3-TTS-12Hz-1.7B-CustomVoice-8bit"
# The speakers the Qwen3-TTS Base/CustomVoice checkpoints ship with.
QWEN3_SPEAKERS = ["Ryan", "Aiden", "Vivian", "Serena", "Uncle_Fu", "Dylan", "Eric"]

# The frame stream is the real stdout, taken before anything else can
# print to it: model loaders and progress bars write to fd 1 freely, and one
# stray line there would put the daemon's frame reader out of sync. Fd 1
# itself is pointed at stderr, so those lines become log lines.
FRAMES = os.fdopen(os.dup(1), "wb", buffering=0)
os.dup2(2, 1)
sys.stdout = sys.stderr


def log(message):
    sys.stderr.write(f"sidecar: {message}\n")
    sys.stderr.flush()


def frame(tag, payload):
    if isinstance(tag, str):
        tag = tag.encode()
    FRAMES.write(tag)
    FRAMES.write(struct.pack("<I", len(payload)))
    FRAMES.write(payload)
    FRAMES.flush()


def frame_json(tag, obj):
    frame(tag, json.dumps(obj, separators=(",", ":")).encode())


def load(model_id):
    os.environ.setdefault("HF_HUB_DISABLE_PROGRESS_BARS", "1")
    os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
    from mlx_audio.tts.utils import load_model  # noqa: PLC0415

    started = time.monotonic()
    model = load_model(model_id)
    load_ms = int((time.monotonic() - started) * 1000)
    rate = int(getattr(model, "sample_rate", 24000) or 24000)
    voices = list(QWEN3_SPEAKERS) if "qwen3-tts" in model_id.lower() else []
    return model, {
        "protocol": PROTOCOL,
        "model": model_id,
        "rate": rate,
        "channels": 1,
        "voices": voices,
        "load_ms": load_ms,
    }


def to_pcm16(audio):
    import numpy as np  # noqa: PLC0415

    samples = np.asarray(audio, dtype=np.float32).reshape(-1)
    return (np.clip(samples, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()


class Jobs:
    """What stdin asked for, read on its own thread so a cancel can land
    while the model is busy."""

    def __init__(self):
        self.lock = threading.Lock()
        self.ready = threading.Condition(self.lock)
        self.queue = []
        self.cancelled = set()
        self.quit = False

    def reader(self):
        for line in sys.stdin.buffer:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except json.JSONDecodeError:
                log(f"unreadable request: {line[:80]!r}")
                continue
            op = req.get("op")
            with self.lock:
                if op == "say" and isinstance(req.get("id"), int):
                    self.queue.append(req)
                elif op == "cancel" and isinstance(req.get("id"), int):
                    self.cancelled.add(req["id"])
                    self.queue = [j for j in self.queue if j["id"] != req["id"]]
                elif op == "quit":
                    self.quit = True
                self.ready.notify_all()
        with self.lock:
            self.quit = True
            self.ready.notify_all()

    def next(self):
        with self.lock:
            while not self.queue and not self.quit:
                self.ready.wait()
            if self.queue:
                return self.queue.pop(0)
            return None

    def is_cancelled(self, job_id):
        with self.lock:
            return job_id in self.cancelled


def say(model, info, jobs, req):
    job_id = req["id"]
    text = (req.get("text") or "").strip()
    if not text:
        frame_json("X", {"id": job_id, "error": "empty text"})
        return
    kwargs = {
        "text": text,
        "lang_code": req.get("lang") or "auto",
        "stream": True,
        "streaming_interval": float(req.get("interval") or 0.32),
    }
    if req.get("voice"):
        kwargs["voice"] = req["voice"]
    if req.get("instruct"):
        kwargs["instruct"] = req["instruct"]
    if req.get("speed"):
        kwargs["speed"] = float(req["speed"])
    started = time.monotonic()
    samples = 0
    sent_header = False
    try:
        for result in model.generate(**kwargs):
            if jobs.is_cancelled(job_id):
                break
            audio = getattr(result, "audio", None)
            if audio is None:
                continue
            pcm = to_pcm16(audio)
            if not pcm:
                continue
            if not sent_header:
                frame_json("H", {"id": job_id, "first_ms": int((time.monotonic() - started) * 1000)})
                sent_header = True
            frame(b"P", pcm)
            samples += len(pcm) // 2
    except Exception as e:  # noqa: BLE001 — anything the model raised is the job's error, not the sidecar's end
        log(f"job {job_id} failed: {e!r}")
        frame_json("X", {"id": job_id, "error": str(e)[:300]})
        return
    if not sent_header:
        frame_json("H", {"id": job_id, "first_ms": int((time.monotonic() - started) * 1000)})
    frame_json(
        "E",
        {
            "id": job_id,
            "ms": int((time.monotonic() - started) * 1000),
            "samples": samples,
            "cancelled": jobs.is_cancelled(job_id),
        },
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--warm", action="store_true", help="load the model (downloading it if needed), print its info, exit")
    args = parser.parse_args()

    try:
        model, info = load(args.model)
    except Exception as e:  # noqa: BLE001
        log(f"cannot load {args.model}: {e!r}")
        if args.warm:
            FRAMES.write((json.dumps({"error": str(e)[:300], "model": args.model}) + "\n").encode())
            return 1
        frame_json("X", {"id": 0, "error": f"cannot load {args.model}: {str(e)[:200]}"})
        return 1
    if args.warm:
        FRAMES.write((json.dumps(info) + "\n").encode())
        return 0
    # The first generation after a load compiles the model's graphs and
    # takes seconds; paying that here, before the daemon is told we are
    # ready, keeps it off the first announcement.
    started = time.monotonic()
    try:
        kwargs = {"text": "Ready.", "stream": True, "streaming_interval": 0.32}
        if info["voices"]:
            kwargs["voice"] = info["voices"][0]
        for _ in model.generate(**kwargs):
            pass
        info["warm_ms"] = int((time.monotonic() - started) * 1000)
    except Exception as e:  # noqa: BLE001 — a failed warm-up is a log line; the first real utterance will say more
        log(f"warm-up failed: {e!r}")
    frame_json("R", info)
    log(f"ready: {info['model']} at {info['rate']} Hz, loaded in {info['load_ms']} ms, warmed in {info.get('warm_ms', '?')} ms")

    jobs = Jobs()
    threading.Thread(target=jobs.reader, name="stdin", daemon=True).start()
    while True:
        req = jobs.next()
        if req is None:
            break
        if jobs.is_cancelled(req["id"]):
            frame_json("E", {"id": req["id"], "ms": 0, "samples": 0, "cancelled": True})
            continue
        say(model, info, jobs, req)
    log("quit")
    return 0


if __name__ == "__main__":
    code = main()
    # Straight out: the interpreter's teardown of the GPU runtime with the
    # stdin thread still blocked ends in an abort, which is noise.
    sys.stderr.flush()
    os._exit(code)
