"""Offline image-lane scope contracts; no Docker or build is executed."""
import argparse
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("release_images", Path(__file__).with_name("release_images.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ImageScopeTests(unittest.TestCase):
    def args(self, directory, targets):
        return argparse.Namespace(targets=targets, revision="a" * 40, arch="amd64",
            registry="ghcr.io/synthetic", image_tag="review-1", version="0.2.1",
            directory=directory, push=False, compose=False)

    def test_web_only_does_not_build_or_launch_backend_engines(self):
        with tempfile.TemporaryDirectory() as temporary:
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                return ("a" * 40 + "\n").encode() if command[:2] == ("git", "rev-parse") else None
            with mock.patch.object(release, "run", side_effect=run), \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images", return_value={}):
                release.build(self.args(Path(temporary), "web"))
            bake = next(command for command in calls if command[:3] == ("docker", "buildx", "bake"))
            self.assertEqual(bake[-1], "web")
            self.assertFalse(any(value in {"runtime", "surrealdb", "search-index"} for value in bake))
            self.assertFalse(any("deploy/verify.py" in command for command in calls))
            self.assertTrue(any(command[:3] == ("docker", "run", "--rm") for command in calls))

    def test_backend_selection_uses_exact_preloaded_compose_ids_without_tar_export(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(release.os.environ, {"GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}):
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                return ("a" * 40 + "\n").encode() if command[:2] == ("git", "rev-parse") else None
            with mock.patch.object(release, "run", side_effect=run), \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images", return_value={}), \
                 mock.patch.object(release.images, "service_image_ids", return_value={"api-gateway": "sha256:" + "b" * 64}):
                release.build(self.args(Path(temporary), "runtime"))
            smoke = next(command for command in calls if "deploy/verify.py" in command)
            self.assertIn("--no-build", smoke)
            self.assertIn("--image-ids", smoke)
            self.assertFalse(any(command[:2] == ("docker", "push") for command in calls))
            self.assertFalse(any(command[:3] == ("docker", "image", "save") for command in calls))

    def test_push_exports_selected_images_tarball_and_records_checksum(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args = self.args(directory, "web")
            args.push = True
            image_id = "sha256:" + "c" * 64
            digest = "sha256:" + "d" * 64
            raw_manifest = release.json.dumps({"config": {"digest": image_id}}).encode()
            repo_digests = release.json.dumps([f"ghcr.io/synthetic/fleetworks-web@{digest}"]).encode()

            def run(*command, **_kwargs):
                if command[:2] == ("git", "rev-parse"):
                    return ("a" * 40 + "\n").encode()
                if command[:4] == ("docker", "buildx", "imagetools", "inspect"):
                    return raw_manifest
                if command[:3] == ("docker", "image", "inspect"):
                    return repo_digests
                return None

            def fake_save(archive, selected_tags):
                self.assertEqual(list(selected_tags), ["web"])
                archive.write_bytes(b"synthetic image tarball")

            probe = mock.Mock(returncode=1, stderr=b"manifest unknown")
            with mock.patch.object(release, "run", side_effect=run), \
                 mock.patch.object(release.subprocess, "run", return_value=probe), \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images", return_value={"web": {"id": image_id}}), \
                 mock.patch.object(release.images, "save_images", side_effect=fake_save):
                release.build(args)

            manifest = release.json.loads((directory / "release-images-amd64.json").read_text())
            archive = directory / "fleetworks-images-linux-amd64.tar.gz"
            checksum = directory / "fleetworks-images-linux-amd64.tar.gz.sha256"
            self.assertTrue(archive.is_file())
            self.assertTrue(checksum.is_file())
            self.assertEqual(manifest["archive"]["name"], "fleetworks-images-linux-amd64.tar.gz")
            self.assertEqual(manifest["archive"]["bytes"], len(b"synthetic image tarball"))
            self.assertEqual(checksum.read_text().split()[0], manifest["archive"]["sha256"])


if __name__ == "__main__":
    unittest.main()
