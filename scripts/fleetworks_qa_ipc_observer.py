#!/usr/bin/env python3
"""Forward the packaged GUI's real companion IPC and record only QA sequence evidence."""

import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import threading

from fleetworks_local_backend_smoke import MAX_IPC_FRAME_BYTES

OBSERVED = {"qa_turn_submit", "qa_events"}
MAX_RECORDS = 128


def read_frame(stream):
    header = stream.read(4)
    if not header:
        return None
    size, = struct.unpack(">I", header)
    if not 0 < size <= MAX_IPC_FRAME_BYTES:
        raise ValueError("Fixture IPC frame size invalid")
    data = stream.read(size)
    if len(data) != size:
        raise ValueError("Fixture IPC frame truncated")
    return header + data, json.loads(data)


def finding_digest(finding):
    return hashlib.sha256(json.dumps(finding, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def observe(companion, journal, args):
    pending = {}
    lock = threading.Lock()

    with journal.open("x", encoding="utf-8", buffering=1) as output, subprocess.Popen(
        [companion, *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    ) as actual:
        output.write(json.dumps({"companion_pid": actual.pid}) + "\n")

        def client_requests():
            with actual.stdin as downstream:
                while (frame := read_frame(sys.stdin.buffer)) is not None:
                    raw, decoded = frame
                    if decoded["frame"] == "request":
                        body = decoded["body"]
                        operation = body["op"]["op"]
                        if operation in OBSERVED:
                            with lock:
                                if len(pending) >= MAX_RECORDS:
                                    raise ValueError("Fixture IPC outstanding requests exceeded their bound")
                                pending[body["request_id"]] = operation
                    downstream.write(raw)
                    downstream.flush()

        input_thread = threading.Thread(target=client_requests, daemon=True)
        input_thread.start()
        recorded = 0
        try:
            while (frame := read_frame(actual.stdout)) is not None:
                raw, decoded = frame
                # This tap sends the production bytes unchanged. The record stays private.
                sys.stdout.buffer.write(raw)
                sys.stdout.buffer.flush()
                if decoded["frame"] != "response":
                    continue
                body = decoded["body"]
                with lock:
                    operation = pending.pop(body["request_id"], None)
                outcome = body["outcome"]
                if operation is None or outcome.get("outcome") != "ok":
                    continue
                if recorded >= MAX_RECORDS:
                    raise ValueError("Fixture IPC journal exceeded its record bound")
                value = outcome["data"]["value"]
                turn = value["turn"] if operation == "qa_events" else value
                events = value["events"] if operation == "qa_events" else []
                output.write(json.dumps({
                    "op": operation,
                    "turn_id": turn["turn_id"],
                    "state": turn["state"],
                    "cursor": turn["cursor"],
                    "findings": [finding_digest(finding) for finding in turn["findings"]],
                    "gap": value.get("gap", False),
                    "events": [
                        {"sequence": event["sequence"], "state": event["state"],
                         "finding": None if event["finding"] is None else finding_digest(event["finding"])}
                        for event in events
                    ],
                }) + "\n")
                recorded += 1
        except BrokenPipeError:
            actual.terminate()
        return actual.wait(timeout=10)


if __name__ == "__main__":
    raise SystemExit(observe(sys.argv[1], Path(sys.argv[2]), sys.argv[3:]))
