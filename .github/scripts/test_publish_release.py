"""Offline release guards: no Docker, GitHub, builds or credentials are used."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish_release.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class PublicationTests(unittest.TestCase):
    def plan(self):
        return {"publish": True, "head": "a" * 40, "version": "1.2.3", "image_tag": "stable-dev-1.2.3-g" + "a" * 40,
                "channel": "stable", "native": False, "image_targets": ["runtime", "web"],
                "tag": "fleetworks-dev-v1.2.3", "prerelease": False}

    def manifests(self, directory, plan):
        result = {}
        for index, arch in enumerate(("amd64", "arm64")):
            images = {role: {"repository": "ghcr.io/synthetic/fleetworks-" + role,
                             "digest": "sha256:" + str(index + offset + 1) * 64}
                      for offset, role in enumerate(plan["image_targets"])}
            value = {"revision": plan["head"], "version": plan["version"], "image_tag": plan["image_tag"],
                     "arch": arch, "selected": plan["image_targets"], "images": images}
            (directory / f"release-images-{arch}.json").write_text(json.dumps(value))
            result[arch] = value
        return result

    def test_failed_or_untrusted_plan_cannot_call_publish_tools(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(release, "run") as run:
            plan = self.plan()
            plan["publish"] = False
            with self.assertRaises(ValueError):
                release.publish(plan, Path(temporary))
            run.assert_not_called()

    def test_missing_native_architecture_and_changed_source_fail_before_promotion(self):
        for fault in ("missing", "source"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary, mock.patch.object(release, "run") as run:
                directory = Path(temporary)
                plan = self.plan()
                self.manifests(directory, plan)
                path = directory / "release-images-arm64.json"
                if fault == "missing":
                    path.unlink()
                else:
                    value = json.loads(path.read_text())
                    value["revision"] = "b" * 40
                    path.write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    release.publish(plan, directory)
                run.assert_not_called()

    def test_all_multiarch_indexes_must_verify_before_any_channel_moves(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            manifests = self.manifests(directory, plan)
            calls = []

            def run(*command, **_kwargs):
                calls.append(command)
                if "inspect" in command:
                    role = "web" if "fleetworks-web:" in command[-1] else "runtime"
                    descriptors = [{"platform": {"os": "linux", "architecture": arch},
                                    "digest": manifests[arch]["images"][role]["digest"]}
                                   for arch in ("amd64", "arm64")]
                    if role == "web":
                        descriptors.pop()
                    return json.dumps({"manifests": descriptors}).encode()
                return b""

            with mock.patch.object(release, "run", side_effect=run), mock.patch.object(release, "lookup_release", return_value=None):
                with self.assertRaisesRegex(ValueError, "inventory mismatch"):
                    release.publish(plan, directory)
            self.assertFalse(any(any(value.endswith(":stable") for value in command) for command in calls))
            self.assertFalse(any(command[0] == "gh" for command in calls))

    def test_duplicate_architecture_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(release, "run") as run:
            directory = Path(temporary)
            plan = self.plan()
            self.manifests(directory, plan)
            (directory / "extra").mkdir()
            (directory / "extra/release-images-amd64.json").write_text((directory / "release-images-amd64.json").read_text())
            with self.assertRaises(ValueError):
                release.publish(plan, directory)
            run.assert_not_called()

    def test_wrong_existing_tag_and_asset_never_promote_channel(self):
        for fault in ("tag", "asset"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                plan = self.plan()
                self.manifests(directory, plan)
                calls = []
                info = {"isDraft": False, "assets": [{"name": "release.json"}, {"name": "release-images-amd64.json"}]}
                inventory = {"format": 2, "plan": plan, "assets": {"release-images-amd64.json": "0" * 64}}
                def run(*command, **_kwargs):
                    calls.append(command)
                    if command[:2] == ("gh", "api"):
                        return (("b" if fault == "tag" else "a") * 40 + "\n").encode()
                    if command[:3] == ("gh", "release", "download"):
                        destination = Path(command[command.index("--dir") + 1])
                        name = command[command.index("--pattern") + 1]
                        (destination / name).write_text(json.dumps(inventory) if name == "release.json" else "corrupted original")
                    return b""
                with mock.patch.object(release, "run", side_effect=run), mock.patch.object(release, "lookup_release", return_value=info):
                    with self.assertRaises(ValueError):
                        release.publish(plan, directory)
                self.assertFalse(any(command[0] == "docker" for command in calls))
                self.assertFalse(any(command[:3] == ("gh", "release", "upload") for command in calls))

    def test_completed_retry_reuses_original_pair_without_uploading_new_attempt(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            plan.update(native=True, image_targets=[])
            original = {}
            for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                base = f"Fleetworks-original-attempt-{target}"
                original[base + ".zip"] = b"original immutable archive"
                original[base + ".zip.sha256"] = (release.hashlib.sha256(original[base + ".zip"]).hexdigest() + "  " + base + ".zip\n").encode()
                original[base + ".manifest.json"] = json.dumps({"revision": plan["head"], "version": plan["version"], "dirty": False, "profile": "release", "target": target}).encode()
            inventory = {"format": 2, "plan": plan, "assets": {name: release.hashlib.sha256(value).hexdigest() for name, value in original.items()}}
            original["release.json"] = json.dumps(inventory).encode()
            info = {"isDraft": False, "assets": [{"name": name} for name in original]}
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                if command[:2] == ("gh", "api"):
                    return (plan["head"] + "\n").encode()
                if command[:3] == ("gh", "release", "download"):
                    destination = Path(command[command.index("--dir") + 1])
                    name = command[command.index("--pattern") + 1]
                    (destination / name).write_bytes(original[name])
                return b""
            with mock.patch.object(release, "run", side_effect=run), mock.patch.object(release, "lookup_release", return_value=info), mock.patch.object(release, "prepare_current") as prepare, mock.patch.object(release, "lookup_channel", return_value={"object": {"type": "commit", "sha": plan["head"]}}):
                release.publish(plan, directory)
                prepare.assert_not_called()
            self.assertFalse(any(command[:3] == ("gh", "release", "upload") for command in calls))
            self.assertEqual(len(list((directory / "assets").glob("*.zip"))), 2)


    def test_stale_retry_keeps_both_channels_and_compares_only_after_asset_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            manifests = self.manifests(directory, plan)
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                if "inspect" in command:
                    role = "web" if "fleetworks-web:" in command[-1] else "runtime"
                    return json.dumps({"manifests": [{"platform": {"os": "linux", "architecture": arch},
                        "digest": manifests[arch]["images"][role]["digest"]} for arch in ("amd64", "arm64")]}).encode()
                if command[:2] == ("gh", "api") and "/compare/" in command[2]:
                    return b'{"status":"behind"}'
                return b""
            with mock.patch.object(release, "run", side_effect=run), mock.patch.object(release, "lookup_release", return_value=None), mock.patch.object(release, "lookup_channel", return_value={"object": {"type": "commit", "sha": "b" * 40}}):
                release.publish(plan, directory)
            published = next(index for index, command in enumerate(calls) if command[:3] == ("gh", "release", "edit"))
            compared = next(index for index, command in enumerate(calls) if command[:2] == ("gh", "api") and "/compare/" in command[2])
            uploaded = max(index for index, command in enumerate(calls) if command[:3] == ("gh", "release", "upload"))
            self.assertLess(uploaded, compared)
            self.assertIn("--latest=false", calls[published])
            self.assertFalse(any("PATCH" in command for command in calls))
            self.assertFalse(any(any(value.endswith(":stable") for value in command) for command in calls))

    def test_new_channel_ref_is_created_only_after_complete_asset_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            manifests = self.manifests(directory, plan)
            calls = []
            def run(*command, **_kwargs):
                calls.append(command)
                if "inspect" in command:
                    role = "web" if "fleetworks-web:" in command[-1] else "runtime"
                    return json.dumps({"manifests": [{"platform": {"os": "linux", "architecture": arch},
                        "digest": manifests[arch]["images"][role]["digest"]} for arch in ("amd64", "arm64")]}).encode()
                return b""
            with mock.patch.object(release, "run", side_effect=run), mock.patch.object(release, "lookup_release", return_value=None), mock.patch.object(release, "lookup_channel", return_value=None):
                release.publish(plan, directory)
            published = next(index for index, command in enumerate(calls) if command[:3] == ("gh", "release", "edit"))
            tagged = next(index for index, command in enumerate(calls) if command[:2] == ("gh", "api") and "POST" in command)
            aliased = [index for index, command in enumerate(calls) if any(value.endswith(":stable") for value in command)]
            self.assertLess(published, tagged)
            self.assertTrue(all(tagged < index for index in aliased))
            self.assertIn("ref=refs/tags/stable", calls[tagged])

    def test_annotated_channel_ref_cannot_be_replaced(self):
        with mock.patch.object(release, "lookup_channel", return_value={"object": {"type": "tag", "sha": "b" * 40}}), mock.patch.object(release, "run") as run:
            with self.assertRaisesRegex(ValueError, "lightweight"):
                release.advance_channel(self.plan())
            run.assert_not_called()



if __name__ == "__main__":
    unittest.main()
