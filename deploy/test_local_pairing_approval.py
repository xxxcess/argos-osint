"""Offline local Operator handoff contracts; no GUI, gateway or token read."""

import json
from pathlib import Path
import socket
import stat
import tempfile
import threading
import unittest
from unittest import mock

import local_pairing_approval as approval


ORIGIN = "http://127.0.0.1:18092"
PAIRING_ID = "a" * 32
CODE = "b" * 32


class LocalPairingApprovalTests(unittest.TestCase):
    def broker(self, *, prompt=None, perform=None):
        return approval.ApprovalBroker(
            ORIGIN,
            Path("/private/operator-token"),
            prompt=prompt or (lambda _request, _timeout: CODE),
            perform=perform or (lambda *_args: {"status": "approved"}),
            clock_ms=lambda: 1_000_000,
            monotonic=lambda: 1_000.0,
        )

    def payload(self, pairing_id=PAIRING_ID, **changes):
        value = {"version": 1, "origin": ORIGIN, "pairing_id": pairing_id,
                 "expires_at_ms": 1_300_000}
        value.update(changes)
        return value

    def test_exact_origin_identity_deadline_and_fields(self):
        broker = self.broker()
        for origin in ("https://gateway.example", "https://127.0.0.1:18092",
                       "http://gateway.example"):
            with self.subTest(origin=origin), self.assertRaises(approval.HandoffError):
                approval.ApprovalBroker(origin, Path("/private/operator-token"))
        for change in (
            {"origin": "https://elsewhere.example"},
            {"origin": "http://127.0.0.1:18093"},
            {"pairing_id": "bad"},
            {"expires_at_ms": 1_000_000},
            {"expires_at_ms": 1_700_000},
            {"version": True},
            {"expires_at_ms": True},
        ):
            with self.subTest(change=change), self.assertRaises(approval.HandoffError):
                broker.reserve(self.payload(**change))
        with self.assertRaises(approval.HandoffError):
            broker.reserve({**self.payload(), "approval_code": CODE})
        request = broker.reserve(self.payload(origin=ORIGIN + "/"))
        self.assertEqual(request, approval.ApprovalRequest(ORIGIN, PAIRING_ID, 1_300_000))

    def test_dedup_and_prompt_rate_are_bounded(self):
        tick = [1000.0]
        broker = approval.ApprovalBroker(
            ORIGIN, Path("/private/operator-token"),
            clock_ms=lambda: 1_000_000, monotonic=lambda: tick[0],
        )
        broker.reserve(self.payload())
        with self.assertRaises(approval.HandoffError):
            broker.reserve(self.payload())
        with self.assertRaises(approval.HandoffError):
            broker.reserve(self.payload("c" * 32))
        for character in "cdefg":
            tick[0] += 31
            broker.reserve(self.payload(character * 32))
        tick[0] += 31
        with self.assertRaises(approval.HandoffError):
            broker.reserve(self.payload("h" * 32))

    def test_only_explicit_typed_code_reaches_operator_client(self):
        decisions = []

        def perform(*arguments):
            decisions.append(arguments)
            return {"status": "approved"}

        broker = self.broker(prompt=lambda _request, _timeout: CODE, perform=perform)
        request = broker.reserve(self.payload())
        self.assertTrue(broker.decide(request, 120))
        self.assertEqual(decisions, [(ORIGIN, Path("/private/operator-token"),
                                      "approve", PAIRING_ID, CODE)])
        for typed in (None, "wrong", "Z" * 32):
            with self.subTest(typed=typed):
                cancelled = self.broker(prompt=lambda _request, _timeout: typed,
                                        perform=perform)
                self.assertFalse(cancelled.decide(request, 120))
        self.assertEqual(len(decisions), 1)

    def test_prompt_passes_no_code_or_bearer_in_process_arguments(self):
        completed = mock.Mock(returncode=0, stdout=CODE + "\n")
        request = approval.ApprovalRequest(ORIGIN, PAIRING_ID, 1_300_000)
        with mock.patch.object(approval.sys, "platform", "darwin"), \
             mock.patch.object(approval.subprocess, "run", return_value=completed) as run:
            self.assertEqual(approval.operator_prompt(request, 60), CODE)
        command = run.call_args.args[0]
        self.assertEqual(command, ["osascript", "-", ORIGIN, PAIRING_ID])
        self.assertNotIn(CODE, " ".join(command))
        self.assertIn("with hidden answer", run.call_args.kwargs["input"])

    def test_private_socket_acknowledges_before_prompt_and_cleans_up(self):
        prompted = threading.Event()
        stopped = threading.Event()
        ready = threading.Event()
        paths = []
        approvals = []

        def prompt(request, _timeout):
            prompted.set()
            return CODE

        def perform(*arguments):
            approvals.append(arguments)
            return {"status": "approved"}

        with tempfile.TemporaryDirectory(prefix="fw-", dir="/tmp") as parent:
            socket_dir = Path(parent) / "review-operator"
            broker = self.broker(prompt=prompt, perform=perform)
            worker = threading.Thread(
                target=approval.serve,
                args=(socket_dir, broker),
                kwargs={"ready": lambda path: (paths.append(path), ready.set()),
                        "stop": stopped.is_set, "lifetime_seconds": 30},
                daemon=True,
            )
            worker.start()
            self.assertTrue(ready.wait(2))
            self.assertEqual(stat.S_IMODE(socket_dir.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(paths[0].stat().st_mode), 0o600)
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(2)
                client.connect(str(paths[0]))
                client.sendall(json.dumps(self.payload()).encode() + b"\n")
                self.assertEqual(json.loads(client.recv(128)), {"status": "prompting"})
            self.assertTrue(prompted.wait(2))
            stopped.set()
            worker.join(3)
            self.assertFalse(worker.is_alive())
            self.assertFalse(socket_dir.exists())
            self.assertEqual(len(approvals), 1)


if __name__ == "__main__":
    unittest.main()
