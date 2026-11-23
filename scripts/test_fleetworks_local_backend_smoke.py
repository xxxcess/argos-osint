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

import fleetworks_local_backend_smoke as smoke
from fleetworks_local_backend_smoke import (
    EXPECTED_RESOURCE_SCOPES,
    MAX_IPC_FRAME_BYTES,
    LocalIpcSession,
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

    def test_observer_preserves_raw_upload_and_control_reader_rejects_it(self):
        upload = {"frame": "request", "body": {"request_id": 7, "op": {
            "op": "dataset_upload_chunk", "args": {"upload_id": "test", "offset": 0},
        }}}
        chunk = b"actor,data\n"
        encoded = smoke.encode_upload_frame(upload, chunk)
        self.assertTrue(encoded.endswith(chunk))
        self.assertEqual(smoke.read_wire_frame(io.BytesIO(encoded), allow_upload=True), (encoded, upload))
        with self.assertRaisesRegex(ValueError, "Unexpected IPC upload frame"):
            read_frame(io.BytesIO(encoded))

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

    def test_run_smoke_verifies_restart_for_a_new_job_or_completed_index(self):
        mock_bin = self.root / "mock-companion.py"
        state_db = self.root / "mock-state.json"
        mock_bin.write_text(
            f"""#!{sys.executable}
import json, os, socket, struct, sys
from pathlib import Path

STATE_FILE = Path({str(state_db)!r})
stream = socket.socket(fileno=int(sys.argv[sys.argv.index("--ipc-fd") + 1]))
reader = stream.makefile("rb")

def read_f():
    hdr = reader.read(4)
    if not hdr:
        return None
    (n,) = struct.unpack(">I", hdr)
    if n & (1 << 31):
        data = reader.read(n & ~(1 << 31))
        (meta,) = struct.unpack(">I", data[:4])
        frame = json.loads(data[4:4 + meta])
        frame["chunk"] = data[4 + meta:]
        return frame
    return json.loads(reader.read(n))

def write_f(obj):
    data = json.dumps(obj).encode("utf-8")
    stream.sendall(struct.pack(">I", len(data)) + data)

def ok(req_id, value):
    write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "ok", "data": {{"payload": "json", "value": value}}}}}}}})

def err(req_id, status=401):
    write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "err", "data": {{"status": status, "code": "rejected", "message": "rejected"}}}}}}}})

hello = read_f()
if hello["body"]["protocol_version"] != 2:
    write_f({{"frame": "hello_ack", "body": {{"protocol_version": 2, "min_protocol_version": 2, "compatible": False, "reason": "unsupported"}}}})
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
write_f({{"frame": "hello_ack", "body": {{"protocol_version": 2, "min_protocol_version": 2, "compatible": True,
    "transport": "embedded-unix-socket", "dataset_upload_chunk_bytes": 262144, "dataset_upload_window": 4,
    "capabilities": CAPS, "ready": {{"status": "ready"}}}}}})
state = json.loads(STATE_FILE.read_text()) if STATE_FILE.exists() else {{"jobs": []}}
upload_active = False

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
        upload_active = True
        ok(req_id, {{"upload_path": "/api/v1/workstation/datasets/uploads/up-1", "expires_in_seconds": 60}})
    elif op == "dataset_upload_chunk":
        assert args == {{"upload_id": "up-1", "offset": 0}} and frame["chunk"]
        write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "ok", "data": {{
            "payload": "dataset_upload_progress", "value": {{"received_bytes": len(frame["chunk"])}},
        }}}}}}}})
    elif op == "dataset_upload_cancel":
        upload_active = False
        write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "ok", "data": {{"payload": "empty"}}}}}}}})
    elif op == "dataset_upload_finish":
        if upload_active:
            ok(req_id, {{"dataset": {{"id": "ds-1", "kind": "actor_catalog", "sha256": "a" * 64, "size_bytes": 10}}}})
            upload_active = False
        else:
            err(req_id, 404)
    elif op == "seed_actors":
        state["jobs"].append({{"id": "job-seed-1", "operation": "actor_seed", "state": "completed"}})
        ok(req_id, {{"job_id": "job-seed-1"}})
    elif op == "reindex_actors":
        assert args["request_id"] == "smoke-reindex-1"
        if not state.get("index_observed"):
            state["index_observed"] = True
            write_f({{"frame": "response", "body": {{"request_id": req_id, "outcome": {{"outcome": "err", "data": {{"status": 422, "message": "Actor index status is still checking or catching up. Refresh status before building."}}}}}}}})
        elif state.get("already_indexed"):
            ok(req_id, {{"job_id": "", "already_completed": True, "reused": True}})
        else:
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

        for already_indexed, expected_job in ((False, "job-reindex-1"), (True, None)):
            with self.subTest(already_indexed=already_indexed):
                state_db.write_text(json.dumps({"jobs": [], "already_indexed": already_indexed}), encoding="utf-8")
                summary = run_smoke(mock_bin, state_dir=self.root / f"smoke-run-{already_indexed}", check_sockets=False, run_rust_probe=False)
                self.assertEqual(summary["status"], "pass")
                self.assertEqual(summary["transport"], "embedded-unix-socket")
                self.assertEqual(summary["canceled_upload"]["status"], "canceled")
                self.assertEqual(summary["rust_client_probe"], {"status": "skipped"})
                self.assertEqual(summary["restart_resume_job_id"], expected_job)
                self.assertEqual(summary["reindex_already_completed"], already_indexed)
                self.assertEqual(summary["search_hits"], 1)
                for key in ("shutdown_ack_1", "shutdown_ack_2"):
                    self.assertEqual(summary[key], {"checkpointed": True, "relinquished_leases": 1})

    def test_shutdown_retains_ack_when_the_process_wait_times_out(self):
        acknowledgement = {"checkpointed": True, "relinquished_leases": 1}
        process, endpoint = mock.Mock(), mock.Mock()
        endpoint.makefile.return_value = io.BytesIO(encode_frame({"frame": "shutdown_ack", "body": acknowledgement}))
        process.stderr = io.BytesIO()
        with mock.patch.object(smoke, "spawn_ipc_child", return_value=(process, endpoint)):
            session = LocalIpcSession(self.root / "synthetic-companion", self.root)

            def wait(*, timeout):
                endpoint.shutdown.assert_not_called()
                self.assertEqual(session.last_shutdown_ack, acknowledgement)
                raise subprocess.TimeoutExpired("synthetic-companion", timeout)

            process.wait.side_effect = wait
            with self.assertRaises(subprocess.TimeoutExpired):
                session.shutdown()
            process.wait.assert_called_once_with(timeout=10)

    def test_packaged_linux_pr_smoke_stages_catalog_and_requires_opt_in_for_seed(self):
        app = self.root / "app"
        output = self.root / "report.json"
        args = ["smoke", "--app", str(app), "--output", str(output)]
        with (
            mock.patch.object(smoke, "resolve_companion_binary", return_value=self.root / "companion"),
            mock.patch.object(smoke, "run_smoke", return_value={"status": "pass"}),
            mock.patch.object(smoke, "run_ci_bundled_catalog") as full_catalog,
            mock.patch.object(sys, "stdout", io.StringIO()),
            mock.patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}),
        ):
            with mock.patch.object(sys, "argv", args):
                self.assertEqual(smoke.main(), 0)
            full_catalog.assert_not_called()
            with mock.patch.object(sys, "argv", [*args, "--full-bundled-catalog"]):
                self.assertEqual(smoke.main(), 0)
            full_catalog.assert_called_once_with(app, output)
            full_catalog.reset_mock()
            with (
                mock.patch.object(sys, "argv", args),
                mock.patch.dict(os.environ, {
                    "GITHUB_ACTIONS": "true", "GITHUB_EVENT_NAME": "pull_request",
                    "LINUX_TARGET": "x86_64-unknown-linux-gnu", "LINUX_PROFILE": "debug",
                }),
            ):
                self.assertEqual(smoke.main(), 0)
            full_catalog.assert_called_once_with(app, output, stage_only=True)

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
