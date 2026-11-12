"""Offline credential/provisioning contracts; no Docker or real workspace."""

import json
import os
import stat
import tempfile
import unittest
from pathlib import Path

from native_review import TOKEN_NAMES, expose_root_environment, provision, review_path


class NativeReviewTests(unittest.TestCase):
    def test_root_environment_is_private_and_review_alias_follows_atomic_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            target = root / "compose.env"
            alias = Path(receipt["workspace"]) / "compose.env"
            self.assertTrue(target.is_file())
            self.assertFalse(target.is_symlink())
            self.assertTrue(alias.is_symlink())
            self.assertEqual(alias.resolve(), target.resolve())
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
            replacement = root / "replacement.env"
            replacement.write_text(target.read_text().replace("'18091'", "'18093'"))
            replacement.chmod(0o600)
            replacement.replace(target)
            self.assertIn("FLEETWORKS_API_PORT='18093'", alias.read_text())

    def test_existing_root_environment_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "compose.env"
            target.write_text("existing operator settings\n")
            with self.assertRaises(FileExistsError):
                provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            self.assertEqual(target.read_text(), "existing operator settings\n")
            self.assertFalse(review_path(root, "local").exists())
            target.unlink()
            target.symlink_to(root / "missing.env")
            with self.assertRaises(FileExistsError):
                provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            self.assertTrue(target.is_symlink())
            self.assertFalse(review_path(root, "local").exists())

    def test_existing_review_can_expose_settings_without_changing_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = provision(root, "local", 18091, "linux/amd64", "a" * 40)
            workspace = Path(receipt["workspace"])
            before = {path.name: path.read_bytes() for path in (workspace / "secrets").iterdir()}
            original = (workspace / "compose.env").read_bytes()
            exposed = expose_root_environment(root, "local")
            self.assertEqual(exposed.read_bytes(), original)
            self.assertEqual({path.name: path.read_bytes() for path in (workspace / "secrets").iterdir()}, before)
            with self.assertRaises(FileExistsError):
                expose_root_environment(root, "local")
            self.assertEqual(exposed.read_bytes(), original)

    def test_private_distinct_purpose_files_and_stable_restart_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = provision(Path(directory), "fixture", 18091, "linux/arm64", "a" * 40)
            root = Path(receipt["workspace"])
            manifest = json.loads((root / "secrets/gateway-credentials.json").read_text())
            self.assertEqual(manifest["version"], 1)
            self.assertEqual([entry["purpose"] for entry in manifest["credentials"]], ["native", "web_relay", "internal_service"])
            self.assertTrue(all("token" not in entry and entry["token_file"].startswith("/run/secrets/") for entry in manifest["credentials"]))
            web = manifest["credentials"][1]
            self.assertFalse(set(web["permissions"]) & {"register_workstation", "maintain_backend", "manage_configuration", "rebind_workstation"})
            tokens = {(root / "secrets" / name).read_text().strip() for name in TOKEN_NAMES}
            self.assertEqual(len(tokens), len(TOKEN_NAMES))
            env = receipt["environment"]
            self.assertEqual(env["FLEETWORKS_ACCESS_SCOPE_ID"], manifest["scope_id"])
            self.assertEqual(env["FLEETWORKS_ACCESS_EPOCH"], manifest["access_epoch"])
            self.assertTrue(env["FLEETWORKS_SEARCH_SOURCE_EPOCH"])
            self.assertTrue(all(token not in (root / "review.json").read_text() for token in tokens))
            if os.name == "posix":
                for path in [root, root / "secrets", root / "deployment", root / "storage/search-index"]:
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
                for path in (root / "secrets").iterdir():
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            with self.assertRaises(FileExistsError):
                provision(Path(directory), "fixture", 18091, "linux/arm64", "b" * 40)
            self.assertEqual(json.loads((root / "review.json").read_text()), receipt)

    def test_rejects_escaping_symlink_and_invalid_platform_without_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("../other", "/absolute", "Nested/path", ""):
                with self.assertRaises(ValueError):
                    review_path(root, name)
            with self.assertRaises(ValueError):
                provision(root, "fixture", 8080, "darwin/arm64", "a" * 40)
            (root / ".agent-scratch").symlink_to(root, target_is_directory=True)
            with self.assertRaises(ValueError):
                provision(root, "fixture", 18091, "linux/amd64", "a" * 40)


    def test_optional_web_is_explicit_and_keeps_image_overrides_in_restart_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            images = {"FLEETWORKS_RUNTIME_IMAGE": "fleetworks-candidate-runtime:" + "a" * 40 + "-amd64"}
            receipt = provision(Path(directory), "web", 18091, "linux/amd64", "a" * 40,
                                web_port=18092, images=images)
            environment = receipt["environment"]
            self.assertEqual(environment["FLEETWORKS_WEB_ORIGIN"], "http://127.0.0.1:18092")
            self.assertEqual(environment["FLEETWORKS_WEB_PORT"], "18092")
            self.assertEqual(environment["FLEETWORKS_RUNTIME_IMAGE"], images["FLEETWORKS_RUNTIME_IMAGE"])
            self.assertIn("native-review-web-", environment["FLEETWORKS_WEB_IMAGE"])
            stored = json.loads((Path(receipt["workspace"]) / "review.json").read_text())
            self.assertEqual(stored, receipt)
            self.assertTrue((Path(receipt["workspace"]) / "secrets/internal-token").is_file())

    def test_invalid_web_or_image_settings_cannot_create_a_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for options in ({"web_port": 18091}, {"web_port": 80},
                            {"images": {"FLEETWORKS_ACCESS_EPOCH": "forged"}},
                            {"images": {"FLEETWORKS_RUNTIME_IMAGE": "unversioned"}}):
                with self.subTest(options=options), self.assertRaises(ValueError):
                    provision(root, "fixture", 18091, "linux/amd64", "a" * 40, **options)
                self.assertFalse(review_path(root, "fixture").exists())


if __name__ == "__main__":
    unittest.main()
