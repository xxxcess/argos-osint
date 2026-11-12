#!/usr/bin/env python3
"""Synthetic disposable cutover fixtures; never touch a user's installation."""
import copy
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

import fleetworks_config_reset as reset


class FakeBackend:
    def __init__(self, *, available=True, legacy=True, finalize=True):
        self.available = available
        self.legacy = legacy
        self.finalize = finalize
        self.requests = []

    def request(self, phase, payload):
        self.requests.append((phase, payload))
        if not self.available or phase == "finalize" and not self.finalize:
            raise reset.Pending("Synthetic authoritative seam unavailable")
        return {"version": 1, "reset_id": payload["reset_id"], "plan_sha256": payload["plan_sha256"],
                "deployment_id": payload["deployment_id"], "scope_id": "fixture-scope", "state": "complete" if phase == "finalize" else "prepared",
                "admission_fenced": phase != "finalize", "first_use_blocked": phase != "finalize", "work_drained": True,
                "user_access_revoked": True, "sessions_revoked": True, "configuration_epoch": "reset-configuration-1",
                "access_epoch": "reset-access-1", "backend_configuration_cleared": phase != "prepare", "legacy_reload_disabled": self.legacy}


class ResetFixtures(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fleetworks-reset-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / ".argos"
        self.ledger = self.base / "protected-ledger"
        self.root.mkdir(mode=0o700)
        self.ledger.mkdir(mode=0o700)
        self.secret = "synthetic-old-provider-secret-do-not-print"
        self.old = {"config.toml": f'configuration_format=1\nfirecrawl_api_key="{self.secret}"\n',
                    "auth.json": json.dumps({"accounts": {"fixture": {"api_key": self.secret}}}), "quota.json": '{"settings":[]}'}
        for name, body in self.old.items():
            self.write(name, body)
        self.protected = self.root / "default"
        self.protected.mkdir(mode=0o700)
        (self.protected / "notes.txt").write_text("preserved analyst work")
        (self.root / "actors.db").write_bytes(b"synthetic retained domain data")
        (self.root / ".native-state.json").write_text('{"installation_marker":"retained-marker"}')
        self.selected = {"version": 1, "reset_id": "synthetic-cutover-1", "ledger": str(self.ledger), "secure_store": [],
                         "backend": {"origin": "https://gateway.example.test", "deployment_id": "fixture-deployment", "scope_id": "fixture-scope",
                                     "expected_configuration_epoch": "old-configuration", "expected_access_epoch": "old-access"},
                         "targets": [{"name": "explicit-legacy-home", "root": str(self.root), "files": list(self.old)}]}

    def write(self, name, body):
        path = self.root / name
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        path.write_text(body)
        path.chmod(0o600)
        return path

    def plan(self):
        return reset.inventory(copy.deepcopy(self.selected))

    def apply(self, plan, backend=None, **options):
        return reset.apply(plan, backend or FakeBackend(), ["explicit-legacy-home"], **options)

    def journal(self):
        return json.loads((self.ledger / "reset-synthetic-cutover-1.json").read_text())

    def test_inventory_is_read_only_nonsecret_and_records_frozen_objects(self):
        before = {name: (self.root / name).read_bytes() for name in self.old}
        plan = self.plan()
        self.assertNotIn(self.secret, json.dumps(plan))
        self.assertEqual(before, {name: (self.root / name).read_bytes() for name in self.old})
        self.assertFalse(list(self.ledger.iterdir()))
        self.assertEqual(len(plan["entries"]), 3)
        self.assertTrue(all(entry["before"]["sha256"] and entry["root_chain"] for entry in plan["entries"]))

    def test_cli_inventory_records_a_private_plan_without_payload_output(self):
        manifest = self.base / "reviewed-manifest.json"
        manifest.write_bytes(reset.encoded(self.selected))
        manifest.chmod(0o600)
        plan = self.ledger / "frozen-plan.json"
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = reset.main(["inventory", "--manifest", str(manifest), "--out", str(plan)])
        self.assertEqual(code, 0)
        self.assertFalse(json.loads(output.getvalue())["applied"])
        self.assertNotIn(self.secret, output.getvalue())
        self.assertEqual(plan.stat().st_mode & 0o777, 0o600)
        self.assertIn(self.secret, (self.root / "auth.json").read_text())

    def test_missing_backend_seam_cannot_erase_a_local_value(self):
        with self.assertRaises(reset.Pending):
            self.apply(self.plan(), FakeBackend(available=False))
        for name, body in self.old.items():
            self.assertEqual((self.root / name).read_text(), body)
        self.assertEqual(self.journal()["state"], "planned")

    def test_complete_is_empty_current_format_and_repeating_preserves_new_keys(self):
        plan = self.plan()
        backend = FakeBackend()
        self.assertEqual(self.apply(plan, backend)["state"], "complete")
        config = (self.root / "config.toml").read_text()
        self.assertTrue(config.startswith("configuration_format = 1\n"))
        self.assertEqual(config.count("[defaults."), 6)
        for role in reset.ROLES:
            self.assertIn(f'[defaults.{role}]\nprovider = ""\nmodel = ""\naccount = ""\nfallbacks = []', config)
        self.assertEqual(json.loads((self.root / "auth.json").read_text()), {"configuration_format": 1, "accounts": {}})
        self.assertEqual((self.protected / "notes.txt").read_text(), "preserved analyst work")
        self.assertEqual((self.root / "actors.db").read_bytes(), b"synthetic retained domain data")
        self.assertIn("retained-marker", (self.root / ".native-state.json").read_text())
        self.write("auth.json", '{"configuration_format":1,"accounts":{"new":{"api_key":"new-synthetic-key"}}}')
        denied = FakeBackend(available=False)
        self.assertTrue(self.apply(plan, denied)["repeated"])
        self.assertEqual(denied.requests, [])
        self.assertIn("new-synthetic-key", (self.root / "auth.json").read_text())
        self.assertNotIn(self.secret, (self.ledger / "reset-synthetic-cutover-1.json").read_text())
        self.assertEqual((self.ledger / "reset-synthetic-cutover-1.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual([phase for phase, _ in backend.requests], ["prepare", "complete", "finalize"])

    def test_interrupted_reset_resumes_only_frozen_unfinished_steps(self):
        plan = self.plan()
        def stop(index):
            raise RuntimeError("synthetic interruption")
        with self.assertRaises(RuntimeError):
            self.apply(plan, after_step=stop)
        self.assertEqual(self.journal()["done"], [0])
        self.write("config.toml", 'configuration_format=1\nfirecrawl_api_key="new-after-interruption"\n')
        self.apply(plan)
        self.assertIn("new-after-interruption", (self.root / "config.toml").read_text())
        self.assertNotIn(self.secret, (self.root / "auth.json").read_text())

    def test_journaled_partial_empty_write_is_recoverable_without_secret_backup(self):
        plan = self.plan()
        with self.assertRaises(reset.Pending):
            self.apply(plan, FakeBackend(available=False))
        journal = self.journal()
        journal["intent"] = 0
        with reset.directory(self.ledger, private=True) as (fd, _):
            reset.write_json(fd, "reset-synthetic-cutover-1.json", journal)
        self.write("config.toml", reset.FRESH_CONFIG[:25].decode())
        self.apply(plan)
        self.assertEqual((self.root / "config.toml").read_bytes(), reset.FRESH_CONFIG)
        self.assertFalse(list(self.root.glob("*.bak")))

    def test_file_change_and_inode_substitution_fail_closed(self):
        for substitute in (False, True):
            with self.subTest(substitute=substitute):
                plan = self.plan()
                if substitute:
                    replacement = self.root / "new-owned-file"
                    replacement.write_bytes((self.root / "auth.json").read_bytes())
                    replacement.chmod(0o600)
                    replacement.replace(self.root / "auth.json")
                else:
                    self.write("auth.json", '{"accounts":{"new":{"api_key":"new-before-apply"}}}')
                with self.assertRaises(reset.ResetError):
                    self.apply(plan)
                self.assertIn(self.secret, (self.root / "config.toml").read_text())
                (self.ledger / "reset-synthetic-cutover-1.json").unlink()

    def test_changed_root_and_symlink_or_hardlink_cannot_be_adopted(self):
        plan = self.plan()
        moved = self.base / "moved-owned-root"
        self.root.rename(moved)
        self.root.mkdir(mode=0o700)
        with self.assertRaises(reset.ResetError):
            self.apply(plan)
        self.root.rmdir()
        moved.rename(self.root)
        (self.ledger / "reset-synthetic-cutover-1.json").unlink()
        (self.root / "auth.json").unlink()
        (self.root / "auth.json").symlink_to(self.protected / "notes.txt")
        with self.assertRaises((reset.ResetError, OSError)):
            self.plan()
        (self.root / "auth.json").unlink()
        os.link(self.protected / "notes.txt", self.root / "auth.json")
        with self.assertRaises(reset.ResetError):
            self.plan()
        self.assertEqual((self.protected / "notes.txt").read_text(), "preserved analyst work")

    def test_forged_plan_entries_and_protected_directories_are_not_an_allowlist(self):
        for path in ("default", "../auth.json", ".native-state.json", "storage", "secrets/jobs-token", ".argos-config-journal.json"):
            selected = copy.deepcopy(self.selected)
            selected["targets"][0]["files"] = [path]
            with self.assertRaises(reset.ResetError):
                reset.inventory(selected)
        plan = self.plan()
        plan["entries"][0]["root"] = str(self.base)
        plan.pop("plan_sha256")
        plan["plan_sha256"] = reset.sha(reset.encoded(plan))
        with self.assertRaises(reset.ResetError):
            self.apply(plan)

    def test_mixed_environment_and_unverified_exports_require_review(self):
        self.write("secrets/environment.toml", '[environment]\nOPENAI_API_KEY="synthetic-user"\nFLEETWORKS_JOBS_TOKEN="synthetic-infrastructure"\n')
        self.write("exports/unrelated.json", '{"unrelated":"retained-data"}')
        self.selected["targets"][0]["files"] += ["secrets/environment.toml", "exports/unrelated.json"]
        plan = self.plan()
        self.assertEqual(len(plan["blockers"]), 2)
        self.assertNotIn("synthetic-infrastructure", json.dumps(plan))
        with self.assertRaises(reset.Pending):
            self.apply(plan)
        self.assertIn("synthetic-infrastructure", (self.root / "secrets/environment.toml").read_text())

    def test_exact_portable_export_backup_and_user_token_payloads_are_emptied(self):
        self.write("exports/profile.json", json.dumps({"schema_version": 4, "providers": [{"api_key": self.secret}], "tool_credentials": [], "model_roles": {}, "rate_limits": []}))
        self.write("auth.json.7.bak", self.secret)
        self.write("secrets/api-token", "synthetic-user-gateway-credential")
        names = ["exports/profile.json", "auth.json.7.bak", "secrets/api-token"]
        self.selected["targets"][0]["files"] += names
        self.apply(self.plan())
        self.assertTrue(all((self.root / name).read_bytes() == b"" for name in names))
        self.assertTrue(all((self.root / name).exists() for name in names))

    def test_legacy_reader_and_finalization_gates_are_separate_recoverable_authorities(self):
        plan = self.plan()
        backend = FakeBackend(legacy=False)
        with self.assertRaises(reset.Pending):
            self.apply(plan, backend)
        self.assertEqual(self.journal()["state"], "files_reset")
        self.assertNotIn("finalize", [phase for phase, _ in backend.requests])
        backend.legacy = True
        backend.finalize = False
        with self.assertRaises(reset.Pending):
            self.apply(plan, backend)
        self.assertEqual(self.journal()["state"], "local_committed")
        self.write("auth.json", '{"accounts":{"new":{"api_key":"new-while-finalization-pending"}}}')
        backend.requests.clear()
        backend.finalize = True
        self.apply(plan, backend)
        self.assertEqual([phase for phase, _ in backend.requests], ["finalize"])
        self.assertIn("new-while-finalization-pending", (self.root / "auth.json").read_text())

    def test_unknown_secure_store_and_insecure_remote_administration_are_rejected(self):
        selected = copy.deepcopy(self.selected)
        selected["secure_store"] = [{"service": "invented-general-keychain-item"}]
        with self.assertRaises(reset.ResetError):
            reset.inventory(selected)
        for origin in ("http://remote.example.test", "https://user:secret@host.test", "https://host.test/not-an-origin"):
            with self.assertRaises(reset.ResetError):
                reset.validate_origin(origin)

    def test_pending_configuration_recovery_and_unlisted_backups_block_reset(self):
        self.write(".argos-config-journal.json", '{"state":"pending","entries":[]}')
        self.write("auth.json.8.bak", self.secret)
        plan = self.plan()
        self.assertEqual(len(plan["blockers"]), 2)
        with self.assertRaises(reset.Pending):
            self.apply(plan)
        self.assertIn(self.secret, (self.root / "auth.json").read_text())

    def test_nonnative_boolean_fence_and_admin_credential_payload_are_rejected(self):
        class BadReceipt(FakeBackend):
            def request(self, phase, payload):
                response = super().request(phase, payload)
                response["work_drained"] = 1
                return response
        with self.assertRaises(reset.Pending):
            self.apply(self.plan(), BadReceipt())
        (self.ledger / "reset-synthetic-cutover-1.json").unlink()
        self.write("secrets/api-token", "synthetic-independent-admin-credential-1234567890")
        self.selected["targets"][0]["files"].append("secrets/api-token")
        backend = reset.Backend("http://127.0.0.1:1", self.root / "secrets/api-token")
        with self.assertRaises(reset.ResetError):
            self.apply(self.plan(), backend)

    def test_reset_epochs_cannot_advance_again_on_resume(self):
        plan = self.plan()
        def stop(_):
            raise RuntimeError("synthetic interruption")
        with self.assertRaises(RuntimeError):
            self.apply(plan, after_step=stop)
        class ChangedEpoch(FakeBackend):
            def request(self, phase, payload):
                receipt = super().request(phase, payload)
                receipt["configuration_epoch"] = "incorrect-second-epoch"
                return receipt
        with self.assertRaises(reset.Pending):
            self.apply(plan, ChangedEpoch())
        self.assertIn(self.secret, (self.root / "auth.json").read_text())

    def test_real_http_adapter_missing_authoritative_seam_preserves_payloads(self):
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                requests.append((self.path, self.headers.get("Authorization")))
                self.send_response(404)
                self.end_headers()
            def log_message(self, *_):
                pass
        server = HTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            self.selected["backend"]["origin"] = f"http://127.0.0.1:{server.server_port}"
            token = self.base / "independent-admin-token"
            token.write_text("synthetic-independent-admin-credential-1234567890")
            token.chmod(0o600)
            plan = self.ledger / "frozen-plan.json"
            plan.write_bytes(reset.encoded(self.plan()))
            plan.chmod(0o600)
            error = io.StringIO()
            with contextlib.redirect_stderr(error):
                code = reset.main(["apply", "--plan", str(plan), "--target", "explicit-legacy-home", "--admin-token-file", str(token)])
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(error.getvalue())["state"], "pending")
            self.assertNotIn(self.secret, error.getvalue())
            self.assertEqual(len(requests), 1)
            self.assertEqual(requests[0][0], "/api/v1/admin/configuration-reset/prepare")
            self.assertTrue(requests[0][1].startswith("Bearer synthetic-independent-admin"))
            self.assertIn(self.secret, (self.root / "auth.json").read_text())
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
