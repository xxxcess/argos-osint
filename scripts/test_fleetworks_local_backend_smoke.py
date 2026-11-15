"""Offline unit tests for the Docker-free local-IPC smoke runner and framing codec."""

import io
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from fleetworks_local_backend_smoke import (
    EXPECTED_RESOURCE_SCOPES,
    MAX_IPC_FRAME_BYTES,
    check_zero_bound_sockets,
    clean_child_environment,
    encode_frame,
    parse_lsof_sockets,
    read_frame,
    resolve_companion_binary,
    run_rust_client_probe,
    run_smoke,
    validate_native_contract,
)


class LocalBackendSmokeTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name).resolve()

    def test_frame_codec_enforces_length_bounds_and_clean_eof(self):
        payload = {"frame": "hello", "body": {"protocol_version": 1}}
        encoded = encode_frame(payload)
        decoded = read_frame(io.BytesIO(encoded))
        self.assertEqual(decoded, payload)
        self.assertIsNone(read_frame(io.BytesIO(b"")))
        with self.assertRaisesRegex(ValueError, "Truncated"):
            read_frame(io.BytesIO(b"\x00\x00"))
        with self.assertRaisesRegex(ValueError, "Out-of-bounds"):
            read_frame(io.BytesIO(struct.pack(">I", 0)))
        with self.assertRaisesRegex(ValueError, "Out-of-bounds"):
            read_frame(io.BytesIO(struct.pack(">I", MAX_IPC_FRAME_BYTES + 1)))

    def test_resolve_companion_binary_supports_mac_and_linux_bundles(self):
        mac_app = self.root / "Fleetworks.app"
        mac_bin = mac_app / "Contents/MacOS/argos-api-host"
        mac_bin.parent.mkdir(parents=True)
        mac_bin.write_bytes(b"#!/bin/sh\n")
        mac_bin.chmod(0o755)
        self.assertEqual(resolve_companion_binary(app=mac_app), mac_bin)

        linux_app = self.root / "Fleetworks-linux-x86_64"
        linux_bin = linux_app / "bin/argos-api-host"
        linux_bin.parent.mkdir(parents=True)
        linux_bin.write_bytes(b"#!/bin/sh\n")
        linux_bin.chmod(0o755)
        self.assertEqual(resolve_companion_binary(app=linux_app), linux_bin)

        with self.assertRaisesRegex(ValueError, "Provide exactly one"):
            resolve_companion_binary()

    def test_socket_checker_detects_listening_sockets_and_cleans_env(self):
        self.assertEqual(parse_lsof_sockets("COMMAND PID USER FD TYPE\n"), [])
        self.assertEqual(
            parse_lsof_sockets("COMMAND PID USER FD TYPE\nargos 10 u 4u IPv4 TCP 127.0.0.1:8080 (LISTEN)\n"),
            ["argos 10 u 4u IPv4 TCP 127.0.0.1:8080 (LISTEN)"],
        )
        with mock.patch("fleetworks_local_backend_smoke.Path.is_dir", return_value=False):
            res = check_zero_bound_sockets(
                10, runner=lambda *_a, **_kw: subprocess.CompletedProcess([], 1, "", ""),
            )
            self.assertEqual(res["socket_count"], 0)
            with self.assertRaisesRegex(ValueError, "unexpectedly opened network sockets"):
                check_zero_bound_sockets(
                    10,
                    runner=lambda *_a, **_kw: subprocess.CompletedProcess(
                        [], 0, "COMMAND PID\nargos 10 IPv4 TCP *:9000\n", "",
                    ),
                )
        with mock.patch.dict(os.environ, {"PATH": "/bin", "ARGOS_OPERATOR_TOKEN": "secret"}, clear=True):
            cleaned = clean_child_environment()
            self.assertNotIn("ARGOS_OPERATOR_TOKEN", cleaned)
            self.assertEqual(cleaned["PATH"], "/bin")

    def test_run_smoke_exercises_version_mismatch_and_restart_resume_with_mock_child(self):
        mock_bin = self.root / "mock-companion.py"
        state_db = self.root / "mock-state.json"
        mock_bin.write_text(
            f"""#!{sys.executable}
import json, os, struct, sys
from pathlib import Path

STATE_FILE = Path({str(state_db)!r})

def read_f():
    hdr = sys.stdin.buffer.read(4)
    if not hdr:
        return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_f(obj):
    data = json.dumps(obj).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(data)) + data)
    sys.stdout.buffer.flush()

def ok(req_id, value):
    write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "ok", "data": {{"payload": "json", "value": value}}}}}}}})

def err(req_id):
    write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "err", "data": {{"status": 401, "code": "unregistered", "message": "unregistered"}}}}}}}})

hello = read_f()
if hello["body"]["protocol_version"] != 1:
    write_f({{"frame": "hello_ack", "body": {{"protocol_version": 1, "min_protocol_version": 1, "compatible": False, "reason": "unsupported"}}}})
    sys.exit(2)

CAPS = {{
    "api_version": 1,
    "deployment_id": "dep-local-1",
    "configuration_epoch": "epoch-1",
    "connect": {{"version": 2, "registration_required": True}},
    "features": ["connect-registry-v1", "tantivy", "embedded"],
    "workspace_control": {{"version": 2, "purpose": "registration_and_activity"}},
    "resource_scopes": {EXPECTED_RESOURCE_SCOPES!r},
}}
write_f({{"frame": "hello_ack", "body": {{"protocol_version": 1, "min_protocol_version": 1, "compatible": True, "capabilities": CAPS, "ready": {{"status": "ready"}}}}}})
state = json.loads(STATE_FILE.read_text()) if STATE_FILE.exists() else {{"jobs": []}}

while True:
    frame = read_f()
    if frame is None:
        break
    if frame["frame"] == "shutdown":
        for job in state["jobs"]:
            if job["state"] == "running":
                job["state"] = "completed"
        STATE_FILE.write_text(json.dumps(state))
        write_f({{"frame": "shutdown_ack", "body": {{"checkpointed": True, "relinquished_leases": 1}}}})
        break
    req_id = frame["body"]["request_id"]
    op = frame["body"]["op"]["op"]
    args = frame["body"]["op"].get("args", {{}})
    if op == "capabilities":
        ok(req_id, CAPS)
    elif op == "jobs_list" and frame["body"].get("registration") is None:
        err(req_id)
    elif op == "registration_ensure":
        ok(req_id, {{"registration_id": "reg-1", "revision": 1}})
    elif op == "registration_context":
        ok(req_id, {{"context": {{"registration_id": "reg-1", "registration_revision": 1, "context_revision": 1}}}})
    elif op == "workstation_readiness":
        ok(req_id, {{"deployment_id": "dep-local-1", "registration": frame["body"].get("registration")}})
    elif op == "dataset_upload_start":
        ok(req_id, {{"upload_path": "/api/v1/workstation/datasets/uploads/up-1", "expires_in_seconds": 60}})
    elif op == "dataset_upload_chunk":
        write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "ok", "data": {{"payload": "empty"}}}}}}}})
    elif op == "dataset_upload_finish":
        ok(req_id, {{"dataset": {{"id": "ds-1", "kind": "actor_catalog", "sha256": "a" * 64, "size_bytes": 10}}}})
    elif op == "seed_actors":
        state["jobs"].append({{"id": "job-seed-1", "operation": "actor_seed", "state": "completed"}})
        ok(req_id, {{"job_id": "job-seed-1"}})
    elif op == "reindex_actors":
        state["jobs"].append({{"id": "job-reindex-1", "operation": "actor_reindex", "state": "running"}})
        ok(req_id, {{"job_id": "job-reindex-1"}})
    elif op == "jobs_list":
        ok(req_id, {{"rows": state["jobs"]}})
    elif op == "actors_search":
        ok(req_id, {{"index": {{"state": "ready"}}, "rows": [{{"actor_id": "catalog-actor-1", "label": "Harbor Beacon Collective"}}]}})
""",
            encoding="utf-8",
        )
        mock_bin.chmod(stat.S_IRWXU)

        summary = run_smoke(mock_bin, state_dir=self.root / "smoke-run", check_sockets=False, run_rust_probe=False)
        self.assertEqual(summary["status"], "pass")
        self.assertEqual(summary["rust_client_probe"], {"status": "skipped"})
        self.assertEqual(summary["restart_resume_job_id"], "job-reindex-1")
        self.assertEqual(summary["search_hits"], 1)

    def test_validate_native_contract_enforces_required_fields(self):
        valid = {
            "api_version": 1,
            "deployment_id": "dep-1",
            "configuration_epoch": "epoch-1",
            "connect": {"version": 2, "registration_required": True},
            "features": ["connect-registry-v1"],
            "workspace_control": {"version": 2, "purpose": "registration_and_activity"},
            "resource_scopes": dict(EXPECTED_RESOURCE_SCOPES),
        }
        validate_native_contract(valid)
        for mutation in (
            {"deployment_id": ""},
            {"connect": {"version": 1, "registration_required": True}},
            {"features": ["tantivy"]},
            {"workspace_control": {"version": 2, "purpose": "wrong"}},
            {"resource_scopes": {"atlas": "local"}},
        ):
            bad = dict(valid)
            bad.update(mutation)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_native_contract(bad)

    def test_run_rust_client_probe_invokes_cargo_or_prebuilt_and_fails_closed(self):
        repo = self.root / "repo"
        (repo / "scripts").mkdir(parents=True)
        (repo / "scripts/agent_cargo.py").write_text("#!/usr/bin/env python3\n", encoding="utf-8")
        host = self.root / "argos-api-host"
        host.write_text("#!/bin/sh\n", encoding="utf-8")
        commands = []

        def ok_runner(cmd, **_kw):
            commands.append(cmd)
            return subprocess.CompletedProcess(cmd, 0, json.dumps({"status": "pass", "api_version": 1}), "")

        res = run_rust_client_probe(host, self.root / "probe-state", repo_root=repo, runner=ok_runner)
        self.assertEqual(res["status"], "pass")
        self.assertEqual(
            commands[0][1:],
            [
                str(repo / "scripts/agent_cargo.py"),
                "run", "--locked", "-p", "fleetworks-contracts", "--example", "local_ipc_probe",
                "--", "--host", str(host), "--state-dir", str((self.root / "probe-state").resolve()),
            ],
        )

        prebuilt = self.root / "local_ipc_probe"
        prebuilt.write_text(f"#!{sys.executable}\nimport json\nprint(json.dumps({{'status': 'pass', 'api_version': 1}}))\n")
        prebuilt.chmod(stat.S_IRWXU)
        direct = run_rust_client_probe(host, self.root / "probe-state-2", probe_bin=prebuilt)
        self.assertEqual(direct["status"], "pass")

        with self.assertRaisesRegex(ValueError, "probe failed"):
            run_rust_client_probe(
                host,
                self.root / "probe-state-fail",
                probe_bin=prebuilt,
                runner=lambda cmd, **_kw: subprocess.CompletedProcess(cmd, 1, "", "contract error"),
            )


if __name__ == "__main__":
    unittest.main()
