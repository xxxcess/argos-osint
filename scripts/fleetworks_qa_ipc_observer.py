#!/usr/bin/env python3
"""Forward packaged GUI IPC and record bounded QA and seed acceptance evidence."""

import hashlib
import json
from pathlib import Path
import socket
import subprocess
import sys
import threading

from fleetworks_local_backend_smoke import encode_frame, read_wire_frame, spawn_ipc_child, transport_attestation

OBSERVED = {"seed_actors", "qa_turn_submit", "qa_events", "qa_cancel"}
MAX_RECORDS = 128
AUDIT_REQUEST_ID = 1 << 63


def finding_digest(finding):
    return hashlib.sha256(json.dumps(finding, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def memory_observation(findings):
    for finding in findings:
        if finding["source"] != {"kind": "host_metric", "id": "memory"}:
            continue
        sample = finding["measurement"]
        value = sample["value"]
        observed_at = finding["observed_at"]
        if (sample["unit"] == "percent_tenths" and sample["subject"] == "API host memory usage"
                and type(value) is int and 0 <= value <= 1000 and len(observed_at) <= 40):
            return {"value": value, "observed_at": observed_at}
    return None


def observe(companion, journal, args):
    fd_index = args.index("--ipc-fd")
    upstream = socket.socket(fileno=int(args[fd_index + 1]))
    upstream.set_inheritable(False)
    command = [companion, *args[:fd_index], *args[fd_index + 2:]]
    pending = {}
    lock = threading.Lock()
    write_lock = threading.Lock()
    reader_failures = []

    with upstream, upstream.makefile("rb") as client_reader, journal.open("x", encoding="utf-8", buffering=1) as output:
        actual, downstream = spawn_ipc_child(command)

        def client_requests():
            try:
                while (frame := read_wire_frame(client_reader, allow_upload=True)) is not None:
                    raw, decoded = frame
                    if decoded["frame"] == "request":
                        body = decoded["body"]
                        operation = body["op"]["op"]
                        if body["request_id"] == AUDIT_REQUEST_ID:
                            raise ValueError("Fixture IPC audit request ID collided")
                        if operation in OBSERVED:
                            registration = body.get("registration") if operation == "qa_cancel" else None
                            with lock:
                                if len(pending) >= MAX_RECORDS:
                                    raise ValueError("Fixture IPC outstanding requests exceeded their bound")
                                pending[body["request_id"]] = (operation, registration)
                    with write_lock:
                        downstream.sendall(raw)
            except (OSError, ValueError) as error:
                reader_failures.append(error)
            finally:
                try:
                    downstream.shutdown(socket.SHUT_WR)
                except OSError:
                    pass

        input_thread = threading.Thread(target=client_requests, daemon=True)
        recorded = 0
        audit_sent = False
        try:
            with downstream, downstream.makefile("rb") as backend_reader:
                output.write(json.dumps({"companion_pid": actual.pid}) + "\n")
                input_thread.start()
                while (frame := read_wire_frame(backend_reader)) is not None:
                    raw, decoded = frame
                    kind, body = decoded["frame"], decoded["body"]
                    if kind == "hello_ack":
                        attested = transport_attestation(downstream, body)
                        if transport_attestation(upstream, body) != attested:
                            raise ValueError("Fixture relay socket differs from packaged companion")
                        output.write(json.dumps({"transport_attestation": attested}) + "\n")
                    audited = kind == "response" and body["request_id"] == AUDIT_REQUEST_ID
                    # Forward the production packets unchanged. The private audit is a read on the same socket.
                    if not audited:
                        upstream.sendall(raw)
                    if kind != "response":
                        continue
                    with lock:
                        observed = pending.pop(body["request_id"], None)
                    if observed is None:
                        continue
                    operation, registration = observed
                    outcome = body["outcome"]
                    if outcome.get("outcome") != "ok":
                        if audited:
                            raise ValueError("Canceled turn audit was rejected")
                        continue
                    if recorded >= MAX_RECORDS:
                        raise ValueError("Fixture IPC journal exceeded its record bound")
                    recorded += 1
                    if operation == "seed_actors":
                        output.write(json.dumps({"op": operation}) + "\n")
                        continue
                    value = outcome["data"]["value"]
                    page = operation in {"qa_events", "qa_cancel_audit"}
                    turn = value["turn"] if page else value
                    events = value["events"] if page else []
                    output.write(json.dumps({
                        "op": operation,
                        "turn_id": turn["turn_id"],
                        "state": turn["state"],
                        "cursor": turn["cursor"],
                        "findings": [finding_digest(finding) for finding in turn["findings"]],
                        "memory_observation": memory_observation(turn["findings"]),
                        "gap": value.get("gap", False),
                        "events": [
                            {"sequence": event["sequence"], "state": event["state"],
                             "finding": None if event["finding"] is None else finding_digest(event["finding"])}
                            for event in events
                        ],
                    }) + "\n")
                    if operation == "qa_cancel":
                        if audit_sent or registration is None:
                            raise ValueError("Expected one authorized GUI Stop request")
                        audit_sent = True
                        # The GUI stops polling after adopting the Cancel reply. Read the retained
                        # event sequence separately to prove exactly one cancel was persisted.
                        audit = {"frame": "request", "body": {
                            "request_id": AUDIT_REQUEST_ID, "registration": registration,
                            "op": {"op": "qa_events", "args": {"turn_id": turn["turn_id"], "after": 0, "wait_seconds": 0}},
                        }}
                        with lock:
                            pending[AUDIT_REQUEST_ID] = ("qa_cancel_audit", None)
                        with write_lock:
                            downstream.sendall(encode_frame(audit))
            if reader_failures:
                raise reader_failures[0]
            return actual.wait(timeout=10)
        except BrokenPipeError:
            return 0
        finally:
            try:
                upstream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            downstream.close()
            if actual.poll() is None:
                actual.terminate()
                try:
                    actual.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    actual.kill()
                    actual.wait(timeout=5)
            if input_thread.ident is not None:
                input_thread.join(timeout=5)


if __name__ == "__main__":
    raise SystemExit(observe(sys.argv[1], Path(sys.argv[2]), sys.argv[3:]))
