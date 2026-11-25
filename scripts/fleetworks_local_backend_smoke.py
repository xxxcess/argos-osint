#!/usr/bin/env python3
"""Docker-free synthetic end-to-end smoke test for the Fleetworks local-IPC companion backend.

Spawns `argos-api-host --role local-ipc --state-dir <private-0o700-dir>` over
an inherited Unix socket pair. Verifies protocol version negotiation, zero
network sockets on the child PID, bounded error/capability handling, actor
catalog upload + seed + reindex + clean Shutdown/restart resume preserving the
exact same job_id without duplicates, and actor search visibility.

This verifies companion backend IPC mechanics cleanly without requiring Docker
or launching the GUI window; GUI verification remains separate.
"""

import argparse
import collections
import hashlib
import json
import os
import platform
import shutil
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

IPC_PROTOCOL_VERSION = 2
IPC_MIN_PROTOCOL_VERSION = 2
MAX_IPC_FRAME_BYTES = 4 * 1024 * 1024
IPC_BINARY_FLAG = 1 << 31
IPC_UPLOAD_METADATA_BYTES = 16 * 1024
IPC_UPLOAD_CHUNK_BYTES = 256 * 1024
IPC_UPLOAD_WINDOW = 4
IPC_SOCKET_TIMEOUT_SECONDS = 60
MAX_STDERR_TAIL_BYTES = 16 * 1024
NATIVE_CONTRACT_API_VERSION = 1
CONNECT_CONTRACT_VERSION = 2
CONNECT_REGISTRY_FEATURE = "connect-registry-v1"
EXPECTED_RESOURCE_SCOPES = {
    "atlas": "backend",
    "brain": "backend",
    "tools": "backend",
    "models": "backend",
    "profile": "backend",
    "native_jobs_logs": "workstation_triggered_activity",
}

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


def encode_upload_frame(envelope, chunk):
    if not 0 < len(chunk) <= IPC_UPLOAD_CHUNK_BYTES:
        raise ValueError("IPC upload chunk length is out of bounds")
    metadata = json.dumps(envelope, separators=(",", ":")).encode("utf-8")
    size = 4 + len(metadata) + len(chunk)
    if len(metadata) > IPC_UPLOAD_METADATA_BYTES or size > MAX_IPC_FRAME_BYTES:
        raise ValueError("IPC upload frame length is out of bounds")
    return struct.pack(">II", IPC_BINARY_FLAG | size, len(metadata)) + metadata + chunk


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


def read_wire_frame(stream, *, max_bytes=MAX_IPC_FRAME_BYTES, allow_upload=False):
    """Read one bounded v2 frame and return the original wire bytes and metadata."""
    header = _read_exact(stream, 4)
    if not header:
        return None
    if len(header) < 4:
        raise ValueError("Truncated IPC frame length prefix")
    (prefix,) = struct.unpack(">I", header)
    binary = bool(prefix & IPC_BINARY_FLAG)
    length = prefix & ~IPC_BINARY_FLAG
    if binary and not allow_upload:
        raise ValueError("Unexpected IPC upload frame")
    if length == 0 or length > max_bytes:
        raise ValueError(f"Out-of-bounds IPC frame length: {length}")
    payload = _read_exact(stream, length)
    if len(payload) != length:
        raise ValueError("Truncated IPC frame payload")
    if binary:
        if len(payload) < 4:
            raise ValueError("Truncated IPC upload metadata length")
        (metadata_length,) = struct.unpack(">I", payload[:4])
        if (not 0 < metadata_length < len(payload) - 4 or metadata_length > IPC_UPLOAD_METADATA_BYTES
                or len(payload) - 4 - metadata_length > IPC_UPLOAD_CHUNK_BYTES):
            raise ValueError("Invalid IPC upload payload lengths")
        metadata = payload[4:4 + metadata_length]
    else:
        metadata = payload
    return header + payload, json.loads(metadata)


def read_frame(stream, *, max_bytes=MAX_IPC_FRAME_BYTES):
    frame = read_wire_frame(stream, max_bytes=max_bytes)
    return frame[1] if frame is not None else None


def spawn_ipc_child(command, **options):
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        with child:
            fd = child.fileno()
            process = subprocess.Popen([*command, "--ipc-fd", str(fd)], pass_fds=(fd,),
                                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, **options)
        return process, parent
    except BaseException:
        parent.close()
        raise


def transport_attestation(stream, hello):
    """Bind the actual socket endpoint to the compatible packaged companion handshake."""
    if (stream.family != socket.AF_UNIX or stream.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM
            or stream.getsockname() not in ("", b"") or stream.getpeername() not in ("", b"")):
        raise ValueError("Expected an unnamed connected Unix stream for Embedded IPC")
    fields = {"transport": "embedded-unix-socket", "protocol_version": IPC_PROTOCOL_VERSION,
              "min_protocol_version": IPC_MIN_PROTOCOL_VERSION, "dataset_upload_chunk_bytes": IPC_UPLOAD_CHUNK_BYTES,
              "dataset_upload_window": IPC_UPLOAD_WINDOW}
    if hello.get("compatible") is not True or any(
        type(hello.get(key)) is not type(value) or hello[key] != value for key, value in fields.items()
    ):
        raise ValueError("Packaged companion handshake did not attest the expected Embedded socket")
    return {**{key: hello[key] for key in fields}, "socket_family": "unix", "socket_type": "stream"}


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

    def __init__(self, binary, state_dir, *, client_name="fleetworks-local-smoke", client_version="0.2.1",
                 log_filter=None, qa_memory_mib=None):
        self.binary = Path(binary)
        self.state_dir = Path(state_dir)
        self.client_name = client_name
        self.client_version = client_version
        self._next_request_id = 1
        self.observed_events = []
        self.last_shutdown_ack = None
        self._stderr_tail = collections.deque(maxlen=256)
        environment = clean_child_environment()
        if log_filter is not None:
            environment["RUST_LOG"] = log_filter
        if qa_memory_mib is not None:
            environment["FLEETWORKS_QA_MEMORY_MIB"] = str(qa_memory_mib)
        self.proc, self.socket = spawn_ipc_child(
            [str(self.binary), "--role", "local-ipc", "--state-dir", str(self.state_dir)],
            stderr=subprocess.PIPE,
            env=environment,
        )
        self.socket.settimeout(IPC_SOCKET_TIMEOUT_SECONDS)
        self.reader = self.socket.makefile("rb")
        self.transport_attestation = None
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
        return "\n".join(list(self._stderr_tail))[-MAX_STDERR_TAIL_BYTES:]

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
        self.socket.sendall(encode_frame(envelope))
        reply = read_frame(self.reader)
        if not reply or reply.get("frame") != "hello_ack":
            raise ValueError(f"Expected hello_ack frame, got {reply!r}; stderr:\n{self.stderr_excerpt()}")
        body = reply["body"]
        if body.get("compatible") is True:
            self.transport_attestation = transport_attestation(self.socket, body)
        return body

    def send_request(self, op, *, registration=None, chunk=None):
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
        self.socket.sendall(encode_frame(envelope) if chunk is None else encode_upload_frame(envelope, chunk))
        return req_id

    def receive_response(self):
        while True:
            frame = read_frame(self.reader)
            if frame is None:
                raise ValueError(f"Companion socket closed while awaiting a response; stderr:\n{self.stderr_excerpt()}")
            kind = frame.get("frame")
            if kind == "event":
                self.observed_events.append(frame.get("body"))
                continue
            if kind == "response":
                return frame["body"]["request_id"], frame["body"]["outcome"]
            raise ValueError("Unexpected companion frame while awaiting a response")

    def request(self, op, *, registration=None, chunk=None):
        started = time.monotonic()
        expected = self.send_request(op, registration=registration, chunk=chunk)
        req_id, outcome = self.receive_response()
        if req_id != expected:
            raise ValueError(f"Unexpected IPC response ID {req_id} while waiting for {expected}")
        return outcome, round((time.monotonic() - started) * 1000.0, 2)

    def request_json(self, op, *, registration=None):
        outcome, latency_ms = self.request(op, registration=registration)
        if outcome.get("outcome") != "ok":
            raise ValueError(f"IPC operation {op.get('op')} failed: {outcome}")
        data = outcome.get("data", {})
        if data.get("payload") not in ("json", "dataset_upload"):
            raise ValueError(f"Expected JSON/dataset IPC payload, got {data!r}")
        return data["value"], latency_ms

    def upload_dataset_bytes(self, source, upload_id, registration):
        """Upload bounded raw chunks and check that control traffic shares the live socket."""
        offset = 0
        control_ms = None
        while True:
            pending = {}
            for _ in range(IPC_UPLOAD_WINDOW):
                chunk = source.read(IPC_UPLOAD_CHUNK_BYTES)
                if not chunk:
                    break
                operation = {"op": "dataset_upload_chunk", "args": {"upload_id": upload_id, "offset": offset}}
                req_id = self.send_request(operation, registration=registration, chunk=chunk)
                offset += len(chunk)
                pending[req_id] = offset
            if not pending:
                return offset, control_ms
            control = None
            if control_ms is None:
                started = time.monotonic()
                control = self.send_request({"op": "workstation_readiness"}, registration=registration)
                pending[control] = None
            while pending:
                req_id, outcome = self.receive_response()
                if req_id not in pending or outcome.get("outcome") != "ok":
                    raise ValueError("Unexpected response while uploading a dataset")
                expected = pending.pop(req_id)
                data = outcome["data"]
                if req_id == control:
                    if data.get("payload") != "json" or not data.get("value", {}).get("deployment_id"):
                        raise ValueError("Readiness failed while transferring a dataset")
                    control_ms = round((time.monotonic() - started) * 1000, 2)
                elif data.get("payload") != "dataset_upload_progress" or data.get("value", {}).get("received_bytes") != expected:
                    raise ValueError("Companion returned an unexpected cumulative upload offset")

    def verify_upload_cancel(self, dataset_bytes, registration):
        """Cancel after sending a partial raw chunk and before consuming its response."""
        ticket, _ = self.request_json({"op": "dataset_upload_start", "args": {
            "kind": "actor_catalog", "sha256": hashlib.sha256(dataset_bytes).hexdigest(),
            "size_bytes": len(dataset_bytes),
        }}, registration=registration)
        upload_id = ticket["upload_path"].rstrip("/").rsplit("/", 1)[-1]
        partial = dataset_bytes[:len(dataset_bytes) // 2]
        chunk = self.send_request({"op": "dataset_upload_chunk", "args": {"upload_id": upload_id, "offset": 0}},
                                  registration=registration, chunk=partial)
        started = time.monotonic()
        cancel = self.send_request({"op": "dataset_upload_cancel", "args": {"upload_id": upload_id}},
                                   registration=registration)
        replies = {}
        for _ in range(2):
            req_id, outcome = self.receive_response()
            if req_id in replies:
                raise ValueError("Unexpected duplicate response while canceling dataset transfer")
            replies[req_id] = outcome
            if req_id == cancel:
                elapsed_ms = round((time.monotonic() - started) * 1000, 2)
        if replies.get(cancel) != {"outcome": "ok", "data": {"payload": "empty"}}:
            raise ValueError("Companion did not acknowledge dataset cancellation")
        chunk_ack = {"outcome": "ok", "data": {
            "payload": "dataset_upload_progress", "value": {"received_bytes": len(partial)},
        }}
        if replies.get(chunk) != chunk_ack and not upload_destination_closed(replies.get(chunk, {})):
            raise ValueError("Outstanding chunk reported an unexpected result during dataset cancellation")
        result, _ = self.request({"op": "dataset_upload_finish", "args": {"upload_id": upload_id}},
                                 registration=registration)
        if not upload_destination_closed(result):
            raise ValueError("Canceled upload remained available for publication")
        return {"status": "canceled", "partial_chunk_bytes": len(partial), "cancel_ms": elapsed_ms}

    def shutdown(self, reason="smoke_complete", *, timeout=20):
        self.socket.settimeout(timeout)
        self.socket.sendall(encode_frame({"frame": "shutdown", "body": {"reason": reason}}))
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            frame = read_frame(self.reader)
            if frame is None:
                raise ValueError("EOF before receiving shutdown_ack")
            if frame.get("frame") == "event":
                self.observed_events.append(frame.get("body"))
                continue
            if frame.get("frame") == "shutdown_ack":
                self.last_shutdown_ack = frame.get("body")
                if (
                    not isinstance(self.last_shutdown_ack, dict)
                    or self.last_shutdown_ack.get("checkpointed") is not True
                ):
                    raise ValueError("Companion did not acknowledge a checkpointed shutdown")
                # The companion closes its side after writing the final ack.
                exit_code = self.proc.wait(timeout=10)
                self._stderr_thread.join(timeout=2)
                return self.last_shutdown_ack, exit_code
        raise TimeoutError("Timed out waiting for shutdown_ack")

    def close_unconditionally(self):
        try:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait(timeout=5)
        except OSError:
            pass
        for handle in (self.reader, self.socket, self.proc.stderr):
            try:
                if handle is not None:
                    handle.close()
            except OSError:
                pass


def upload_destination_closed(outcome):
    return outcome.get("outcome") == "err" and outcome.get("data", {}).get("status") == 404


def validate_native_contract(capabilities):
    """Enforce the same required native capabilities contract as `Capabilities::require_native_contract`."""
    if not isinstance(capabilities, dict):
        raise ValueError(f"Companion failed to advertise capabilities: {capabilities!r}")
    if capabilities.get("api_version") != NATIVE_CONTRACT_API_VERSION:
        raise ValueError(f"Unsupported native contract api_version: {capabilities.get('api_version')!r}")
    for key in ("deployment_id", "configuration_epoch"):
        value = capabilities.get(key)
        if not isinstance(value, str) or not value:
            raise ValueError(f"Companion capabilities missing required `{key}`: {capabilities!r}")
    connect = capabilities.get("connect")
    if (
        not isinstance(connect, dict)
        or connect.get("version") != CONNECT_CONTRACT_VERSION
        or connect.get("registration_required") is not True
    ):
        raise ValueError(f"Companion capabilities missing v{CONNECT_CONTRACT_VERSION} connect contract: {connect!r}")
    features = capabilities.get("features")
    if not isinstance(features, list) or CONNECT_REGISTRY_FEATURE not in features:
        raise ValueError(f"Companion capabilities missing feature `{CONNECT_REGISTRY_FEATURE}`: {features!r}")
    workspace_control = capabilities.get("workspace_control")
    if (
        not isinstance(workspace_control, dict)
        or workspace_control.get("version") != CONNECT_CONTRACT_VERSION
        or workspace_control.get("purpose") != "registration_and_activity"
    ):
        raise ValueError(f"Companion capabilities invalid workspace_control: {workspace_control!r}")
    if capabilities.get("resource_scopes") != EXPECTED_RESOURCE_SCOPES:
        raise ValueError(f"Companion capabilities invalid resource_scopes: {capabilities.get('resource_scopes')!r}")


def run_rust_client_probe(binary, state_dir, *, probe_bin=None, repo_root=None, runner=None):
    """Run the compiled Rust `LocalIpcClient` / `Client::from_local_ipc` probe against `binary`."""
    state_dir = Path(state_dir).resolve()
    state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    state_dir.chmod(0o700)
    root = Path(repo_root).resolve() if repo_root else Path(__file__).resolve().parent.parent
    if probe_bin is not None:
        command = [str(Path(probe_bin).resolve()), "--host", str(binary), "--state-dir", str(state_dir)]
    else:
        agent_cargo = root / "scripts/agent_cargo.py"
        if not agent_cargo.is_file():
            raise ValueError(f"Missing Cargo wrapper for Rust IPC probe: {agent_cargo}")
        command = [
            sys.executable,
            str(agent_cargo),
            "run",
            "--locked",
            "-p",
            "fleetworks-contracts",
            "--example",
            "local_ipc_probe",
            "--",
            "--host",
            str(binary),
            "--state-dir",
            str(state_dir),
        ]
    completed = (runner or subprocess.run)(
        command,
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
        timeout=600,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip()[-2000:]
        raise ValueError(f"Rust local-IPC client probe failed ({completed.returncode}): {detail or 'no output'}")
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"Rust local-IPC client probe emitted invalid JSON: {error}") from error
    if not isinstance(payload, dict) or payload.get("status") != "pass":
        raise ValueError(f"Rust local-IPC client probe did not pass: {payload!r}")
    return payload


def run_smoke(binary, *, state_dir=None, check_sockets=True, run_rust_probe=True, rust_probe_bin=None):
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
            capabilities = ack1.get("capabilities")
            validate_native_contract(capabilities)
            rpc_caps, lat = session1.request_json({"op": "capabilities"})
            rpc_latencies_ms.append(lat)
            validate_native_contract(rpc_caps)
            if (
                rpc_caps.get("deployment_id") != capabilities.get("deployment_id")
                or rpc_caps.get("configuration_epoch") != capabilities.get("configuration_epoch")
            ):
                raise ValueError(
                    f"Capabilities identity mismatch between hello_ack ({capabilities!r}) and RPC ({rpc_caps!r})"
                )

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

            canceled_upload = session1.verify_upload_cancel(DEFAULT_CSV, registration)

            # Stage synthetic actor catalog CSV using one binary upload frame.
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
                    },
                },
                registration=registration,
                chunk=DEFAULT_CSV,
            )
            rpc_latencies_ms.append(lat)
            if chunk_outcome != {"outcome": "ok", "data": {
                "payload": "dataset_upload_progress", "value": {"received_bytes": len(DEFAULT_CSV)},
            }}:
                raise ValueError("Unexpected cumulative acknowledgement for the synthetic upload")
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

            # Reuse the same request as normal index observations settle. A caught-up
            # generation is accepted without creating a job.
            reindex_request = {
                "op": "reindex_actors",
                "args": {"request_id": "smoke-reindex-1", "context": mutation_context},
            }
            deadline = time.monotonic() + 30.0
            while True:
                outcome, lat = session1.request(reindex_request, registration=registration)
                rpc_latencies_ms.append(lat)
                data = outcome.get("data", {})
                if outcome.get("outcome") == "ok" and data.get("payload") == "json":
                    reindex_receipt = data["value"]
                    break
                if not (
                    data.get("status") == 422
                    and data.get("message", "").startswith("Actor index status is still checking or catching up.")
                ):
                    raise ValueError(f"IPC reindex admission failed: {outcome!r}")
                if time.monotonic() >= deadline:
                    raise TimeoutError("Actor index status did not settle before reindex admission")
                time.sleep(0.1)

            reindex_job_id = reindex_receipt["job_id"]
            reindex_already_completed = reindex_receipt.get("already_completed", False)
            if reindex_already_completed:
                if reindex_job_id or not reindex_receipt.get("reused"):
                    raise ValueError("Completed reindex must reuse coverage without creating a job")
            elif not reindex_job_id:
                raise ValueError("Accepted reindex is missing its job ID")
            rss1 = read_process_rss_bytes(session1.proc.pid) or 0
            peak_rss_bytes = max(peak_rss_bytes, rss1)

            shutdown_ack, exit_code1 = session1.shutdown("smoke_mid_reindex_restart")
            if exit_code1 != 0:
                raise ValueError(f"Companion session 1 exited non-zero ({exit_code1}) on shutdown")
        finally:
            session1.close_unconditionally()

        # 3. Verify the same state directory retains the admitted job or completed reuse.
        session2 = LocalIpcSession(binary, live_dir)
        try:
            ack2 = session2.hello()
            if ack2.get("compatible") is not True:
                raise ValueError(f"Session 2 rejected compatible handshake: {ack2!r}")
            caps2 = ack2.get("capabilities")
            validate_native_contract(caps2)
            if (
                caps2.get("deployment_id") != capabilities.get("deployment_id")
                or caps2.get("configuration_epoch") != capabilities.get("configuration_epoch")
            ):
                raise ValueError(
                    f"Deployment identity drifted across restart: before={capabilities!r}, after={caps2!r}"
                )
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
                if reindex_already_completed:
                    if reindex_rows:
                        raise ValueError(f"Reusing the index created reindex jobs: {reindex_rows!r}")
                    replay, lat = session2.request_json(reindex_request, registration=registration2)
                    rpc_latencies_ms.append(lat)
                    if replay != reindex_receipt:
                        raise ValueError("Completed reindex receipt changed after restart")
                    break
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

            deadline = time.monotonic() + 30.0
            while True:
                search_resp, lat = session2.request_json(
                    {
                        "op": "actors_search",
                        "args": {"query": "Harbor Beacon", "mode": "discovery", "limit": 10},
                    },
                    registration=registration2,
                )
                rpc_latencies_ms.append(lat)
                hits = search_resp.get("rows", [])
                if search_resp.get("index", {}).get("state") == "ready" and any(
                    hit.get("label") == "Harbor Beacon Collective" for hit in hits
                ):
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"Seeded actor was not searchable after restart: {search_resp!r}")
                time.sleep(0.1)

            rss2 = read_process_rss_bytes(session2.proc.pid) or 0
            peak_rss_bytes = max(peak_rss_bytes, rss2)
            shutdown_ack2, exit_code2 = session2.shutdown("smoke_complete")
            if exit_code2 != 0:
                raise ValueError(f"Companion session 2 exited non-zero ({exit_code2}) on shutdown")
        finally:
            session2.close_unconditionally()

        if run_rust_probe:
            probe_dir = root_dir / "rust-probe-state"
            rust_client_probe = run_rust_client_probe(binary, probe_dir, probe_bin=rust_probe_bin)
        else:
            rust_client_probe = {"status": "skipped"}

        return {
            "status": "pass",
            "non_gui_synthetic_smoke": True,
            "docker_required": False,
            "transport": session1.transport_attestation["transport"],
            "transport_attestation": session1.transport_attestation,
            "canceled_upload": canceled_upload,
            "zero_network_sockets_verified": bool(check_sockets),
            "socket_check_method": socket_check_1["method"],
            "incompatible_version_rejected": True,
            "cold_start_ms": cold_start_ms,
            "rpc_count": len(rpc_latencies_ms),
            "rpc_latency_max_ms": max(rpc_latencies_ms),
            "peak_rss_bytes": peak_rss_bytes,
            "capabilities": capabilities,
            "rust_client_probe": rust_client_probe,
            "seed_job_id": seed_job_id,
            "restart_resume_job_id": reindex_job_id or None,
            "reindex_already_completed": reindex_already_completed,
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


def is_ci_bundled_catalog_app(app, output):
    environment = os.environ
    return (app is not None and output is not None and environment.get("GITHUB_ACTIONS") == "true"
            and environment.get("GITHUB_EVENT_NAME") == "pull_request"
            and environment.get("LINUX_TARGET") == "x86_64-unknown-linux-gnu"
            and environment.get("LINUX_PROFILE") == "debug")


def run_ci_bundled_catalog(app, output, *, stage_only=False):
    # The Linux native package step has already verified and extracted this exact archive.
    if not is_ci_bundled_catalog_app(app, output):
        raise ValueError("Full bundled catalog verification requires a Linux x86-64 debug PR package and output path")
    environment = os.environ
    revision = environment["EXPECTED_HEAD"]
    run_id = environment["GITHUB_RUN_ID"]
    run_attempt = environment["GITHUB_RUN_ATTEMPT"]
    stem = "bundled-catalog-stage" if stage_only else "bundled-catalog"
    evidence = output.with_name(f"{stem}-x86_64-unknown-linux-gnu.json")
    script = Path(__file__).resolve().with_name("fleetworks_qa_smoke.py")
    result = None
    try:
        with tempfile.TemporaryDirectory(prefix="fleetworks-catalog-home-") as home, evidence.open("w") as report:
            command = [
                "/usr/bin/sudo", "--non-interactive", "--", "/usr/bin/unshare", "--net",
                "--setgid", str(os.getgid()), "--setuid", str(os.getuid()), "--",
                "/usr/bin/env", "-i", f"HOME={home}", "PATH=/usr/sbin:/usr/bin:/sbin:/bin",
                "/usr/bin/python3", "-B", str(script), "catalog", "--app", str(app.resolve()),
                "--revision", revision, "--run-id", run_id, "--run-attempt", run_attempt,
            ]
            if stage_only:
                command.append("--stage-only")
            completed = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=report,
                                       timeout=(5 if stage_only else 45) * 60, check=False)
        result = json.loads(evidence.read_text())
        if (completed.returncode != 0 or not isinstance(result, dict)
                or result.get("source_sha") != revision or result.get("run_id") != run_id
                or result.get("run_attempt") != run_attempt or not isinstance(result.get("bundled_catalog"), dict)
                or result["bundled_catalog"].get("scope") != ("stage_only" if stage_only else "full_catalog")
                or result["bundled_catalog"].get("status") != "passed"):
            raise ValueError("offline bundled catalog smoke did not pass")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        if not isinstance(result, dict) or not isinstance(result.get("bundled_catalog"), dict):
            evidence.write_text(json.dumps({
                "source_sha": revision, "run_id": run_id, "run_attempt": run_attempt,
                "bundled_catalog": {"status": "failed", "stage": "offline_launch", "failure_type": type(error).__name__},
            }, indent=2) + "\n", encoding="utf-8")
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, help="Path to argos-api-host executable")
    parser.add_argument("--app", type=Path, help="Path to packaged Fleetworks.app or Fleetworks-linux-x86_64 bundle")
    parser.add_argument("--state-dir", type=Path, help="Optional scratch directory for local backend state")
    parser.add_argument("--rust-probe-bin", type=Path, help="Optional prebuilt local_ipc_probe Rust binary")
    parser.add_argument("--skip-rust-probe", action="store_true", help="Skip the compiled Rust LocalIpcClient probe")
    parser.add_argument("--output", type=Path, help="Optional file path to write JSON smoke summary")
    parser.add_argument("--full-bundled-catalog", action="store_true",
                        help="Also run the full offline catalog import on a verified Linux debug PR package")
    args = parser.parse_args()
    try:
        binary = resolve_companion_binary(host=args.host, app=args.app)
        summary = run_smoke(
            binary,
            state_dir=args.state_dir,
            run_rust_probe=not args.skip_rust_probe,
            rust_probe_bin=args.rust_probe_bin,
        )
    except (OSError, ValueError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"fleetworks-local-backend-smoke: {error}", file=sys.stderr)
        return 1
    rendered = json.dumps(summary, indent=2)
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered, flush=True)
    if args.full_bundled_catalog or is_ci_bundled_catalog_app(args.app, args.output):
        try:
            if args.full_bundled_catalog:
                run_ci_bundled_catalog(args.app, args.output)
            else:
                run_ci_bundled_catalog(args.app, args.output, stage_only=True)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            print(f"fleetworks-local-backend-smoke: {error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
