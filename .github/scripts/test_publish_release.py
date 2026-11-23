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
                "channel": "stable", "native": False, "bert": False, "image_targets": ["runtime", "web"],
                "tag": "fleetworks-dev-v1.2.3", "prerelease": False}

    def manifests(self, directory, plan):
        result = {}
        for index, arch in enumerate(("amd64", "arm64")):
            archive_name = f"fleetworks-images-linux-{arch}.tar.gz"
            archive_bytes = f"synthetic-images-{arch}".encode()
            archive_digest = release.hashlib.sha256(archive_bytes).hexdigest()
            (directory / archive_name).write_bytes(archive_bytes)
            (directory / f"{archive_name}.sha256").write_text(f"{archive_digest}  {archive_name}\n")
            images = {role: {"repository": "ghcr.io/synthetic/fleetworks-" + role,
                             "digest": "sha256:" + str(index + offset + 1) * 64}
                      for offset, role in enumerate(plan["image_targets"])}
            value = {"revision": plan["head"], "version": plan["version"], "image_tag": plan["image_tag"],
                     "arch": arch, "selected": plan["image_targets"], "images": images,
                     "archive": {"name": archive_name, "sha256": archive_digest, "bytes": len(archive_bytes)}}
            (directory / f"release-images-{arch}.json").write_text(json.dumps(value))
            result[arch] = value
        return result

    def bert_bundles(self, directory, plan):
        for arch in ("amd64", "arm64"):
            bundle_dir = directory / f"atlas-bert-bundle-{arch}"
            evidence_dir = directory / f"atlas-bert-evidence-{arch}"
            bundle_dir.mkdir(parents=True, exist_ok=True)
            evidence_dir.mkdir(parents=True, exist_ok=True)
            archive_name = f"atlas-bert-linux-{arch}.tar.gz"
            payload = f"synthetic-bert-{arch}".encode()
            digest = release.hashlib.sha256(payload).hexdigest()
            (bundle_dir / archive_name).write_bytes(payload)
            evidence = {
                "arch": arch,
                "source_sha": plan["head"],
                "status": "passed",
                "dirty": False,
                "unit_tests_passed": 3,
                "build_inference": {"status": "passed"},
                "extracted_inference": {"status": "passed"},
                "archive": {"file": archive_name, "bytes": len(payload), "sha256": digest},
            }
            (evidence_dir / "evidence.json").write_text(json.dumps(evidence))

    def qa_pack_pairs(self, directory, plan):
        for target in release.INBOX_QA_TARGETS:
            target_dir = directory / target
            target_dir.mkdir()
            native_digest = "f" * 64
            if target.endswith("-apple-darwin"):
                native = target_dir / f"Fleetworks-{target}.zip"
                native.write_bytes(f"native archive {target}".encode())
                native_digest = release.sha256(native)
                native.with_name(native.name + ".sha256").write_text(f"{native_digest}  {native.name}\n")
                (target_dir / "Fleetworks.app.manifest.json").write_text(json.dumps({
                    "revision": plan["head"], "version": plan["version"], "dirty": False,
                    "profile": "release", "target": target,
                }))
            archive = target_dir / f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
            archive.write_bytes(f"QA archive {target}".encode())
            receipt = archive.with_name(archive.name + ".digests.json")
            receipt.write_text(json.dumps({
                "target": target, "source_sha": plan["head"], "version": plan["version"],
                "model_revision": release.MODEL_REVISION, "archive": archive.name,
                "archive_sha256": release.sha256(archive), "manifest_sha256": "e" * 64,
                "status": "passed", "runtime": "embedded-unix-socket", "native_archive_sha256": native_digest,
                "transport_attestation": {
                    "transport": "embedded-unix-socket",
                    "socket_family": "unix",
                    "socket_type": "stream",
                    "protocol_version": 2,
                    "min_protocol_version": 2,
                    "dataset_upload_chunk_bytes": 262144,
                    "dataset_upload_window": 4,
                },
                "terminal_state": "answered", "terminal_events": 1, "model_pid_count": 1,
                "cancelled_terminal_state": "canceled", "cancelled_terminal_events": 1,
            }))

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
            inventory = {"format": 2, "plan": plan.copy(), "assets": {name: release.hashlib.sha256(value).hexdigest() for name, value in original.items()}}
            plan["inbox_qa"] = False  # Old inventories omit this unselected optional field.
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

    def test_bert_bundles_and_image_archives_are_verified_and_published(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            plan["bert"] = True
            manifests = self.manifests(directory, plan)
            self.bert_bundles(directory, plan)
            calls = []

            def run(*command, **_kwargs):
                calls.append(command)
                if "inspect" in command:
                    role = "web" if "fleetworks-web:" in command[-1] else "runtime"
                    return json.dumps({"manifests": [{"platform": {"os": "linux", "architecture": arch},
                        "digest": manifests[arch]["images"][role]["digest"]} for arch in ("amd64", "arm64")]}).encode()
                return b""

            with mock.patch.object(release, "run", side_effect=run), \
                 mock.patch.object(release, "lookup_release", return_value=None), \
                 mock.patch.object(release, "lookup_channel", return_value=None):
                release.publish(plan, directory)

            inventory = json.loads((directory / "assets/release.json").read_text())
            for arch in ("amd64", "arm64"):
                self.assertIn(f"fleetworks-images-linux-{arch}.tar.gz", inventory["assets"])
                self.assertIn(f"fleetworks-images-linux-{arch}.tar.gz.sha256", inventory["assets"])
                self.assertIn(f"atlas-bert-linux-{arch}.tar.gz", inventory["assets"])
                self.assertIn(f"atlas-bert-linux-{arch}.tar.gz.sha256", inventory["assets"])
                self.assertIn(f"atlas-bert-evidence-{arch}.json", inventory["assets"])

    def test_publish_includes_three_smoke_verified_inbox_qa_pack_pairs(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            plan.update(channel="nightly", native=True, inbox_qa=True, image_targets=[],
                        event="push", ref="refs/heads/nightly")
            self.qa_pack_pairs(directory, plan)
            with mock.patch.object(release, "run") as run, \
                 mock.patch.object(release, "lookup_release", return_value=None), \
                 mock.patch.object(release, "lookup_channel", return_value=None):
                release.publish(plan, directory)

            inventory = json.loads((directory / "assets/release.json").read_text())
            self.assertIs(inventory["plan"]["inbox_qa"], True)
            payload_calls = [call.args for call in run.call_args_list
                             if call.args[:3] == ("gh", "release", "upload") and len(call.args) > 5]
            self.assertEqual(len(payload_calls), 1)
            for target in release.INBOX_QA_TARGETS:
                archive = f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
                for name in (archive, archive + ".digests.json"):
                    asset = directory / "assets" / name
                    self.assertEqual(inventory["assets"][name], release.sha256(asset))
                    self.assertIn(str(asset), payload_calls[0])

    def test_invalid_inbox_qa_pairs_stop_publication(self):
        for fault in ("missing", "archive", "source", "wrong_native", "untrusted",
                      "stdio", "missing_handshake", "wrong_family", "wrong_window",
                      "missing_cancel", "duplicate_cancel", "boolean_count"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                plan = self.plan()
                plan.update(channel="nightly", native=True, inbox_qa=True, image_targets=[],
                            event="push", ref="refs/heads/nightly")
                self.qa_pack_pairs(directory, plan)
                target = "aarch64-apple-darwin"
                archive = directory / target / f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
                receipt = archive.with_name(archive.name + ".digests.json")
                if fault == "missing":
                    receipt.unlink()
                elif fault == "archive":
                    archive.write_bytes(b"different pack")
                elif fault == "untrusted":
                    plan["event"] = "workflow_dispatch"
                else:
                    evidence = json.loads(receipt.read_text())
                    if fault == "source":
                        evidence["source_sha"] = "b" * 64
                    elif fault == "wrong_native":
                        evidence["native_archive_sha256"] = "b" * 64
                    elif fault == "stdio":
                        evidence["runtime"] = "embedded-stdio"
                    elif fault == "missing_handshake":
                        del evidence["transport_attestation"]
                    elif fault == "wrong_family":
                        evidence["transport_attestation"]["socket_family"] = "inet"
                    elif fault == "wrong_window":
                        evidence["transport_attestation"]["dataset_upload_window"] = 8
                    elif fault == "missing_cancel":
                        del evidence["cancelled_terminal_events"]
                    elif fault == "duplicate_cancel":
                        evidence["cancelled_terminal_events"] = 2
                    elif fault == "boolean_count":
                        evidence["terminal_events"] = True
                    receipt.write_text(json.dumps(evidence))

                with mock.patch.object(release, "run") as run, \
                     mock.patch.object(release, "lookup_release", return_value=None):
                    with self.assertRaisesRegex(ValueError, "Inbox QA"):
                        release.publish(plan, directory)
                    run.assert_not_called()

    def test_graphical_receipt_requires_visible_citation_and_stop(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            plan = self.plan()
            self.qa_pack_pairs(directory, plan)
            target = "x86_64-unknown-linux-gnu"
            receipt = next((directory / target).glob("*.digests.json"))
            evidence = json.loads(receipt.read_text())
            del evidence["model_pid_count"]
            evidence.update(graphical="x11", first_citation_while_running=True, stop_while_running=True)
            release.require_embedded_smoke(evidence, target=target, head=plan["head"], graphical=True)
            for field in ("first_citation_while_running", "stop_while_running"):
                with self.subTest(field=field), self.assertRaisesRegex(ValueError, "Inbox QA window"):
                    release.require_embedded_smoke({**evidence, field: False}, target=target,
                                                   head=plan["head"], graphical=True)


if __name__ == "__main__":
    unittest.main()
