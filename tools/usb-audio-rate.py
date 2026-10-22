#!/usr/bin/env python3
"""Play prepared, EQ-enabled 16/48 kHz comparisons on the USB bench firmware.

Requires hidapi. See docs/usb-audio-rate-experiment.md for preparation.
The ordinary daemon is suspended only during playback and always resumed.
"""
import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--deckd", required=True, help="deckd executable, used to inspect the running service")
    parser.add_argument("--serial", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("flash_usb", Path(__file__).with_name("flash-usb.py"))
    usb = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(usb)

    def call(method):
        run = subprocess.run([args.deckd, "ctl", "--scopes", "observe", method],
                             capture_output=True, text=True, check=True, timeout=10)
        return json.loads(run.stdout)

    tracks = json.loads(args.manifest.read_text())["tracks"]
    assert tracks, "Empty comparison"
    for track in tracks:
        raw = (args.manifest.parent / track["file"]).read_bytes()
        assert len(raw) % 64 == 0 and len(raw) >= 192
        packets = [raw[i:i + 64] for i in range(0, len(raw), 64)]
        assert all(p[:2] == bytes([6, 7]) and 0 < p[2] <= 61 for p in packets)
        start, end = packets[0], packets[-1]
        rate = start[6] * 1000
        assert rate in (16000, 48000) and start[2] == 5 and start[3] == 1
        assert start[5] == 1 and start[7] == 20
        stream = start[4]
        assert end[3:5] == bytes([3, stream]) and end[5] == (len(packets) - 2) % 256
        assert all(p[2] > 6 and p[3:6] == bytes([2, stream, i % 256])
                   for i, p in enumerate(packets[1:-1]))
        samples = sum((p[2] - 6) * 2 for p in packets[1:-1])
        track.update(packets=packets, rate=rate, samples=samples, stream=stream)

    info = call("service.info")
    assert info["link"] == "connected"
    voice = call("voice.get")
    assert voice["output_uid"] == "usb:" + args.serial
    assert voice["ptt"] is None and voice["speaking"] is None, "Deck is busy with voice activity"
    settings = call("device.get")["settings"]
    assert settings["speaker"] > 0, "Speaker is muted"
    pid = info["pid"]
    hid = usb.hid_module()
    if sys.platform == "darwin":
        lib = ctypes.CDLL(hid.__file__)
        lib.hid_darwin_set_open_exclusive.argtypes = [ctypes.c_int]
        lib.hid_darwin_set_open_exclusive(0)
    paths = usb.device_paths(args.serial)
    assert len(paths) == 1, "Expected one selected USB deck"
    link = usb.Link(paths[0])

    def rpc(method):
        reply = link.rpc(method, {}, timeout_s=3)
        if not isinstance(reply, dict) or "result" not in reply:
            raise RuntimeError(f"{method}: {reply}")
        return reply["result"]

    paused = False
    watchdog = None
    current = None
    results = []
    try:
        version = rpc("sys.version")
        assert version.get("audio_test") and version.get("usb_speaker_rates") == [16000, 48000], version
        total = sum(t["samples"] / t["rate"] + 4 for t in tracks)
        watchdog = subprocess.Popen([sys.executable, "-c",
            "import os,signal,sys,time; time.sleep(float(sys.argv[2])); os.kill(int(sys.argv[1]),signal.SIGCONT)",
            str(pid), str(total + 30)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True)
        os.kill(pid, signal.SIGSTOP)
        paused = True
        time.sleep(.15)
        for track in tracks:
            before = rpc("audio.test.status")
            assert before["playing"] == 0, "A previous stream is still active"
            current = track
            print(f'Playing {track["label"]}: {track["rate"]} Hz, EQ enabled, speaker {settings["speaker"]}/10.', flush=True)
            packets = track["packets"]
            assert link.dev.write(packets[0]) == 64
            # PLL/amp settle is ~113 ms. Avoid filling the 73 ms 48k queue
            # during that wait; the DAC begins with an 80 ms silent ring.
            time.sleep(.120)
            start = time.monotonic()
            sent = 0
            late = []
            for packet in packets[1:-1]:
                samples = (packet[2] - 6) * 2
                due = start + max(0, (sent + samples) / track["rate"] - .040)
                wait = due - time.monotonic()
                if wait > 0:
                    time.sleep(wait)
                late.append(max(0, time.monotonic() - due))
                if link.dev.write(packet) != 64:
                    raise RuntimeError("Incomplete USB audio write")
                sent += samples
            wait = start + sent / track["rate"] - time.monotonic()
            if wait > 0:
                time.sleep(wait)
            assert link.dev.write(packets[-1]) == 64
            for _ in range(30):
                time.sleep(.1)
                after = rpc("audio.test.status")
                if after["playing"] == 0:
                    break
            keys = ["starts", "ended", "frames", "samples", "gaps", "underruns", "queue_drops", "seq_errors"]
            delta = {key: after[key] - before[key] for key in keys}
            valid = (after["playing"] == 0 and after["rate_hz"] == track["rate"]
                     and delta["starts"] == delta["ended"] == 1
                     and delta["frames"] == len(packets) - 2 and delta["samples"] == sent
                     and all(delta[k] == 0 for k in ["gaps", "underruns", "queue_drops", "seq_errors"]))
            result = dict(label=track["label"], rate_hz=track["rate"], duration_s=sent / track["rate"],
                          device=delta, valid=valid, max_lateness_ms=round(max(late) * 1000, 2),
                          reports_late_over_20ms=sum(t > .020 for t in late), speaker=settings["speaker"], eq=True)
            results.append(result)
            args.out.write_text(json.dumps(results, indent=2) + "\n")
            print(json.dumps(result), flush=True)
            current = None
            if not valid:
                raise RuntimeError("Stream integrity check failed; stopping comparison to avoid misleading results")
            time.sleep(2)
    finally:
        try:
            if current is not None:
                link.send(7, bytes([3, current["stream"], 0, 1]))
        finally:
            link.close()
            if paused:
                os.kill(pid, signal.SIGCONT)
            if watchdog is not None:
                watchdog.terminate()
                watchdog.wait(timeout=5)
    time.sleep(1)
    assert call("service.info")["pid"] == pid
    assert call("device.get")["settings"]["speaker"] == settings["speaker"]
    print("Comparison complete; normal service resumed and speaker volume preserved.", flush=True)


if __name__ == "__main__":
    main()
