#!/usr/bin/env python3
"""Approve, deny or revoke native pairing with a private Operator token file.

The native app displays the pairing ID and approval code. Operator credentials
stay in this process; the gateway derives all Native grants from that principal.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request

LIMIT = 16 * 1024
IDENTITY = re.compile(r"[A-Za-z0-9_-]{16,128}\Z")


class PairingError(Exception):
    """Safe, non-secret operator diagnostic."""


def origin(raw: str) -> str:
    try:
        value = urllib.parse.urlsplit(raw)
        port = value.port
    except ValueError as error:
        raise PairingError("Invalid gateway origin") from error
    if (
        value.scheme not in ("https", "http")
        or not value.hostname
        or value.username is not None
        or value.password is not None
        or value.path not in ("", "/")
        or value.query
        or value.fragment
        or port == 0
        or (value.scheme == "http" and value.hostname not in ("localhost", "127.0.0.1", "::1"))
    ):
        raise PairingError("Use an exact HTTPS gateway origin (HTTP is allowed only on loopback)")
    return urllib.parse.urlunsplit((value.scheme, value.netloc, "", "", ""))


def read_operator_token(path: Path) -> str:
    """Read one current-user-owned, single-link private regular file safely."""
    descriptor = None
    try:
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid() or before.st_mode & 0o077 or before.st_nlink != 1:
            raise PairingError("Operator token file must be a private regular file owned by this user")
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino, opened.st_mode, opened.st_uid, opened.st_nlink) != (
            before.st_dev, before.st_ino, before.st_mode, before.st_uid, before.st_nlink
        ):
            raise PairingError("Operator token file changed while opening")
        raw = os.read(descriptor, 4097)
        after = path.lstat()
        if (after.st_dev, after.st_ino) != (opened.st_dev, opened.st_ino) or len(raw) > 4096:
            raise PairingError("Operator token file changed or exceeds its bound")
        token = raw.decode("ascii").strip()
        if not re.fullmatch(r"[A-Za-z0-9._~+/=-]{32,256}", token):
            raise PairingError("Operator token file has invalid credential bytes")
        return token
    except (OSError, UnicodeError) as error:
        raise PairingError("Operator token file is unavailable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise PairingError("Gateway redirects are refused; use the exact configured origin")


def perform(gateway: str, token_file: Path, action: str, reference: str, approval_code: str | None = None) -> dict:
    gateway = origin(gateway)
    if action not in ("approve", "deny", "revoke"):
        raise PairingError("Unsupported pairing operation")
    if action == "revoke":
        if not re.fullmatch(r"paired-[A-Za-z0-9_.-]{1,121}", reference):
            raise PairingError("Invalid Native credential ID")
        payload = {"credential_id": reference}
    else:
        if not IDENTITY.fullmatch(reference) or not IDENTITY.fullmatch(approval_code or ""):
            raise PairingError("Use the exact pairing ID and approval code displayed by the native app")
        payload = {"pairing_id": reference, "approval_code": approval_code}
    token = read_operator_token(token_file)
    request = urllib.request.Request(
        gateway + "/api/v1/admin/pairing/" + action,
        data=json.dumps(payload, separators=(",", ":")).encode(),
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        method="POST",
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    try:
        with opener.open(request, timeout=15) as response:
            raw = response.read(LIMIT + 1)
            if response.status != 200 or len(raw) > LIMIT:
                raise PairingError("Gateway returned an invalid pairing response")
            result = json.loads(raw)
    except urllib.error.HTTPError as error:
        messages = {401: "Operator authentication was refused", 403: "Operator purpose, scope or approval is not permitted", 409: "Pairing conflicts with retained state", 410: "Pairing expired; start again in the native app", 429: "Pairing is rate limited; retry later"}
        raise PairingError(messages.get(error.code, "Gateway pairing operation failed")) from error
    except (urllib.error.URLError, TimeoutError, OSError, ValueError) as error:
        raise PairingError("Gateway pairing response is unavailable") from error
    expected = {"approve": "approved", "deny": "denied", "revoke": "revoked"}[action]
    if result != {"status": expected}:
        raise PairingError("Gateway returned an invalid pairing response")
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True)
    parser.add_argument("--operator-token-file", required=True, type=Path)
    commands = parser.add_subparsers(dest="action", required=True)
    for action in ("approve", "deny"):
        command = commands.add_parser(action)
        command.add_argument("--pairing-id", required=True)
        command.add_argument("--approval-code", required=True)
    commands.add_parser("revoke").add_argument("--credential-id", required=True)
    args = parser.parse_args(argv)
    try:
        result = perform(args.origin, args.operator_token_file, args.action, args.credential_id if args.action == "revoke" else args.pairing_id, getattr(args, "approval_code", None))
    except PairingError as error:
        print(str(error), file=sys.stderr)
        return 1
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
