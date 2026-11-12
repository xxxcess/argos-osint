"""Offline credential/provisioning contracts; no Docker or real workspace."""

import json
import os
import stat
import tempfile
import unittest
from pathlib import Path

from native_review import TOKEN_NAMES, provision, review_path


class NativeReviewTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
