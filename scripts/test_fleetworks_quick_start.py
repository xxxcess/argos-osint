"""Exercise the Quick Start pack admission path with offline Actions fixtures."""

from contextlib import contextmanager
from io import BytesIO
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

import fleetworks_quick_start as launcher


class InboxArtifactTest(unittest.TestCase):
    def exercise_install(self, target, *, smoke_attempt=1, smoke_digest=None, provide_smoke=True):
        revision = "a" * 40
        run_id = 23456
        run = {"id": run_id, "head_sha": revision, "status": "completed", "conclusion": "failure",
               "path": ".github/workflows/fleetworks.yml", "run_attempt": smoke_attempt}
        run_info = {"id": run_id, "head_sha": revision, "head_repository_id": 23, "repository_id": 23}
        receipt = {"target": target, "source_sha": revision, "run_id": str(run_id), "run_attempt": 1,
                   "archive_bytes": 4, "archive_sha256": "b" * 64, "manifest_sha256": "c" * 64}
        report = {"target": target, "source_sha": revision, "runtime": "embedded-unix-socket",
                  "transport_attestation": {"transport": "embedded-unix-socket", "socket_family": "unix",
                                            "socket_type": "stream", "protocol_version": 2,
                                            "min_protocol_version": 2, "dataset_upload_chunk_bytes": 262144,
                                            "dataset_upload_window": 4},
                  "terminal_state": "answered", "terminal_events": 1, "model_pid_count": 1,
                  "cancelled_terminal_state": "canceled", "cancelled_terminal_events": 1,
                  "generated_dialogue": True}
        pack = {"id": 1, "workflow_run": run_info, "expired": False, "digest": "sha256:" + "d" * 64,
                "name": f"Fleetworks-Inbox-QA-Pack-{target}-{revision}-{run_id}.1"}
        smoke = {**pack, "id": 2,
                 "name": f"Fleetworks-Inbox-QA-Embedded-{target}-{revision}-{run_id}.{smoke_attempt}"}
        endpoints = {"actions/artifacts/1": pack, "actions/artifacts/2": smoke, f"actions/runs/{run_id}": run}

        def gh_json(endpoint):
            if endpoint.startswith(f"actions/runs/{run_id}/artifacts?name="):
                return {"artifacts": [smoke] if provide_smoke and endpoint.endswith(smoke["name"]) else []}
            return endpoints[endpoint]

        @contextmanager
        def download(artifact, temporary, limit):
            with BytesIO() as stream:
                with ZipFile(stream, "w") as outer:
                    if artifact == smoke:
                        smoke_receipt = receipt if smoke_digest is None else {**receipt, "archive_sha256": smoke_digest}
                        outer.writestr(f"pack-digests-{target}.json", json.dumps(smoke_receipt))
                        outer.writestr(f"embedded-{target}.json", json.dumps(report))
                    else:
                        outer.writestr(f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz", b"pack")
                        outer.writestr("digests.json", json.dumps(receipt))
                with ZipFile(stream) as outer:
                    yield outer

        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary).resolve()
            state = profile / "state"
            state.mkdir(mode=0o700)
            with patch.object(launcher, "gh_json", gh_json), patch.object(launcher, "download_artifact", download), \
                    patch.object(launcher, "build_inputs_match", return_value=True), \
                    patch.object(launcher.qa, "install") as install:
                with self.assertRaisesRegex(ValueError, "successful Fleetworks run"):
                    launcher.artifact_metadata(1, target, ("Fleetworks-Inbox-QA-Pack",))
                try:
                    launcher.install_inbox(1, target, profile, state)
                except ValueError:
                    install.assert_not_called()
                    raise
                install.assert_called_once()
                admitted = install.call_args.args[0]
                self.assertEqual(admitted.target, target)
                self.assertEqual(admitted.state_dir, state)
                self.assertEqual(admitted.archive_sha256, receipt["archive_sha256"])
                self.assertEqual(admitted.manifest_sha256, receipt["manifest_sha256"])

    def test_verified_platform_pack_can_install_when_another_ci_job_failed(self):
        for target in ("x86_64-apple-darwin", "aarch64-apple-darwin"):
            with self.subTest(target=target):
                self.exercise_install(target)

    def test_verified_rerun_can_use_pack_from_an_earlier_attempt(self):
        self.exercise_install("x86_64-apple-darwin", smoke_attempt=2)

    def test_pack_without_platform_smoke_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "No verified Embedded Inbox smoke receipt"):
            self.exercise_install("x86_64-apple-darwin", provide_smoke=False)

    def test_pack_with_different_smoke_digest_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "do not match the verified Embedded smoke"):
            self.exercise_install("x86_64-apple-darwin", smoke_digest="e" * 64)


if __name__ == "__main__":
    unittest.main()
