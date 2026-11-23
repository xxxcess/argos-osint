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
            directory=directory, builder="buildx", depot_project=None, push=False, compose=False,
            dev_smoke=False)

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

    def test_depot_load_uses_native_platform_and_existing_smoke_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = self.args(Path(temporary), "web")
            args.arch = "arm64"
            args.builder = "depot"
            args.depot_project = "review-project"
            tag = "ghcr.io/synthetic/fleetworks-web:review-1-arm64"
            with mock.patch.object(release, "run", side_effect=[b"a" * 40, None, None]) as run, \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/arm64"), \
                 mock.patch.object(release.images, "inspect_images", return_value={}) as inspect:
                release.build(args)

            bake = run.call_args_list[1]
            self.assertEqual(bake.args, ("depot", "bake", "--project", "review-project",
                "--build-platform", "linux/arm64", "-f", "deploy/docker-bake.hcl", "--load",
                "--set", "*.platforms=linux/arm64", "--set", f"web.tags={tag}", "web"))
            self.assertEqual(bake.kwargs["env"]["VERSION"], "0.2.1")
            inspect.assert_called_once_with({"web": tag}, "linux/arm64")
            self.assertEqual(run.call_args.args[:6], ("docker", "run", "--rm", "--entrypoint", "node", tag))
            manifest = release.json.loads((args.directory / "release-images-arm64.json").read_text())
            self.assertEqual(manifest["builder"], "depot")
            self.assertEqual(set(manifest["timings_seconds"]), {"bake_and_load", "smoke"})

    def test_depot_failure_propagates_before_smoke_or_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = self.args(Path(temporary), "web")
            args.builder = "depot"
            args.depot_project = "review-project"
            failure = release.subprocess.CalledProcessError(1, ("depot", "bake"))
            with mock.patch.object(release, "run", side_effect=[b"a" * 40, failure]) as run, \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images") as inspect:
                with self.assertRaises(release.subprocess.CalledProcessError) as raised:
                    release.build(args)

            self.assertIs(raised.exception, failure)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args.args[:2], ("depot", "bake"))
            inspect.assert_not_called()

    def test_depot_requires_explicit_project(self):
        argv = ["release_images.py", "--targets", "web", "--arch", "amd64", "--revision", "a" * 40,
                "--version", "0.2.1", "--image-tag", "review-1", "--registry", "ghcr.io/synthetic",
                "--directory", "/unused", "--builder", "depot"]
        with mock.patch.object(release.sys, "argv", argv), \
             mock.patch.object(release.sys, "stderr") as stderr, \
             mock.patch.object(release, "build") as build:
            with self.assertRaises(SystemExit) as raised:
                release.main()

        self.assertEqual(raised.exception.code, 2)
        self.assertIn("--builder depot requires --depot-project", stderr.write.call_args.args[0])
        build.assert_not_called()

    def test_backend_selection_uses_exact_preloaded_compose_ids_without_tar_export(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(release.os.environ, {"GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}):
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                return ("a" * 40 + "\n").encode() if command[:2] == ("git", "rev-parse") else None
            with mock.patch.object(release, "run", side_effect=run), \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images", side_effect=lambda tags, _: {
                     role: {"tag": tag} for role, tag in tags.items()}), \
                 mock.patch.object(release.images, "service_image_ids", return_value={"api-gateway": "sha256:" + "b" * 64}):
                release.build(self.args(Path(temporary), "runtime"))
            smoke = next(command for command in calls if "deploy/verify.py" in command)
            self.assertIn("--no-build", smoke)
            self.assertIn("--image-ids", smoke)
            self.assertFalse(any(command[:2] == ("docker", "push") for command in calls))
            self.assertFalse(any(command[:3] == ("docker", "image", "save") for command in calls))


    def test_depot_bakes_development_web_once_and_smokes_both_overlays(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(release.os.environ, {
                "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}):
            args = self.args(Path(temporary), "runtime,web")
            args.builder = "depot"
            args.depot_project = "review-project"
            args.dev_smoke = True

            def inspect(tags, _platform):
                return {role: {"tag": tag, "id": tag} for role, tag in tags.items()}

            def expected_ids(_overrides, inspected, _platform, *, web):
                self.assertTrue(web)
                return {role: item["id"] for role, item in inspected.items()}

            with mock.patch.object(release, "run", side_effect=[b"a" * 40, None, None, None]) as run, \
                 mock.patch.object(release.images, "verify_daemon", return_value="linux/amd64"), \
                 mock.patch.object(release.images, "inspect_images", side_effect=inspect), \
                 mock.patch.object(release.images, "service_image_ids", side_effect=expected_ids):
                release.build(args)

            _, bake, production, development = run.call_args_list
            self.assertEqual(bake.args[:2], ("depot", "bake"))
            self.assertEqual(bake.args[-5:], ("runtime", "search-index", "surrealdb", "web", "web-dev"))
            self.assertIn(f"web-dev.args.FLEETWORKS_UID={release.os.getuid()}", bake.args)
            self.assertIn(f"web-dev.args.FLEETWORKS_GID={release.os.getgid()}", bake.args)
            self.assertIn("--no-build", production.args)
            self.assertIn("--no-build", development.args)
            self.assertIn("--dev", development.args)
            self.assertIn("--web", development.args)
            self.assertNotIn("--dev", production.args)
            self.assertNotEqual(production.kwargs["env"]["FLEETWORKS_WEB_IMAGE"],
                                development.kwargs["env"]["FLEETWORKS_WEB_IMAGE"])
            prod_ids = release.json.loads(Path(production.args[-1]).read_text())
            dev_ids = release.json.loads(Path(development.args[-1]).read_text())
            self.assertNotEqual(prod_ids.pop("web"), dev_ids.pop("web"))
            self.assertEqual(prod_ids, dev_ids)

    def test_release_rejects_development_web_smoke(self):
        args = self.args(Path("/unused"), "web")
        args.push = True
        args.dev_smoke = True
        with self.assertRaisesRegex(ValueError, "review builds only"):
            release.build(args)

    def test_depot_release_keeps_registry_credentials_local_and_records_archive_checksum(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args = self.args(directory, "web")
            args.push = True
            args.builder = "depot"
            args.depot_project = "release-project"
            image_id = "sha256:" + "c" * 64
            digest = "sha256:" + "d" * 64
            raw_manifest = release.json.dumps({"config": {"digest": image_id}}).encode()
            repo_digests = release.json.dumps([f"ghcr.io/synthetic/fleetworks-web@{digest}"]).encode()

            def run(*command, **kwargs):
                if command[:2] == ("git", "rev-parse"):
                    return ("a" * 40 + "\n").encode()
                if command[:3] == ("docker", "context", "inspect"):
                    return b"unix:///var/run/custom-docker.sock\n"
                if command[:2] == ("depot", "bake"):
                    environment = kwargs["env"]
                    config = Path(environment["DOCKER_CONFIG"])
                    self.assertEqual(release.json.loads((config / "config.json").read_text()),
                                     {"auths": {"ghcr.io": {}}})
                    self.assertNotEqual(config, directory)
                    self.assertNotIn("DOCKER_CONTEXT", environment)
                    self.assertNotIn("DOCKER_AUTH_CONFIG", environment)
                    self.assertEqual(environment["DOCKER_HOST"], "unix:///var/run/custom-docker.sock")
                if command[:2] == ("docker", "push"):
                    self.assertNotIn("env", kwargs)
                    self.assertEqual(release.os.environ["DOCKER_CONFIG"], str(directory))
                if command[:4] == ("docker", "buildx", "imagetools", "inspect"):
                    return raw_manifest
                if command[:3] == ("docker", "image", "inspect"):
                    return repo_digests
                return None

            def fake_save(archive, selected_tags):
                self.assertEqual(list(selected_tags), ["web"])
                archive.write_bytes(b"synthetic image tarball")

            probe = mock.Mock(returncode=1, stderr=b"manifest unknown")
            with mock.patch.dict(release.os.environ, {
                    "DOCKER_CONFIG": str(directory), "DOCKER_CONTEXT": "release",
                    "DOCKER_AUTH_CONFIG": "synthetic-registry-config"}), \
                 mock.patch.object(release, "run", side_effect=run), \
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
