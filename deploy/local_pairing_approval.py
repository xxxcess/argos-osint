#!/usr/bin/env python3
"""Bounded local Operator handoff for a native-review pairing challenge.

The native app sends challenge metadata and its approval code through a private
Unix socket. A separate macOS dialog requires an explicit Operator decision.
Only this helper reads the Operator token file and submits the decision.
"""

from __future__ import annotations

import argparse
from collections import deque
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import signal
import socket
import stat
import subprocess
import sys
import time
from typing import Callable
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from fleetworks_pairing import origin as normalize_origin  # noqa: E402
from fleetworks_pairing import perform as perform_decision  # noqa: E402
from fleetworks_pairing import PairingError  # noqa: E402

MAX_LINE_BYTES = 2048
MAX_PROMPTS = 6
LIFETIME_SECONDS = 3600
SOCKET_NAME = "approval.sock"
IDENTITY = re.compile(r"[A-Za-z0-9_-]{16,128}\Z")
APPROVAL_CODE = re.compile(r"[0-9a-f]{32}\Z")

APPLE_SCRIPT = '''on run argv
    set gatewayOrigin to item 1 of argv
    set requestID to item 2 of argv
    set approvalCode to "__APPROVAL_CODE__"
    set messageText to "Gateway: " & gatewayOrigin & return & "Pairing request: " & requestID & return & "Approval code: " & approvalCode & return & return & "Approve this Mac only if these details match Fleetworks."
    try
        set answerRecord to display dialog messageText buttons {"Cancel", "Approve"} default button "Cancel" with title "Fleetworks Operator Approval" giving up after 120
        if gave up of answerRecord then return "Cancel"
        return button returned of answerRecord
    on error number -128
        return "Cancel"
    end try
end run
'''

FEEDBACK_SCRIPT = '''try
    display alert "Fleetworks pairing was not approved" message "The gateway did not accept this pairing request. Check Fleetworks and try again." as warning buttons {"OK"} default button "OK" giving up after 15
end try
'''

class HandoffError(Exception):
    """A bounded request or private-socket error safe to report without data."""


@dataclass(frozen=True)
class ApprovalRequest:
    origin: str
    pairing_id: str
    approval_code: str
    expires_at_ms: int


def operator_prompt(request: ApprovalRequest, timeout: float) -> bool:
    """Return whether the Operator explicitly clicked Approve."""
    if sys.platform != "darwin":
        return False
    try:
        result = subprocess.run(
            ["osascript", "-", request.origin, request.pairing_id],
            input=APPLE_SCRIPT.replace("__APPROVAL_CODE__", request.approval_code),
            capture_output=True,
            text=True,
            timeout=min(timeout, 130),
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    if result.returncode != 0 or len(result.stdout) > 256:
        return False
    return result.stdout.strip() == "Approve"


def operator_feedback() -> None:
    """Show a generic, bounded gateway failure without disclosing credentials."""
    if sys.platform != "darwin":
        return
    try:
        subprocess.run(
            ["osascript", "-"], input=FEEDBACK_SCRIPT, capture_output=True,
            text=True, timeout=20, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        pass


class ApprovalBroker:
    def __init__(
        self,
        origin: str,
        token_file: Path,
        *,
        prompt: Callable[[ApprovalRequest, float], bool] = operator_prompt,
        perform: Callable[..., dict] = perform_decision,
        clock_ms: Callable[[], int] = lambda: int(time.time() * 1000),
        monotonic: Callable[[], float] = time.monotonic,
    ):
        try:
            self.origin = normalize_origin(origin)
        except PairingError as error:
            raise HandoffError("Local approval requires a loopback HTTP gateway") from error
        parsed = urlsplit(self.origin)
        if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost", "::1"):
            raise HandoffError("Local approval requires a loopback HTTP gateway")
        self.token_file = token_file
        self.prompt = prompt
        self.perform = perform
        self.clock_ms = clock_ms
        self.monotonic = monotonic
        self.seen: set[str] = set()
        self.prompt_times: deque[float] = deque()

    def reserve(self, payload: object) -> ApprovalRequest:
        if type(payload) is not dict or set(payload) != {
            "version", "origin", "pairing_id", "approval_code", "expires_at_ms"
        }:
            raise HandoffError("Invalid local pairing handoff")
        if type(payload["version"]) is not int or payload["version"] != 1:
            raise HandoffError("Unsupported local pairing handoff")
        if type(payload["origin"]) is not str or len(payload["origin"]) > 256:
            raise HandoffError("Invalid local pairing origin")
        try:
            requested_origin = normalize_origin(payload["origin"])
        except PairingError as error:
            raise HandoffError("Invalid local pairing origin") from error
        if requested_origin != self.origin:
            raise HandoffError("Local pairing origin differs from this review")
        pairing_id = payload["pairing_id"]
        if type(pairing_id) is not str or not IDENTITY.fullmatch(pairing_id):
            raise HandoffError("Invalid local pairing request")
        approval_code = payload["approval_code"]
        if type(approval_code) is not str or not APPROVAL_CODE.fullmatch(approval_code):
            raise HandoffError("Invalid local pairing approval code")
        expiry = payload["expires_at_ms"]
        now_ms = self.clock_ms()
        if (type(expiry) is not int or expiry <= now_ms + 1000
                or expiry > now_ms + 630_000):
            raise HandoffError("Local pairing request expired or has an invalid deadline")
        now = self.monotonic()
        while self.prompt_times and now - self.prompt_times[0] >= LIFETIME_SECONDS:
            self.prompt_times.popleft()
        if pairing_id in self.seen:
            raise HandoffError("Local pairing request already prompted")
        if len(self.prompt_times) >= MAX_PROMPTS:
            raise HandoffError("Local pairing prompts are temporarily limited")
        self.seen.add(pairing_id)
        self.prompt_times.append(now)
        return ApprovalRequest(self.origin, pairing_id, approval_code, expiry)

    def decide(self, request: ApprovalRequest, lifetime_remaining: float) -> bool:
        if lifetime_remaining <= 15 or request.expires_at_ms <= self.clock_ms() + 1000:
            return False
        timeout = min(130.0, lifetime_remaining - 15)
        if self.prompt(request, timeout) is not True:
            return False
        if request.expires_at_ms <= self.clock_ms():
            return False
        result = self.perform(
            request.origin, self.token_file, "approve", request.pairing_id,
            request.approval_code,
        )
        return result == {"status": "approved"}


def _read_request(connection: socket.socket) -> object:
    connection.settimeout(1.0)
    data = bytearray()
    while b"\n" not in data:
        chunk = connection.recv(min(512, MAX_LINE_BYTES + 1 - len(data)))
        if not chunk:
            raise HandoffError("Incomplete local pairing handoff")
        data.extend(chunk)
        if len(data) > MAX_LINE_BYTES:
            raise HandoffError("Local pairing handoff exceeds its bound")
    line, remainder = bytes(data).split(b"\n", 1)
    if remainder or not line:
        raise HandoffError("Invalid local pairing handoff framing")
    try:
        return json.loads(line)
    except (UnicodeError, ValueError) as error:
        raise HandoffError("Invalid local pairing handoff JSON") from error


def _peer_is_owner(connection: socket.socket) -> bool:
    if hasattr(connection, "getpeereid"):
        uid, _ = connection.getpeereid()
        return uid == os.getuid()
    return True  # The 0700 directory is the peer boundary on Linux CI.


def _socket_dir(raw: Path) -> Path:
    if not raw.is_absolute() or raw.name in ("", ".", ".."):
        raise HandoffError("Choose a new absolute private socket directory")
    parent = raw.parent.resolve(strict=True)
    info = parent.stat()
    if not stat.S_ISDIR(info.st_mode) or (
        info.st_mode & 0o022 and not info.st_mode & stat.S_ISVTX
    ):
        raise HandoffError("Socket parent directory is not private or sticky")
    directory = parent / raw.name
    if len(os.fsencode(str(directory / SOCKET_NAME))) >= 100:
        raise HandoffError("Local pairing socket path is too long")
    os.mkdir(directory, 0o700)
    try:
        os.chmod(directory, 0o700)
        created = directory.lstat()
        if not stat.S_ISDIR(created.st_mode) or created.st_uid != os.getuid():
            raise HandoffError("Private socket directory ownership changed")
    except Exception:
        directory.rmdir()
        raise
    return directory


def serve(
    raw_dir: Path,
    broker: ApprovalBroker,
    *,
    ready: Callable[[Path], None] = lambda path: print(f"READY {path}", flush=True),
    stop: Callable[[], bool] = lambda: False,
    monotonic: Callable[[], float] = time.monotonic,
    lifetime_seconds: float = LIFETIME_SECONDS,
) -> None:
    """Serve one bounded request at a time; acknowledge before showing UI."""
    directory = _socket_dir(raw_dir)
    path = directory / SOCKET_NAME
    directory_identity = directory.stat()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    socket_identity = None
    try:
        previous_umask = os.umask(0o077)
        try:
            listener.bind(str(path))
        finally:
            os.umask(previous_umask)
        os.chmod(path, 0o600)
        socket_identity = path.lstat()
        listener.listen(4)
        listener.settimeout(1.0)
        deadline = monotonic() + lifetime_seconds
        ready(path)
        while not stop() and monotonic() < deadline:
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                continue
            request = None
            with connection:
                try:
                    if not _peer_is_owner(connection):
                        raise HandoffError("Local pairing peer is not this user")
                    request = broker.reserve(_read_request(connection))
                    connection.sendall(b'{"status":"prompting"}\n')
                except (HandoffError, OSError, ValueError):
                    try:
                        connection.sendall(b'{"status":"rejected"}\n')
                    except OSError:
                        pass
            if request is not None:
                try:
                    if not broker.decide(request, deadline - monotonic()):
                        print("Local pairing approval was canceled or unavailable", file=sys.stderr)
                except Exception:
                    print("Local pairing approval was refused by the gateway", file=sys.stderr)
                    operator_feedback()
    finally:
        listener.close()
        try:
            current = path.lstat()
            if socket_identity is not None and (current.st_dev, current.st_ino) == (
                socket_identity.st_dev, socket_identity.st_ino
            ) and stat.S_ISSOCK(current.st_mode):
                path.unlink()
        except FileNotFoundError:
            pass
        try:
            current = directory.lstat()
            if (current.st_dev, current.st_ino) == (
                directory_identity.st_dev, directory_identity.st_ino
            ) and stat.S_ISDIR(current.st_mode):
                directory.rmdir()
        except (FileNotFoundError, OSError):
            pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    command = parser.add_subparsers(dest="command", required=True)
    run = command.add_parser("serve")
    run.add_argument("--socket-dir", type=Path, required=True)
    run.add_argument("--origin", required=True)
    run.add_argument("--operator-token-file", type=Path, required=True)
    args = parser.parse_args()
    try:
        if sys.platform != "darwin":
            raise HandoffError("Local Operator approval requires macOS")
        broker = ApprovalBroker(args.origin, args.operator_token_file)
        interrupted = False

        def stop_handler(_signal, _frame):
            nonlocal interrupted
            interrupted = True

        signal.signal(signal.SIGTERM, stop_handler)
        signal.signal(signal.SIGINT, stop_handler)
        serve(args.socket_dir, broker, stop=lambda: interrupted)
        return 0
    except (HandoffError, PairingError, OSError, ValueError) as error:
        print(f"Local pairing approval unavailable: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
