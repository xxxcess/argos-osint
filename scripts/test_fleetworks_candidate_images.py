"""Offline checks for the native Docker archive handoff."""

import gzip
import io
import json
import tempfile
import unittest
from contextlib import ExitStack, redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import fleetworks_candidate_images as candidate


class CandidateImageTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.directory = Path(scratch.name)
        self.revision = "1" * 40
        self.target = "x86_64-unknown-linux-gnu"
        self.platform = "linux/amd64"
        self.tags = candidate.image_tags(self.revision, self.platform)
        self.images = {
            role: {"tag": tag, "id": f"sha256:{index:064x}", "bytes": 1}
            for index, (role, tag) in enumerate(self.tags.items())
        }

    def test_linux_image_candidates_require_matching_physical_host_and_daemon(self):
        host = SimpleNamespace(system="Linux", docker_platform="linux/amd64", rosetta=False)
        with patch.object(candidate, "detect", return_value=host), patch.object(candidate, "docker", return_value="linux/amd64\n"):
            self.assertEqual(candidate.verify_daemon(self.target), "linux/amd64")
        with patch.object(candidate, "detect", return_value=host), patch.object(candidate, "docker", return_value="linux/arm64\n"):
            with self.assertRaisesRegex(ValueError, "daemon is not native"):
                candidate.verify_daemon(self.target)
        with patch.object(candidate, "detect", return_value=host), patch.object(candidate, "docker") as docker:
            with self.assertRaisesRegex(ValueError, "physical Linux host"):
                candidate.verify_daemon("aarch64-unknown-linux-gnu")
        docker.assert_not_called()

    def test_archive_export_bounds_bytes_and_does_not_replace_existing_archive(self):
        archive = self.directory / "images.tar.gz"
        payload = b"a native docker archive"
        with patch.object(candidate.subprocess, "Popen") as spawn:
            spawn.return_value.stdout = io.BytesIO(payload)
            spawn.return_value.wait.return_value = 0
            candidate.save_images(archive, self.tags)
            self.assertEqual(gzip.decompress(archive.read_bytes()), payload)
            self.assertEqual(spawn.call_args.args[0], ["docker", "image", "save", *self.tags.values()])
            with self.assertRaises(FileExistsError):
                candidate.save_images(archive, self.tags)
            self.assertEqual(gzip.decompress(archive.read_bytes()), payload)
            self.assertEqual(spawn.call_count, 1)
        archive.unlink()
        with patch.object(candidate.subprocess, "Popen") as spawn, patch.object(candidate, "MAX_IMAGE_BYTES", 1):
            spawn.return_value.stdout = io.BytesIO(payload)
            spawn.return_value.poll.return_value = None
            with self.assertRaisesRegex(ValueError, "uncompressed size limit"):
                candidate.save_images(archive, self.tags)
            spawn.return_value.kill.assert_called_once()
            self.assertFalse(archive.exists())

    def test_manifest_and_archive_must_match_the_packaging_job(self):
        archive = self.directory / "images.tar.gz"
        archive.write_bytes(b"example archive")
        archive_digest = candidate.sha256(archive)
        archive.with_suffix(".gz.sha256").write_text(candidate.checksum_line(archive, archive_digest))
        manifest = {
            "format": 1, "revision": self.revision, "target": self.target, "platform": self.platform,
            "archive": {"bytes": archive.stat().st_size, "sha256": archive_digest}, "images": self.images,
        }
        metadata = self.directory / "images.json"
        metadata.write_text(json.dumps(manifest))
        manifest_digest = candidate.sha256(metadata)
        args = (self.directory, manifest_digest, self.revision, self.target, self.platform)
        self.assertEqual(candidate.read_manifest(*args), (manifest, archive))
        with self.assertRaisesRegex(ValueError, "packaging job's SHA-256"):
            candidate.read_manifest(self.directory, "0" * 64, self.revision, self.target, self.platform)
        archive.write_bytes(b"example Archive")
        with self.assertRaisesRegex(ValueError, "expected SHA-256"):
            candidate.read_manifest(*args)

    def test_resolved_compose_services_must_all_bind_loaded_image_ids(self):
        services = {"bootstrap": self.tags["runtime"], "api": self.tags["runtime"], "native-worker":self.tags["runtime"], **self.tags}
        with patch.object(candidate, "compose_images", return_value=services):
            ids = candidate.service_image_ids({}, self.images)
            self.assertEqual(ids["bootstrap"], ids["api"])
            self.assertEqual(ids["api"], self.images["runtime"]["id"])
            self.assertEqual(ids["native-worker"], self.images["runtime"]["id"])
            self.assertEqual(ids.keys(), services.keys())
        with patch.object(candidate, "compose_images", return_value={**services, "other": "registry.invalid/image"}):
            with self.assertRaisesRegex(ValueError, "exactly the archived image tags"):
                candidate.service_image_ids({}, self.images)
        with patch.object(candidate,"compose_images",return_value={key:value for key,value in services.items() if key!="native-worker"}):
            with self.assertRaisesRegex(ValueError,"Registered native worker"):
                candidate.service_image_ids({},self.images)

    def test_image_inspect_rejects_mismatched_native_architecture(self):
        sample = json.dumps([{"Id": f"sha256:{1:064x}", "Os": "linux", "Architecture": "arm64", "Size": 123}])
        with patch.object(candidate, "docker", return_value=sample):
            with self.assertRaisesRegex(ValueError, "valid linux/amd64 image identity"):
                candidate.inspect_images({"runtime": self.tags["runtime"]}, self.platform)

    def test_downloaded_images_use_direct_owned_compose_without_a_cli_archive(self):
        manifest = {"images": self.images}
        image_ids = {role: item["id"] for role, item in self.images.items()}
        with ExitStack() as mocks:
            mocks.enter_context(redirect_stdout(io.StringIO()))
            for name, value in (("verify_daemon", self.platform),
                                ("read_manifest", (manifest, self.directory / "images.tar.gz")),
                                ("docker", None), ("inspect_images", self.images),
                                ("service_image_ids", image_ids)):
                mocks.enter_context(patch.object(candidate, name, return_value=value))
            run = mocks.enter_context(patch.object(candidate.subprocess, "run"))
            candidate.verify(self.directory, "manifest", self.revision, self.target)
        run.assert_called_once()
        command = run.call_args.args[0]
        self.assertIn(str(candidate.ROOT / "deploy/verify.py"), command)
        self.assertIn("--no-build", command)
        self.assertIn("--source-journey", command)
        self.assertIn("--image-ids", command)
        self.assertNotIn("--web", command)
        self.assertIn("--atlas", command)
        self.assertNotIn("--venture", command)
        self.assertEqual(command[command.index("--platform") + 1], self.platform)
        for role, key in candidate.IMAGE_ENV.items():
            self.assertEqual(run.call_args.kwargs["env"][key], self.tags[role])

    def test_optional_web_is_additive_to_backend_images(self):
        self.assertEqual(set(self.tags), {"runtime", "surrealdb", "search-index"})
        web = candidate.image_tags(self.revision, self.platform, web=True)
        self.assertEqual(set(web), {"runtime", "surrealdb", "search-index", "web"})
        self.assertEqual({key: web[key] for key in self.tags}, self.tags)

    def test_loaded_identity_mismatch_never_starts_a_stack(self):
        with patch.object(candidate, "verify_daemon", return_value=self.platform), \
             patch.object(candidate, "read_manifest", return_value=({"images": self.images}, self.directory / "images.tar.gz")), \
             patch.object(candidate, "docker"), \
             patch.object(candidate, "inspect_images", return_value={**self.images, "runtime": {**self.images["runtime"], "id": "sha256:" + "f" * 64}}), \
             patch.object(candidate.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "Loaded image IDs"):
                candidate.verify(self.directory, "manifest", self.revision, self.target)
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
