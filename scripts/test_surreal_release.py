"""Offline checks for the SurrealDB release manifest and fetch-only contract."""

import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock

import test_surreal as harness


class SurrealReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.cache = Path(temporary.name)
        self.release = harness.load_release()

    def test_cached_binary_is_selected_for_each_supported_host(self):
        for host, asset in harness.HOST_ASSETS.items():
            with self.subTest(host=host):
                expected = self.cache / f"{self.release['version']}-{asset}" / "surreal"
                expected.parent.mkdir(parents=True, exist_ok=True)
                expected.write_bytes(b"cached fixture")
                with (
                    mock.patch.object(harness.platform, "system", return_value=host[0]),
                    mock.patch.object(harness.platform, "machine", return_value=host[1]),
                    mock.patch.object(harness.urllib.request, "urlopen") as download,
                ):
                    self.assertEqual(harness.fetch(self.cache, self.release), expected)
                    download.assert_not_called()

    def test_fetch_only_prints_the_binary_path_without_starting_it(self):
        binary = self.cache / "fixture directory" / "surreal"
        stdout = io.StringIO()
        with (
            contextlib.redirect_stdout(stdout),
            mock.patch.object(harness.sys, "argv", [
                "test_surreal.py", "--mode", "server", "--fetch-only", "--bin", str(binary),
            ]),
            mock.patch.object(harness, "smoke") as smoke,
            mock.patch.object(harness, "fetch") as fetch,
        ):
            harness.main()
        self.assertEqual(stdout.getvalue(), f"test-surreal: server binary {binary.resolve()}\n")
        smoke.assert_not_called()
        fetch.assert_not_called()

    def test_required_embedded_phase_executes_cycle_history_with_no_skip_environment(self):
        with (
            contextlib.redirect_stdout(io.StringIO()),
            mock.patch.object(harness.sys, "argv", ["test_surreal.py", "--mode", "embedded"]),
            mock.patch.object(harness.subprocess, "run", return_value=mock.Mock(returncode=0)) as run,
        ):
            harness.main()
        run.assert_called_once()
        command = run.call_args.args[0]
        targets = [command[index + 1] for index, argument in enumerate(command[:-1]) if argument == "--test"]
        self.assertEqual(targets.count("storage_cycle_history"), 1)
        self.assertIn("--locked", command)
        environment = run.call_args.kwargs["env"]
        self.assertEqual(environment["ARGOS_SURREAL_MODE"], "embedded")
        self.assertEqual(environment["ARGOS_SURREAL_REQUIRED"], "1")

    def test_missing_and_duplicate_pins_are_rejected(self):
        assets = self.release["archive_sha256"]
        cases = {
            "missing archive": {key: value for key, value in assets.items() if key != "linux-amd64"},
            "duplicate checksum": dict(assets, **{"linux-amd64": assets["linux-arm64"]}),
            "malformed checksum": dict(assets, **{"linux-amd64": "invalid"}),
        }
        manifest = self.cache / "surrealdb-release.json"
        for case, pins in cases.items():
            with self.subTest(case=case):
                manifest.write_text(
                    json.dumps({**self.release, "archive_sha256": pins}), encoding="utf-8"
                )
                with (
                    mock.patch.object(harness, "RELEASE_PATH", manifest),
                    self.assertRaisesRegex(SystemExit, "invalid SurrealDB release manifest"),
                ):
                    harness.load_release()

    def test_download_verifies_the_archive_before_installing(self):
        file = b"surreal fixture"
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w:gz") as tar:
            member = tarfile.TarInfo("surreal")
            member.size = len(file)
            tar.addfile(member, io.BytesIO(file))
        payload = archive.getvalue()
        asset = "linux-amd64"
        target = self.cache / f"{self.release['version']}-{asset}" / "surreal"
        url = self.release["release_url_template"].format(v=self.release["version"], p=asset)
        with (
            contextlib.redirect_stdout(io.StringIO()),
            mock.patch.object(harness.platform, "system", return_value="Linux"),
            mock.patch.object(harness.platform, "machine", return_value="x86_64"),
            mock.patch.object(
                harness.urllib.request, "urlopen", side_effect=[io.BytesIO(payload), io.BytesIO(payload)]
            ) as download,
        ):
            with self.assertRaisesRegex(SystemExit, "SHA-256 mismatch"):
                harness.fetch(self.cache, self.release)
            self.assertFalse(target.exists())

            self.release["archive_sha256"][asset] = hashlib.sha256(payload).hexdigest()
            self.assertEqual(harness.fetch(self.cache, self.release), target)
            self.assertEqual(target.read_bytes(), file)
            self.assertTrue(os.access(target, os.X_OK))
            self.assertEqual(download.call_args_list, [mock.call(url, timeout=300)] * 2)


if __name__ == "__main__":
    unittest.main()
