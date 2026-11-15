#!/usr/bin/env python3
"""Docker-free synthetic end-to-end smoke test for the Fleetworks local-IPC companion backend.

Spawns `argos-api-host --role local-ipc --state-dir <private-0o700-dir>` over
inherited length-delimited binary stdin/stdout frames (NEVER HTTP over pipes and
NEVER opening TCP/UDP sockets), verifies protocol version negotiation, zero
open sockets on the child PID, bounded error/capability handling, actor
catalog upload + seed + reindex + clean Shutdown/restart resume preserving the
exact same job_id without duplicates, and actor search visibility.

This verifies companion backend IPC mechanics cleanly without requiring Docker
or launching the GUI window; GUI verification remains separate.
"""

import argparse
import base64
import collections
import hashlib
import json
import os
import platform
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

IPC_PROTOCOL_VERSION = 1
IPC_MIN_PROTOCOL_VERSION = 1
MAX_IPC_FRAME_BYTES = 4 * 1024 * 1024
MAX_STDERR_TAIL_BYTES = 16 * 1024

DEFAULT_CSV = (
    b"actor_id,actor_name,actor_type,aliases,name_normalized\n"
    b"actor-smoke-1,Harbor Beacon Collective,group,[],harbor beacon collective\n"
    b"actor-smoke-2,Northwind Courier Network,organization,[],northwind courier network\n"
)


def resolve_companion_binary(*, host=None, app=None):
    """Resolve `argos-api-host` executable directly or inside a packaged bundle."""
    if bool(host) == bool(app):
        raise ValueError("Provide exactly one of --host or --app")
    if host is not None:
        candidate = Path(host).expanduser().resolve()
    else:
        bundle = Path(app).expanduser().resolve()
        mac_bin = bundle / "Contents/MacOS/argos-api-host"
        linux_bin = bundle / "bin/argos-api-host"
        if mac_bin.is_file():
            candidate = mac_bin
        elif linux_bin.is_file():
            candidate = linux_bin
        else:
            raise ValueError(f"Packaged bundle does not contain argos-api-host: {bundle}")
    if candidate.is_symlink() or not candidate.is_file() or not os.access(candidate, os.X_OK):
        raise ValueError(f"Companion backend binary is not a regular executable: {candidate}")
    return candidate


def encode_frame(envelope, *, max_bytes=MAX_IPC_FRAME_BYTES):
    """Encode an `IpcEnvelope` dict into a 4-byte BE length-prefixed UTF-8 JSON frame."""
    payload = json.dumps(envelope, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    if not payload or len(payload) > max_bytes:
        raise ValueError(f"IPC frame length out of bounds: {len(payload)}")
    return struct.pack(">I", len(payload)) + payload


def _read_exact(stream, count):
    chunks = []
    remaining = count
    while remaining > 0:
        chunk = stream.read(remaining)
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_frame(stream, *, max_bytes=MAX_IPC_FRAME_BYTES):
    """Read one bounded 4-byte BE length-prefixed JSON frame; returns None on clean EOF."""
    header = _read_exact(stream, 4)
    if not header:
        return None
    if len(header) < 4:
        raise ValueError("Truncated IPC frame length prefix")
    (length,) = struct.unpack(">I", header)
    if length == 0 or length > max_bytes:
        raise ValueError(f"Out-of-bounds IPC frame length: {length}")
    payload = _read_exact(stream, length)
    if len(payload) != length:
        raise ValueError("Truncated IPC frame payload")
    return json.loads(payload.decode("utf-8"))


def clean_child_environment():
    """Return a minimal environment stripped of remote tokens/secrets/URLs."""
    keep_keys = ("PATH", "HOME", "TMPDIR", "USER", "LOGNAME", "LANG", "LC_ALL", "RUST_LOG")
    env = {}
    for key in keep_keys:
        if key in os.environ:
            env[key] = os.environ[key]
    env.setdefault("RUST_LOG", "info")
    return env


def parse_lsof_sockets(stdout):
    """Return non-header socket lines from `lsof -nP -a -p <pid> -i`."""
    rows = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("COMMAND "):
            continue
        rows.append(stripped)
    return rows


def check_zero_bound_sockets(pid, *, runner=None):
    """Assert the given PID holds zero TCP/UDP network sockets."""
    proc_fd = Path(f"/proc/{pid}/fd")
    if proc_fd.is_dir():
        socket_inodes = set()
        for entry in proc_fd.iterdir():
            try:
                target = os.readlink(entry)
            except OSError:
                continue
            if target.startswith("socket:[") and target.endswith("]"):
                socket_inodes.add(target[8:-1])
        if socket_inodes:
            active_inodes = set()
            for proto in ("tcp", "tcp6", "udp", "udp6"):
                net_file = Path(f"/proc/{pid}/net/{proto}")
                if not net_file.is_file():
                    continue
                for line in net_file.read_text(encoding="utf-8", errors="replace").splitlines()[1:]:
                    parts = line.split()
                    if len(parts) > 9:
                        active_inodes.add(parts[9])
            bound = sorted(socket_inodes & active_inodes)
            if bound:
                raise ValueError(f"Local backend PID {pid} unexpectedly owns TCP/UDP sockets: {bound}")
        return {"method": "procfs", "socket_count": 0}

    result = (runner or subprocess.run)(
        ["lsof", "-nP", "-a", "-p", str(pid), "-i"],
        capture_output=True, text=True, timeout=10, check=False,
    )
    # `lsof` returns 1 when zero matching open files/sockets exist.
    if result.returncode not in (0, 1):
        raise ValueError(f"Unable to verify zero sockets via lsof: {result.stderr.strip()}")
    rows = parse_lsof_sockets(result.stdout)
    if rows:
        raise ValueError(f"Local backend PID {pid} unexpectedly opened network sockets: {rows}")
    return {"method": "lsof", "socket_count": 0}


def read_process_rss_bytes(pid, *, runner=None):
    """Observe child resident set size in bytes via `ps -o rss=`."""
    result = (runner or subprocess.run)(
        ["ps", "-o", "rss=", "-p", str(pid)],
        capture_output=True, text=True, timeout=5, check=False,
    )
    if result.returncode != 0:
        return None
    raw = result.stdout.strip()
    return int(raw) * 1024 if raw.isdigit() else None


class LocalIpcSession:
    """Managed child `argos-api-host --role local-ipc` process speaking binary IPC frames."""

    def __init__(self, binary, state_dir, *, client_name="fleetworks-local-smoke", client_version="0.2.1"):
        self.binary = Path(binary)
        self.state_dir = Path(state_dir)
        self.client_name = client_name
        self.client_version = client_version
        self._next_request_id = 1
        self.observed_events = []
        self._stderr_tail = collections.deque(maxlen=256)
        self.proc = subprocess.Popen(
            [str(self.binary), "--role", "local-ipc", "--state-dir", str(self.state_dir)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=clean_child_environment(),
        )
        self._stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._stderr_thread.start()

    def _drain_stderr(self):
        captured = 0
        while True:
            line = self.proc.stderr.readline()
            if not line:
                break
            text = line.decode("utf-8", errors="replace").rstrip("\n")
            captured += len(text)
            if captured <= MAX_STDERR_TAIL_BYTES * 8:
                self._stderr_tail.append(text[:512])

    def stderr_excerpt(self):
        return "\n".join(self._stderr_tail)[-MAX_STDERR_TAIL_BYTES:]

    def hello(self, *, protocol_version=IPC_PROTOCOL_VERSION, min_protocol_version=IPC_MIN_PROTOCOL_VERSION):
        envelope = {
            "frame": "hello",
            "body": {
                "protocol_version": protocol_version,
                "min_protocol_version": min_protocol_version,
                "client_name": self.client_name,
                "client_version": self.client_version,
            },
        }
        self.proc.stdin.write(encode_frame(envelope))
        self.proc.stdin.flush()
        reply = read_frame(self.proc.stdout)
        if not reply or reply.get("frame") != "hello_ack":
            raise ValueError(f"Expected hello_ack frame, got {reply!r}; stderr:\n{self.stderr_excerpt()}")
        return reply["body"]

    def request(self, op, *, registration=None):
        req_id = self._next_request_id
        self._next_request_id += 1
        envelope = {
            "frame": "request",
            "body": {
                "request_id": req_id,
                "registration": registration,
                "op": op,
            },
        }
        started = time.monotonic()
        self.proc.stdin.write(encode_frame(envelope))
        self.proc.stdin.flush()
        while True:
            frame = read_frame(self.proc.stdout)
            if frame is None:
                raise ValueError(f"Companion pipe closed while awaiting response {req_id}; stderr:\n{self.stderr_excerpt()}")
            kind = frame.get("frame")
            if kind == "event":
                self.observed_events.append(frame.get("body"))
                continue
            if kind == "response" and frame.get("body", {}).get("request_id") == req_id:
                latency_ms = round((time.monotonic() - started) * 1000.0, 2)
                return frame["body"]["outcome"], latency_ms
            raise ValueError(f"Unexpected IPC frame {frame!r} while waiting for request {req_id}")

    def request_json(self, op, *, registration=None):
        outcome, latency_ms = self.request(op, registration=registration)
        if outcome.get("outcome") != "ok":
            raise ValueError(f"IPC operation {op.get('op')} failed: {outcome}")
        data = outcome.get("data", {})
        if data.get("payload") not in ("json", "dataset_upload"):
            raise ValueError(f"Expected JSON/dataset IPC payload, got {data!r}")
        return data["value"], latency_ms

    def shutdown(self, reason="smoke_complete", *, timeout=20):
        self.proc.stdin.write(encode_frame({"frame": "shutdown", "body": {"reason": reason}}))
        self.proc.stdin.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            frame = read_frame(self.proc.stdout)
            if frame is None:
                raise ValueError("EOF before receiving shutdown_ack")
            if frame.get("frame") == "event":
                self.observed_events.append(frame.get("body"))
                continue
            if frame.get("frame") == "shutdown_ack":
                exit_code = self.proc.wait(timeout=10)
                self._stderr_thread.join(timeout=2)
                return frame["body"], exit_code
        raise TimeoutError("Timed out waiting for shutdown_ack")

    def close_unconditionally(self):
        try:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait(timeout=5)
        except OSError:
            pass
        for handle in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
            try:
                if handle is not None:
                    handle.close()
            except OSError:
                pass


def run_smoke(binary, *, state_dir=None, check_sockets=True):
    """Execute the full Docker-free local-IPC lifecycle smoke suite against `binary`."""
    binary = Path(binary).resolve()
    cleanup_dir = None
    if state_dir is None:
        cleanup_dir = tempfile.TemporaryDirectory(prefix="fleetworks-local-ipc-smoke-")
        root_dir = Path(cleanup_dir.name).resolve()
    else:
        root_dir = Path(state_dir).expanduser().resolve()
        root_dir.mkdir(parents=True, exist_ok=True)
    root_dir.chmod(0o700)

    try:
        # 1. Verify incompatible IPC protocol versions fail closed with compatible=False.
        mismatch_dir = root_dir / "version-mismatch"
        mismatch_dir.mkdir(mode=0o700, exist_ok=True)
        mismatch_dir.chmod(0o700)
        bad_session = LocalIpcSession(binary, mismatch_dir)
        try:
            bad_ack = bad_session.hello(protocol_version=99, min_protocol_version=99)
            if bad_ack.get("compatible") is not False:
                raise ValueError(f"Expected incompatible hello_ack, got {bad_ack!r}")
            bad_exit = bad_session.proc.wait(timeout=10)
            if bad_exit == 0:
                # Exiting 0 or non-zero cleanly without hanging or falling back to HTTP is acceptable,
                # as long as compatible=False was sent first and no fallback occurred.
                pass
        finally:
            bad_session.close_unconditionally()

        # 2. Run stateful local-IPC lifecycle with mid-job Shutdown & same-state-dir restart resume.
        live_dir = root_dir / "state"
        live_dir.mkdir(mode=0o700, exist_ok=True)
        live_dir.chmod(0o700)
        rpc_latencies_ms = []
        peak_rss_bytes = 0
        installation_marker = "install-local-smoke-0001"

        session1 = LocalIpcSession(binary, live_dir)
        try:
            t0 = time.monotonic()
            ack1 = session1.hello()
            cold_start_ms = round((time.monotonic() - t0) * 1000.0, 2)
            if ack1.get("compatible") is not True:
                raise ValueError(f"Expected compatible hello_ack, got {ack1!r}")
            capabilities = ack1.get("capabilities") or {}
            if not capabilities.get("features") and not capabilities.get("categories"):
                raise ValueError(f"Companion failed to advertise capabilities: {capabilities!r}")

            if check_sockets:
                socket_check_1 = check_zero_bound_sockets(session1.proc.pid)
            else:
                socket_check_1 = {"method": "skipped", "socket_count": 0}

            # Verify pre-registration readiness exposes disabled_reason while unregistered jobs_list fails closed.
            pre_readiness, lat = session1.request_json({"op": "workstation_readiness"})
            rpc_latencies_ms.append(lat)
            if pre_readiness.get("registration") is not None:
                raise ValueError(f"Expected unregistered readiness registration=None, got {pre_readiness!r}")
            unreg_jobs, lat = session1.request({"op": "jobs_list", "args": {}})
            rpc_latencies_ms.append(lat)
            if unreg_jobs.get("outcome") != "err":
                raise ValueError(f"Expected unregistered jobs_list rejection, got {unreg_jobs!r}")

            # Register local workstation and resolve active RegisteredContext.
            ensured, lat = session1.request_json({
                "op": "registration_ensure",
                "args": {
                    "request_id": "smoke-ensure-1",
                    "workstation_ip": None,
                    "installation_marker": installation_marker,
                },
            })
            rpc_latencies_ms.append(lat)
            ctx_resp, lat = session1.request_json({
                "op": "registration_context",
                "args": {
                    "registration_id": ensured["registration_id"],
                    "expected_revision": ensured["revision"],
                },
            })
            rpc_latencies_ms.append(lat)
            registration = ctx_resp["context"]

            readiness, lat = session1.request_json(
                {"op": "workstation_readiness"}, registration=registration,
            )
            rpc_latencies_ms.append(lat)
            if not readiness.get("deployment_id"):
                raise ValueError(f"Workstation readiness missing deployment_id: {readiness!r}")

            # Stage synthetic actor catalog CSV via chunked binary IPC.
            csv_digest = hashlib.sha256(DEFAULT_CSV).hexdigest()
            ticket, lat = session1.request_json(
                {
                    "op": "dataset_upload_start",
                    "args": {
                        "kind": "actor_catalog",
                        "sha256": csv_digest,
                        "size_bytes": len(DEFAULT_CSV),
                    },
                },
                registration=registration,
            )
            rpc_latencies_ms.append(lat)
            upload_id = ticket["upload_path"].rstrip("/").rsplit("/", 1)[-1]
            chunk_outcome, lat = session1.request(
                {
                    "op": "dataset_upload_chunk",
                    "args": {
                        "upload_id": upload_id,
                        "offset": 0,
                        "data_b64": base64.b64encode(DEFAULT_CSV).decode("ascii"),
                    },
                },
                registration=registration,
            )
            rpc_latencies_ms.append(lat)
            if chunk_outcome.get("outcome") != "ok":
                raise ValueError(f"dataset_upload_chunk failed: {chunk_outcome!r}")
            finished_upload, lat = session1.request_json(
                {"op": "dataset_upload_finish", "args": {"upload_id": upload_id}},
                registration=registration,
            )
            rpc_latencies_ms.append(lat)
            dataset_ref = finished_upload["dataset"]

            mutation_context = {
                "registration_id": registration["registration_id"],
                "expected_registration_revision": registration["registration_revision"],
                "expected_context_revision": registration["context_revision"],
                "expected_configuration_revision": 0,
            }
            seed_receipt, lat = session1.request_json(
                {
                    "op": "seed_actors",
                    "args": {
                        "request_id": "smoke-seed-1",
                        "context": mutation_context,
                        "dataset": dataset_ref,
                    },
                },
                registration=registration,
            )
            rpc_latencies_ms.append(lat)
            seed_job_id = seed_receipt["job_id"]

            # Poll until SeedActors job completes cleanly.
            deadline = time.monotonic() + 30.0
            while True:
                jobs, lat = session1.request_json(
                    {"op": "jobs_list", "args": {}}, registration=registration,
                )
                rpc_latencies_ms.append(lat)
                matched = [row for row in jobs.get("rows", []) if row.get("id") == seed_job_id]
                if matched and matched[0].get("state") == "completed":
                    break
                if matched and matched[0].get("state") in ("failed", "cancelled"):
                    raise ValueError(f"Seed job ended unexpectedly: {matched[0]!r}")
                if time.monotonic() > deadline:
                    raise TimeoutError(f"Seed job {seed_job_id} did not complete within 30s; rows={jobs!r}")
                time.sleep(0.1)

            # Dispatch ReindexActors, then immediately trigger graceful Shutdown before it finishes.
            reindex_receipt, lat = session1.request_json(
                {
                    "op": "reindex_actors",
                    "args": {
                        "request_id": "smoke-reindex-1",
                        "context": mutation_context,
                    },
                },
                registration=registration,
            )
            rpc_latencies_ms.append(lat)
            reindex_job_id = reindex_receipt["job_id"]
            rss1 = read_process_rss_bytes(session1.proc.pid) or 0
            peak_rss_bytes = max(peak_rss_bytes, rss1)

            shutdown_ack, exit_code1 = session1.shutdown("smoke_mid_reindex_restart")
            if exit_code1 != 0:
                raise ValueError(f"Companion session 1 exited non-zero ({exit_code1}) on shutdown")
        finally:
            session1.close_unconditionally()

        # 3. Restart on the EXACT SAME state directory and verify same job_id resumes with zero duplicates.
        session2 = LocalIpcSession(binary, live_dir)
        try:
            ack2 = session2.hello()
            if ack2.get("compatible") is not True:
                raise ValueError(f"Session 2 rejected compatible handshake: {ack2!r}")
            if check_sockets:
                check_zero_bound_sockets(session2.proc.pid)

            ensured2, lat = session2.request_json({
                "op": "registration_ensure",
                "args": {
                    "request_id": "smoke-ensure-2",
                    "workstation_ip": None,
                    "installation_marker": installation_marker,
                },
            })
            rpc_latencies_ms.append(lat)
            ctx_resp2, lat = session2.request_json({
                "op": "registration_context",
                "args": {
                    "registration_id": ensured2["registration_id"],
                    "expected_revision": ensured2["revision"],
                },
            })
            rpc_latencies_ms.append(lat)
            registration2 = ctx_resp2["context"]

            deadline = time.monotonic() + 30.0
            while True:
                jobs2, lat = session2.request_json(
                    {"op": "jobs_list", "args": {}}, registration=registration2,
                )
                rpc_latencies_ms.append(lat)
                reindex_rows = [
                    row for row in jobs2.get("rows", [])
                    if row.get("operation") == "actor_reindex"
                ]
                if len(reindex_rows) != 1:
                    raise ValueError(
                        f"Expected exactly 1 actor_reindex job after restart (no duplicates), found {len(reindex_rows)}: {reindex_rows!r}"
                    )
                if reindex_rows[0].get("id") != reindex_job_id:
                    raise ValueError(
                        f"Expected resumed job_id {reindex_job_id}, got {reindex_rows[0].get('id')}"
                    )
                if reindex_rows[0].get("state") == "completed":
                    break
                if reindex_rows[0].get("state") in ("failed", "cancelled"):
                    raise ValueError(f"Resumed reindex job failed unexpectedly: {reindex_rows[0]!r}")
                if time.monotonic() > deadline:
                    raise TimeoutError(f"Resumed reindex job {reindex_job_id} timed out: {reindex_rows[0]!r}")
                time.sleep(0.1)

            search_resp, lat = session2.request_json(
                {
                    "op": "actors_search",
                    "args": {
                        "query": "Harbor Beacon",
                        "mode": "discovery",
                        "limit": 10,
                    },
                },
                registration=registration2,
            )
            rpc_latencies_ms.append(lat)
            hits = search_resp.get("rows", [])
            if search_resp.get("index", {}).get("state") != "ready":
                raise ValueError(f"Actor search index not ready after restart completion: {search_resp!r}")
            if not any(hit.get("label") == "Harbor Beacon Collective" for hit in hits):
                raise ValueError(f"Seeded actor missing from local search results: {hits!r}")

            rss2 = read_process_rss_bytes(session2.proc.pid) or 0
            peak_rss_bytes = max(peak_rss_bytes, rss2)
            shutdown_ack2, exit_code2 = session2.shutdown("smoke_complete")
            if exit_code2 != 0:
                raise ValueError(f"Companion session 2 exited non-zero ({exit_code2}) on shutdown")
        finally:
            session2.close_unconditionally()

        return {
            "status": "pass",
            "non_gui_synthetic_smoke": True,
            "docker_required": False,
            "transport": "length_delimited_stdin_stdout_ipc",
            "zero_network_sockets_verified": bool(check_sockets),
            "socket_check_method": socket_check_1["method"],
            "incompatible_version_rejected": True,
            "cold_start_ms": cold_start_ms,
            "rpc_count": len(rpc_latencies_ms),
            "rpc_latency_max_ms": max(rpc_latencies_ms),
            "peak_rss_bytes": peak_rss_bytes,
            "capabilities": capabilities,
            "seed_job_id": seed_job_id,
            "restart_resume_job_id": reindex_job_id,
            "shutdown_ack_1": shutdown_ack,
            "shutdown_ack_2": shutdown_ack2,
            "search_hits": len(hits),
            "platform": {
                "system": platform.system(),
                "machine": platform.machine(),
            },
        }
    finally:
        if cleanup_dir is not None:
            cleanup_dir.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, help="Path to argos-api-host executable")
    parser.add_argument("--app", type=Path, help="Path to packaged Fleetworks.app or Fleetworks-linux-x86_64 bundle")
    parser.add_argument("--state-dir", type=Path, help="Optional scratch directory for local backend state")
    parser.add_argument("--output", type=Path, help="Optional file path to write JSON smoke summary")
    args = parser.parse_args()
    try:
        binary = resolve_companion_binary(host=args.host, app=args.app)
        summary = run_smoke(binary, state_dir=args.state_dir)
    except (OSError, ValueError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"fleetworks-local-backend-smoke: {error}", file=sys.stderr)
        return 1
    rendered = json.dumps(summary, indent=2)
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())
