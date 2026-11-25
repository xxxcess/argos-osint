"""Offline regression coverage for the packaged workstation benchmark."""

import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import fleetworks_workstation_bench as bench


class WorkstationBenchTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.app = self.root / "Fleetworks"
        self.binary = self.app / "bin/argos-api-host"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"fixture companion")
        self.binary.chmod(0o700)
        self.manifest = self.app / "share/fleetworks/fleetworks-manifest.json"
        self.manifest.parent.mkdir(parents=True)
        self.digest = bench.pack.sha256(self.binary)
        self.data = {"target": "x86_64-unknown-linux-gnu", "revision": "a" * 40, "dirty": False,
                     "files": {"bin/argos-api-host": self.digest}}
        self.manifest.write_text(json.dumps(self.data), encoding="utf-8")
        for attribute, value in (("system", "Linux"), ("machine", "x86_64")):
            patch = mock.patch.object(bench.platform, attribute, return_value=value)
            patch.start()
            self.addCleanup(patch.stop)

    def test_companion_digest_is_checked_before_launch(self):
        binary, _, digest = bench.verify_companion(self.app)
        self.assertEqual(binary, self.binary)
        self.assertEqual(digest, self.digest)
        self.binary.write_bytes(b"changed companion")
        args = SimpleNamespace(app=self.app)
        with mock.patch.object(bench, "run_iteration") as run:
            with self.assertRaisesRegex(ValueError, "Companion SHA-256"):
                bench.benchmark(args)
            run.assert_not_called()

    def test_manifest_must_match_host_and_be_bounded_and_unlinked(self):
        for target in ("aarch64-apple-darwin", "aarch64-unknown-linux-gnu"):
            self.manifest.write_text(json.dumps({**self.data, "target": target}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "does not match the benchmark host"):
                bench.verify_companion(self.app)
        self.manifest.write_bytes(b" " * (bench.MAX_APP_MANIFEST_BYTES + 1))
        with self.assertRaisesRegex(ValueError, "bounded regular file"):
            bench.verify_companion(self.app)
        outside = self.root / "linked.json"
        outside.write_text(json.dumps(self.data), encoding="utf-8")
        self.manifest.unlink()
        self.manifest.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "unlinked paths"):
            bench.verify_companion(self.app)

    def test_operational_replies_must_copy_entire_passages(self):
        def finding(key, text):
            return {"source": {"kind": "readiness", "id": key}, "text": text,
                    "observed_at": "2026-11-25T18:00:00Z", "start": 0, "end": len(text)}

        catalog = "Actor catalog is not ready. No completed seed was verified."
        gateway = "Gateway is ready."
        turn = {"state": "answered", "findings": [finding("actor_catalog", catalog), finding("gateway", gateway)],
                "reply": f"Here are the current checks. {catalog} {gateway}"}
        oracle = {"checks": {"actor_catalog": "not_ready", "gateway": "ready"}}
        self.assertEqual(bench.factual_errors(turn, "overview", oracle, None), [])
        turn["reply"] = "Gateway is ready. No completed seed was verified."
        self.assertIn("published reply differs from its complete cited passages",
                      bench.factual_errors(turn, "overview", oracle, None))

    def test_marker_spacing_and_decimal_sentence_boundaries(self):
        for text in ("Earlier  user", "Earlier\tassistant", "Current\nuser"):
            self.assertTrue(bench.contains_prompt_marker(text), text)
        self.assertEqual(bench.sentences("CPU is 25.5%. What now? Hello!"),
                         ["cpu is 25.5%", "what now", "hello"])


if __name__ == "__main__":
    unittest.main()
