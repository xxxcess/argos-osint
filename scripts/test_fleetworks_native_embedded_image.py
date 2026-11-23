"""Offline contracts for packaging verified native artifacts into a development image."""

import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import fleetworks_native_embedded_image as image


class NativeEmbeddedImageTests(unittest.TestCase):
    def args(self):
        return SimpleNamespace(
            archive=Path("native.tar.gz"), sha256="a" * 64, revision="b" * 40, version="0.2.1",
            profile="debug", builder="depot", depot_project="project", tag="fleetworks-native:test", output=None,
        )

    def test_verified_bundle_is_the_only_artifact_in_the_build_context(self):
        args = self.args()
        image_id = "sha256:" + "c" * 64
        bundle_name = "Fleetworks-linux-x86_64"

        def extract(archive, destination, **options):
            self.assertEqual(archive, args.archive)
            self.assertEqual(options["expected_digest"], args.sha256)
            self.assertEqual(options["revision"], args.revision)
            self.assertEqual(options["target"], image.TARGET)
            self.assertEqual(options["profile"], args.profile)
            self.assertTrue(options["require_clean"])
            bundle = destination / bundle_name
            bundle.mkdir()
            return bundle, {"companion": {"name": "argos-api-host"}}

        def build(command, *, cwd, **options):
            self.assertEqual({path.name for path in cwd.iterdir()}, {bundle_name, image.BAKEFILE, image.DOCKERFILE})
            self.assertEqual(command[:6], ("depot", "bake", "--project", "project", "--build-platform", "linux/amd64"))
            self.assertIn("--load", command)
            self.assertIn(f"native-embedded.args.NATIVE_ARCHIVE_SHA256={args.sha256}", command)
            self.assertEqual(command[-1], "native-embedded")
            self.assertTrue(options["check"])

        with patch.object(image, "verify_daemon", return_value="linux/amd64"), \
             patch.object(image, "locked_rune_revision", return_value="c" * 40), \
             patch.object(image, "extract_native", side_effect=extract), \
             patch.object(image.subprocess, "run", side_effect=build), \
             patch.object(image, "inspect_images", return_value={image.IMAGE: {"tag": args.tag, "id": image_id}}), \
             patch.object(image, "smoke_image", return_value={"status": "pass"}) as smoke:
            result = image.package(args)

        smoke.assert_called_once_with(image_id)
        self.assertEqual(result["image"]["id"], image_id)
        self.assertEqual(result["native_archive_sha256"], args.sha256)

    def test_native_artifact_without_companion_is_rejected_before_build(self):
        with patch.object(image, "verify_daemon", return_value="linux/amd64"), \
             patch.object(image, "locked_rune_revision", return_value="c" * 40), \
             patch.object(image, "extract_native", return_value=(Path("bundle"), {})), \
             patch.object(image.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "verified bundled companion"):
                image.package(self.args())
        run.assert_not_called()

    def test_container_smoke_uses_the_exact_image_without_network(self):
        evidence = {
            "status": "pass",
            "socket_check_method": "procfs",
            "platform": {"system": "Linux", "machine": "x86_64"},
        }
        image_id = "sha256:" + "d" * 64
        with patch.object(image.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps(evidence))) as run:
            self.assertEqual(image.smoke_image(image_id), evidence)
        command = run.call_args.args[0]
        self.assertEqual(command[:3], ["docker", "run", "--rm"])
        self.assertEqual(command[command.index("--network") + 1], "none")
        self.assertEqual(command[command.index("--cap-drop") + 1], "ALL")
        self.assertIn(image_id, command)
        self.assertIn("ldd", command[-1])
        self.assertIn("--skip-rust-probe", command[-1])


if __name__ == "__main__":
    unittest.main()
