"""Offline operator approval boundary tests; no real credentials or gateway."""
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import urllib.error

import fleetworks_pairing as pairing


class Response:
    status = 200
    def __init__(self, payload): self.payload = payload
    def __enter__(self): return self
    def __exit__(self, *args): return False
    def read(self, limit): return self.payload[:limit]


class PairingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.token = Path(self.directory.name) / "operator-token"
        self.token.write_text("synthetic-operator-0123456789abcdef")
        self.token.chmod(0o600)

    def test_approval_sends_only_challenge_and_keeps_operator_secret_out_of_result(self):
        opener = mock.Mock()
        opener.open.return_value = Response(b'{"status":"approved"}')
        with mock.patch.object(pairing.urllib.request, "build_opener", return_value=opener) as factory:
            result = pairing.perform("https://gateway.example/", self.token, "approve", "a" * 32, "b" * 32)
        request = opener.open.call_args.args[0]
        self.assertEqual(json.loads(request.data), {"pairing_id": "a" * 32, "approval_code": "b" * 32})
        self.assertEqual(request.full_url, "https://gateway.example/api/v1/admin/pairing/approve")
        self.assertEqual(request.get_header("Authorization"), "Bearer " + self.token.read_text())
        self.assertEqual(result, {"status": "approved"})
        self.assertIsInstance(factory.call_args.args[0], pairing.urllib.request.ProxyHandler)
        self.assertEqual(factory.call_args.args[0].proxies, {})
        self.assertIsInstance(factory.call_args.args[1], pairing.NoRedirect)

    def test_denial_and_revocation_have_no_client_authority_fields(self):
        for action, reference, code, expected in [("deny", "a" * 32, "b" * 32, {"pairing_id": "a" * 32, "approval_code": "b" * 32}), ("revoke", "paired-" + "c" * 64, None, {"credential_id": "paired-" + "c" * 64})]:
            opener = mock.Mock()
            opener.open.return_value = Response(json.dumps({"status": "denied" if action == "deny" else "revoked"}).encode())
            with mock.patch.object(pairing.urllib.request, "build_opener", return_value=opener):
                pairing.perform("http://127.0.0.1:18091", self.token, action, reference, code)
            self.assertEqual(json.loads(opener.open.call_args.args[0].data), expected)

    def test_private_file_rejects_symlink_hardlink_and_world_readable_file(self):
        self.token.chmod(0o644)
        with self.assertRaises(pairing.PairingError): pairing.read_operator_token(self.token)
        self.token.chmod(0o600)
        link = self.token.parent / "link"
        link.symlink_to(self.token)
        with self.assertRaises(pairing.PairingError): pairing.read_operator_token(link)
        link.unlink()
        os.link(self.token, link)
        with self.assertRaises(pairing.PairingError): pairing.read_operator_token(self.token)

    def test_origin_rejects_credentials_paths_remote_http_and_redirects(self):
        for value in ["http://gateway.example", "https://user:secret@example.com", "https://example.com/api", "https://example.com?token=secret", "https://example.com#secret"]:
            with self.assertRaises(pairing.PairingError): pairing.origin(value)
        with self.assertRaises(pairing.PairingError): pairing.NoRedirect().redirect_request(None, None, 302, "", {}, "https://elsewhere.example")

    def test_failures_and_malformed_or_oversized_responses_never_echo_body_or_secret(self):
        for response in [Response(b'{"access_token":"private"}'), Response(b"private" * 4000), urllib.error.HTTPError("https://gateway.example", 403, self.token.read_text(), {}, None)]:
            opener = mock.Mock()
            if isinstance(response, Exception): opener.open.side_effect = response
            else: opener.open.return_value = response
            with mock.patch.object(pairing.urllib.request, "build_opener", return_value=opener):
                with self.assertRaises(pairing.PairingError) as error:
                    pairing.perform("https://gateway.example", self.token, "approve", "a" * 32, "b" * 32)
            self.assertNotIn("private", str(error.exception))
            self.assertNotIn(self.token.read_text(), str(error.exception))

    def test_cli_success_and_safe_failure(self):
        with mock.patch.object(pairing, "perform", return_value={"status": "approved"}), contextlib.redirect_stdout(io.StringIO()) as output:
            status = pairing.main(["--origin", "https://gateway.example", "--operator-token-file", str(self.token), "approve", "--pairing-id", "a" * 32, "--approval-code", "b" * 32])
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(output.getvalue()), {"status": "approved"})
        with contextlib.redirect_stderr(io.StringIO()) as output:
            status = pairing.main(["--origin", "http://remote.example", "--operator-token-file", str(self.token), "revoke", "--credential-id", "paired-" + "c" * 64])
        self.assertEqual(status, 1)
        self.assertNotIn(self.token.read_text(), output.getvalue())


if __name__ == "__main__": unittest.main()
