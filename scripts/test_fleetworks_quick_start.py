"""Exercise Quick Start rebuilds and pack admission with offline fixtures."""

from contextlib import ExitStack, contextmanager, nullcontext, redirect_stdout
from io import BytesIO, StringIO
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from zipfile import ZipFile

import fleetworks_quick_start as launcher


class SourceQuickStartTest(unittest.TestCase):
    def setUp(self):
        resources = ExitStack()
        self.addCleanup(resources.close)
        directory = Path(resources.enter_context(tempfile.TemporaryDirectory()))
        self.output = directory / "debug"
        self.profile = directory / "profile"
        self.revision = "a" * 40
        self.target = "x86_64-apple-darwin"
        self.receipt = {
            "workflow": launcher.SOURCE_WORKFLOW,
            "revision": self.revision,
            "profile": "debug",
            "dirty": False,
            "version": "0.2.1",
            "dependency_provenance": {"cargo_lock_sha256": "b" * 64},
        }
        self.app = self.write_bundle(self.output, "old")
        resources.enter_context(patch.object(launcher, "check_bundle"))
        resources.enter_context(patch.object(launcher.native, "artifact_version", return_value="0.2.1"))
        resources.enter_context(patch.object(launcher.native, "sha256", return_value="b" * 64))
        resources.enter_context(patch.object(launcher.native, "repository_identity",
                                             return_value=(self.revision, False)))
        resources.enter_context(patch.object(launcher, "require_no_other_app"))
        resources.enter_context(patch.object(launcher, "app_pid", return_value=12))

    def write_bundle(self, output, contents):
        app = output / launcher.native.NATIVE_APP
        app.mkdir(parents=True)
        (app / "fixture").write_text(contents)
        (output / launcher.native.NATIVE_MANIFEST).write_text(json.dumps(self.receipt))
        return app

    def prepare_source(self):
        return launcher.prepare_source(self.target, self.output, self.app, self.receipt,
                                       launcher.REPOSITORY, "debug", self.profile)

    def stage_bundle(self, staged, revision, shared, profile):
        self.assertEqual((revision, shared, profile), (self.revision, launcher.REPOSITORY, "debug"))
        self.assertEqual((self.app / "fixture").read_text(), "old")
        self.write_bundle(staged, "new")

    def test_matching_receipt_still_builds_and_waits_for_the_companion(self):
        with patch.object(launcher, "build_source", self.stage_bundle), \
                patch.object(launcher, "executable_pid", return_value=13), \
                patch.object(launcher, "embedded_running", return_value=True) as embedded, \
                patch.object(launcher, "bundle_pid", side_effect=[12, 13, None, None]) as running, \
                patch.object(launcher.time, "sleep"), \
                patch.object(launcher.subprocess, "run") as quit_process:
            self.assertEqual(self.prepare_source(), self.app)
        quit_process.assert_called_once()
        self.assertEqual(quit_process.call_args.args[0], ["osascript", "-", "12", str(self.app)])
        embedded.assert_called_once_with(
            self.app, 12, self.profile / ".fleetworks" / "local-backend" / "embedded")
        self.assertEqual(running.call_count, 4)
        self.assertEqual((self.app / "fixture").read_text(), "new")

    def test_failed_build_keeps_the_installed_process_and_bundle(self):
        with patch.object(launcher, "build_source", side_effect=ValueError("build failed")), \
                patch.object(launcher, "close_app") as close:
            with self.assertRaisesRegex(ValueError, "build failed"):
                self.prepare_source()
        close.assert_not_called()
        self.assertEqual((self.app / "fixture").read_text(), "old")

    def test_same_bundle_with_another_profile_is_left_running(self):
        with patch.object(launcher, "build_source", self.stage_bundle), \
                patch.object(launcher, "executable_pid", return_value=None), \
                patch.object(launcher, "app_profile_matches", return_value=False), \
                patch.object(launcher.subprocess, "run") as quit_process:
            with self.assertRaisesRegex(ValueError, "different profile"):
                self.prepare_source()
        quit_process.assert_not_called()
        self.assertEqual((self.app / "fixture").read_text(), "old")

    def test_default_source_is_the_invoking_worktree(self):
        worktree = self.output.parent / "worktree"
        repository = self.output.parent / "repository"
        scripts = worktree / "scripts"
        scripts.mkdir(parents=True)
        for helper in ("fleetworks_candidate.py", "fleetworks_qa_pack.py"):
            (scripts / helper).touch()
        with patch.object(launcher, "SCRIPT_CHECKOUT", worktree), \
                patch.object(sys, "argv", ["fleetworks_quick_start.py", "--start-embedded"]), \
                patch.object(launcher, "git_path", side_effect=lambda checkout, option: (
                    repository / ".git" if option == "--git-common-dir" else checkout)):
            self.assertEqual(launcher.source_checkouts(), (worktree, repository))


class BuildSelectionTest(unittest.TestCase):
    def test_empty_input_uses_nightly(self):
        with patch("builtins.input", return_value=""):
            self.assertEqual(launcher.prompt_source(), "nightly")

    def test_pr_picker_lists_open_nightly_prs_and_uses_the_pr_number(self):
        with patch("builtins.input", side_effect=["pr", "1", "78"]), \
                patch.object(launcher, "command", return_value='\n'.join([
                    json.dumps({"number": 78, "title": "Assistant"}),
                    json.dumps({"number": 77, "title": "Performance"}),
                ])) as gh, redirect_stdout(StringIO()) as output:
            self.assertEqual(launcher.prompt_source(), "#78")
        self.assertIn("#78  Assistant", output.getvalue())
        self.assertIn("#77  Performance", output.getvalue())
        self.assertIn("Select a PR number from the list.", output.getvalue())
        self.assertIn("--paginate", gh.call_args.args)
        self.assertIn("state=open&base=nightly", gh.call_args.args[3])

    def test_explicit_source_flags_bypass_the_prompt(self):
        class SourceSelected(Exception):
            pass

        info = SimpleNamespace(native_target="x86_64-apple-darwin")
        cases = [(["--nightly"], "nightly"), (["--pr", "78"], "#78"),
                 (["--commit", "A" * 40], "a" * 40)]
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "profile"
            for flags, expected in cases:
                with self.subTest(flags=flags), \
                        patch.object(sys, "argv", ["quick-start", "--profile-dir", str(profile), *flags]), \
                        patch.object(launcher.native, "detect", return_value=info), \
                        patch.object(launcher, "quick_start_lock", return_value=nullcontext()), \
                        patch.object(launcher, "require_no_other_app"), \
                        patch.object(launcher, "prompt_source", side_effect=AssertionError("unexpected prompt")), \
                        patch.object(launcher, "checkout_build_source", side_effect=SourceSelected) as checkout, \
                        redirect_stdout(StringIO()):
                    with self.assertRaises(SourceSelected):
                        launcher.main()
                    checkout.assert_called_once_with(launcher.REPOSITORY, expected)

    def test_fork_pr_resolves_only_the_verified_head(self):
        sha = "a" * 40
        pull = {"number": 78, "state": "open", "base": {"ref": "nightly",
                "repo": {"full_name": launcher.REPO}}, "head": {"sha": sha}}
        shared = Path("/repo")
        with patch.object(launcher, "gh_json", return_value=pull), \
                patch.object(launcher, "fetch_build_revision", return_value=sha) as fetch:
            self.assertEqual(launcher.build_revision(shared, "#78"), sha)
        fetch.assert_called_once_with(shared, "refs/pull/78/head", remote=f"https://github.com/{launcher.REPO}.git")
        with patch.object(launcher, "gh_json", return_value={**pull, "state": "closed"}), \
                patch.object(launcher, "fetch_build_revision") as fetch:
            with self.assertRaisesRegex(ValueError, "must be open"):
                launcher.build_revision(shared, "#78")
        fetch.assert_not_called()
        with patch.object(launcher, "gh_json", return_value=pull), \
                patch.object(launcher, "fetch_build_revision", return_value="b" * 40):
            with self.assertRaisesRegex(ValueError, "changed while fetching"):
                launcher.build_revision(shared, "#78")

    def test_switches_build_worktree_and_keeps_editing_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory).resolve()
            repository = directory / "source"
            repository.mkdir()
            origin = directory / "origin"
            origin.mkdir()
            git = launcher.git_at
            git(origin, "init", "--quiet", "--bare")
            git(repository, "init", "--quiet", "-b", "nightly")
            git(repository, "config", "user.name", "Quick Start Fixture")
            git(repository, "config", "user.email", "fixture@example.invalid")
            git(repository, "config", "commit.gpgsign", "false")
            git(repository, "config", "core.hooksPath", "/dev/null")
            git(repository, "remote", "add", "origin", str(origin))
            content = repository / "source.txt"
            content.write_text("nightly")
            git(repository, "add", "source.txt")
            git(repository, "commit", "--quiet", "-m", "nightly")
            nightly = git(repository, "rev-parse", "HEAD")
            git(repository, "push", "--quiet", "origin", "nightly")
            git(repository, "switch", "--quiet", "-c", "editing")
            content.write_text("selected commit")
            git(repository, "commit", "--quiet", "-am", "selected")
            selected = git(repository, "rev-parse", "HEAD")
            content.write_text("in-progress edits")

            with redirect_stdout(StringIO()):
                checkout = launcher.checkout_build_source(repository, selected[:12])
                self.assertEqual(git(checkout, "rev-parse", "HEAD"), selected)
                self.assertEqual(launcher.checkout_build_source(repository, "nightly"), checkout)
                self.assertEqual(git(checkout, "rev-parse", "HEAD"), nightly)
                self.assertEqual((checkout / "source.txt").read_text(), "nightly")
                self.assertEqual(git(repository, "rev-parse", "--abbrev-ref", "HEAD"), "editing")
                self.assertEqual(content.read_text(), "in-progress edits")

                (checkout / "source.txt").write_text("worktree edit")
                with self.assertRaisesRegex(ValueError, "worktree has local changes"):
                    launcher.checkout_build_source(repository, selected)
                self.assertEqual((checkout / "source.txt").read_text(), "worktree edit")


def inbox_artifact_fixture(target, *, smoke_attempt=1, smoke_digest=None, provide_smoke=True,
                           archive_digest=None):
    revision = "a" * 40
    run_id = 23456
    run = {"id": run_id, "head_sha": revision, "status": "completed", "conclusion": "failure",
           "path": ".github/workflows/fleetworks.yml", "run_attempt": smoke_attempt}
    run_info = {"id": run_id, "head_sha": revision, "head_repository_id": 23, "repository_id": 23}
    receipt = {"target": target, "source_sha": revision, "run_id": str(run_id), "run_attempt": 1,
               "archive_bytes": 4, "archive_sha256": archive_digest or "b" * 64, "manifest_sha256": "c" * 64}
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

    return gh_json, download, receipt


class InboxArtifactTest(unittest.TestCase):
    def test_pack_compatibility_ignores_setup_documentation(self):
        revision = "a" * 40
        with patch.object(launcher, "command", side_effect=["deploy/inbox-qa.md", ""]):
            self.assertTrue(launcher.build_inputs_match(revision, local_desktop=True))
        with patch.object(launcher, "command", side_effect=["services/api-host/src/qa_worker.rs", ""]):
            self.assertFalse(launcher.build_inputs_match(revision, local_desktop=True))

    def exercise_install(self, target, *, smoke_attempt=1, smoke_digest=None, provide_smoke=True):
        gh_json, download, receipt = inbox_artifact_fixture(
            target, smoke_attempt=smoke_attempt, smoke_digest=smoke_digest, provide_smoke=provide_smoke,
        )
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


class AutomaticInboxTest(unittest.TestCase):
    target = "x86_64-apple-darwin"
    revision = "a" * 40

    def setUp(self):
        resources = ExitStack()
        self.addCleanup(resources.close)
        directory = Path(resources.enter_context(tempfile.TemporaryDirectory())).resolve()
        self.profile = directory / "profile"
        self.profile.mkdir(mode=0o700)
        self.state = self.profile / ".fleetworks" / "local-backend" / "embedded"
        for parent in (self.state.parent.parent, self.state.parent, self.state):
            parent.mkdir(mode=0o700)
        self.output = directory / "debug"
        self.app = self.output / launcher.native.NATIVE_APP
        self.app.mkdir(parents=True)
        self.receipt = {
            "workflow": launcher.SOURCE_WORKFLOW,
            "revision": self.revision,
            "profile": "debug",
            "dirty": False,
            "version": "0.2.1",
        }
        (self.output / launcher.native.NATIVE_MANIFEST).write_text(json.dumps(self.receipt))

    def sync(self):
        return launcher.sync_inbox(self.target, self.revision, self.profile, self.state)

    def test_unavailable_candidates_leave_the_existing_pointer_untouched(self):
        pointer = self.state / "qa-pack.json"
        previous = {"root": str(self.profile / "qa-pack-7"), "manifest_sha256": "b" * 64}
        pointer.write_text(json.dumps(previous))
        pointer.chmod(0o600)
        unavailable = (
            (subprocess.CalledProcessError(1, "gh api"), None),
            (None, ValueError("No verified Embedded Inbox smoke receipt")),
        )
        for discovery_error, install_error in unavailable:
            with self.subTest(discovery=discovery_error, install=install_error), \
                    patch.object(launcher, "discover_inbox_artifacts", return_value=iter([8]),
                                 side_effect=discovery_error), \
                    patch.object(launcher, "install_inbox", side_effect=install_error) as install, \
                    redirect_stdout(StringIO()):
                self.assertFalse(self.sync())
                self.assertEqual(json.loads(pointer.read_text()), previous)
                if discovery_error is not None:
                    install.assert_not_called()
                else:
                    self.assertEqual([entry.args[0] for entry in install.call_args_list], [8])

    def test_sync_advances_past_unverified_candidate(self):
        with patch.object(launcher, "discover_inbox_artifacts", return_value=iter([8, 9])), \
                patch.object(launcher, "install_inbox", side_effect=[ValueError("smoke did not match"), None]) as install, \
                redirect_stdout(StringIO()):
            self.assertTrue(self.sync())
        self.assertEqual([entry.args[0] for entry in install.call_args_list], [8, 9])
        self.assertTrue(all(entry.kwargs.get("automatic") is True for entry in install.call_args_list))

    def write_pointer(self, identity, digest):
        pack = self.profile / f"qa-pack-{identity}"
        pack.mkdir(mode=0o700)
        pointer = self.state / "qa-pack.json"
        configuration = {"root": str(pack), "manifest_sha256": digest}
        pointer.write_text(json.dumps(configuration))
        pointer.chmod(0o600)
        return pointer, configuration

    def test_verified_identical_digest_does_not_download_or_replace_the_pack(self):
        gh_json, download, receipt = inbox_artifact_fixture(self.target)
        pointer, previous = self.write_pointer(7, receipt["manifest_sha256"])
        with patch.object(launcher, "gh_json", gh_json), \
                patch.object(launcher, "download_artifact", wraps=download) as acquire, \
                patch.object(launcher, "build_inputs_match", return_value=True), \
                patch.object(launcher.qa, "verify_directory") as verify, \
                patch.object(launcher.qa, "install") as install:
            self.assertFalse(launcher.install_inbox(
                1, self.target, self.profile, self.state, automatic=True,
            ))
        self.assertEqual([entry.args[0]["id"] for entry in acquire.call_args_list], [2])
        verify.assert_called_once_with(Path(previous["root"]), self.target, receipt["manifest_sha256"])
        install.assert_not_called()
        self.assertEqual(json.loads(pointer.read_text()), previous)
        self.assertFalse((self.profile / "qa-pack-1").exists())

    def test_automatic_replacement_waits_for_the_smoke_and_archive_verification(self):
        pointer, previous = self.write_pointer(7, "e" * 64)
        correct_archive = hashlib.sha256(b"pack").hexdigest()
        for archive_digest, smoke_digest, fail_manifest, error in [
            ("b" * 64, "d" * 64, False, "do not match the verified Embedded smoke"),
            ("b" * 64, None, False, "pack archive SHA-256 mismatch"),
            (correct_archive, None, True, "pack manifest SHA-256 mismatch"),
            (correct_archive, None, False, None),
        ]:
            with self.subTest(archive_digest=archive_digest, smoke_digest=smoke_digest,
                              fail_manifest=fail_manifest):
                gh_json, download, receipt = inbox_artifact_fixture(
                    self.target, archive_digest=archive_digest, smoke_digest=smoke_digest,
                )

                def verify(root, target, digest):
                    self.assertEqual(json.loads(pointer.read_text()), previous)
                    self.assertEqual(target, self.target)
                    self.assertEqual(digest, receipt["manifest_sha256"])
                    if fail_manifest:
                        raise ValueError("pack manifest SHA-256 mismatch")
                    return {"target": target, "model_revision": launcher.qa.MODEL_REVISION, "files": {}}

                with patch.object(launcher, "gh_json", gh_json), \
                        patch.object(launcher, "download_artifact", download), \
                        patch.object(launcher, "build_inputs_match", return_value=True), \
                        patch.object(launcher, "reserve_space"), \
                        patch.object(launcher.qa, "extract") as extract, \
                        patch.object(launcher.qa, "verify_directory", side_effect=verify) as verified:
                    if error:
                        with self.assertRaisesRegex(ValueError, error):
                            launcher.install_inbox(1, self.target, self.profile, self.state, automatic=True)
                        self.assertEqual(extract.call_count, int(fail_manifest))
                        self.assertEqual(verified.call_count, int(fail_manifest))
                        self.assertEqual(json.loads(pointer.read_text()), previous)
                        self.assertFalse((self.profile / "qa-pack-1").exists())
                    else:
                        self.assertTrue(launcher.install_inbox(
                            1, self.target, self.profile, self.state, automatic=True,
                        ))
                        extract.assert_called_once()
                        verified.assert_called()
                        self.assertEqual(json.loads(pointer.read_text()), {
                            "root": str(self.profile / "qa-pack-1"),
                            "manifest_sha256": receipt["manifest_sha256"],
                        })
                        self.assertEqual(pointer.stat().st_mode & 0o777, 0o600)
                        self.assertTrue(Path(previous["root"]).is_dir())

    def test_running_profile_rejects_an_explicit_switch_before_pack_download(self):
        gh_json, download, _ = inbox_artifact_fixture(self.target)
        pointer, previous = self.write_pointer(7, "e" * 64)
        with patch.object(launcher, "gh_json", gh_json), \
                patch.object(launcher, "download_artifact", wraps=download) as acquire, \
                patch.object(launcher, "build_inputs_match", return_value=True), \
                patch.object(launcher.qa, "install") as install:
            with self.assertRaisesRegex(ValueError, "Quit Fleetworks before switching"):
                launcher.install_inbox(1, self.target, self.profile, self.state, replace_existing=False)
        self.assertEqual([entry.args[0]["id"] for entry in acquire.call_args_list], [2])
        install.assert_not_called()
        self.assertEqual(json.loads(pointer.read_text()), previous)

    def test_cached_installation_is_adopted_only_after_directory_verification(self):
        gh_json, download, receipt = inbox_artifact_fixture(self.target)
        pointer, previous = self.write_pointer(7, "e" * 64)
        cached = self.profile / "qa-pack-1"
        cached.mkdir(mode=0o700)
        with patch.object(launcher, "gh_json", gh_json), \
                patch.object(launcher, "download_artifact", wraps=download) as acquire, \
                patch.object(launcher, "build_inputs_match", return_value=True), \
                patch.object(launcher.qa, "verify_directory", side_effect=[
                    ValueError("pack manifest SHA-256 mismatch"), None,
                ]) as verify, \
                patch.object(launcher.qa, "install") as install:
            with self.assertRaisesRegex(ValueError, "pack manifest SHA-256 mismatch"):
                launcher.install_inbox(1, self.target, self.profile, self.state, automatic=True)
            self.assertEqual(json.loads(pointer.read_text()), previous)
            self.assertTrue(launcher.install_inbox(1, self.target, self.profile, self.state, automatic=True))
        self.assertEqual([entry.args for entry in verify.call_args_list],
                         [(cached, self.target, receipt["manifest_sha256"])] * 2)
        self.assertEqual([entry.args[0]["id"] for entry in acquire.call_args_list], [2, 2])
        install.assert_not_called()
        self.assertEqual(json.loads(pointer.read_text()), {
            "root": str(cached), "manifest_sha256": receipt["manifest_sha256"],
        })

    def test_discovery_prioritizes_exact_source_and_only_lists_bounded_compatible_runs(self):
        compatible = "b" * 40
        incompatible = "c" * 40

        def run(identity, revision, created, *, repository=launcher.REPO, status="completed"):
            return {
                "id": identity, "path": ".github/workflows/fleetworks.yml", "status": status,
                "head_sha": revision, "run_attempt": 2, "created_at": f"2026-11-26T{created:02}:00:00Z",
                "repository": {"full_name": launcher.REPO},
                "head_repository": {"full_name": repository},
            }

        exact = run(31, self.revision, 9)
        exact_older = run(30, self.revision, 8)
        matching = run(24, compatible, 10)
        other = run(25, incompatible, 11)
        fork = run(26, self.revision, 12, repository="someone/fork")

        def artifact(identity, run, attempt, *, target=self.target, expired=False):
            return {
                "id": identity, "expired": expired,
                "name": f"Fleetworks-Inbox-QA-Pack-{target}-{run['head_sha']}-{run['id']}.{attempt}",
            }

        exact_listing = (
            "actions/workflows/fleetworks.yml/runs?status=completed"
            f"&head_sha={self.revision}&per_page=6"
        )
        recent_listing = "actions/workflows/fleetworks.yml/runs?status=completed&per_page=16"
        endpoints = {
            exact_listing: {"workflow_runs": [exact_older, fork, exact]},
            recent_listing: {"workflow_runs": [other, matching, exact, exact_older]},
            "actions/runs/31/artifacts?per_page=100": {"artifacts": [
                artifact(1, exact, 1), artifact(2, exact, 2),
                artifact(3, exact, 3),
                artifact(4, exact, 2, target="aarch64-apple-darwin"),
                artifact(5, exact, 2, expired=True),
            ]},
            "actions/runs/30/artifacts?per_page=100": {"artifacts": []},
            "actions/runs/24/artifacts?per_page=100": {"artifacts": [artifact(6, matching, 2)]},
        }
        with patch.object(launcher, "gh_json", side_effect=endpoints.__getitem__) as gh, \
                patch.object(launcher, "build_inputs_match",
                             side_effect=lambda revision, **kwargs: revision != incompatible):
            found = iter(launcher.discover_inbox_artifacts(self.target, self.revision))
            self.assertEqual(next(found), 2)
            self.assertNotIn(recent_listing, [entry.args[0] for entry in gh.call_args_list])
            self.assertEqual(list(found), [1, 6])
        self.assertEqual(set(entry.args[0] for entry in gh.call_args_list), set(endpoints))
        self.assertEqual(gh.call_count, len(endpoints))

    def run_launch(self, flags, *, running=False, fresh=False):
        events = []
        if fresh:
            self.state.rmdir()

        def open_app(app, profile, *, start_embedded):
            events.append("open")
            if fresh:
                self.assertFalse(self.state.exists(), "Quick Start must leave state creation to the companion")
                if start_embedded:
                    self.state.mkdir(mode=0o700)
            return 12, self.state

        pid = 12 if running else None
        info = SimpleNamespace(native_target=self.target)
        with patch.object(sys, "argv", [
            "quick-start", "--reuse-bundle", "--output", str(self.output),
            "--profile-dir", str(self.profile), *flags,
        ]), patch.object(launcher.native, "detect", return_value=info), \
                patch.object(launcher.native, "native_output", return_value=self.output), \
                patch.object(launcher, "quick_start_lock", return_value=nullcontext()), \
                patch.object(launcher, "require_no_other_app"), \
                patch.object(launcher, "prepare_app", return_value=self.app), \
                patch.object(launcher, "app_pid", return_value=pid), \
                patch.object(launcher, "bundle_pid", return_value=pid), \
                patch.object(launcher, "executable_pid", return_value=pid), \
                patch.object(launcher, "app_profile_matches", return_value=True), \
                patch.object(launcher, "embedded_running", side_effect=lambda app, pid, state: pid is not None), \
                patch.object(launcher, "close_app") as close, \
                patch.object(launcher, "sync_inbox", side_effect=lambda *args, **kwargs: events.append("sync") or True) as sync, \
                patch.object(launcher, "install_inbox", side_effect=lambda *args, **kwargs: events.append("explicit")) as install, \
                patch.object(launcher, "open_app", side_effect=open_app), \
                redirect_stdout(StringIO()):
            launcher.main()
        close.assert_not_called()
        return events, sync, install

    def test_default_sync_precedes_launch_only_for_a_stopped_profile(self):
        stopped, sync, _ = self.run_launch(["--start-embedded"])
        self.assertEqual(stopped, ["sync", "open"])
        sync.assert_called_once()
        self.assertIn(self.revision, sync.call_args.args)
        self.assertIn(self.profile, sync.call_args.args)

        active, sync, _ = self.run_launch(["--start-embedded"], running=True)
        self.assertEqual(active, ["open"])
        sync.assert_not_called()

    def test_first_embedded_launch_waits_for_the_companion_to_create_its_state(self):
        events, sync, _ = self.run_launch(["--start-embedded"], fresh=True)
        self.assertEqual(events, ["open", "sync"])
        sync.assert_called_once()

    def test_opt_out_skips_sync_and_explicit_install_remains_strict(self):
        opted_out, sync, install = self.run_launch(["--start-embedded", "--no-inbox-sync"])
        self.assertEqual(opted_out, ["open"])
        sync.assert_not_called()
        install.assert_not_called()

        explicit, sync, install = self.run_launch(["--no-inbox-sync", "--inbox-artifact", "42"])
        self.assertEqual(explicit, ["explicit", "open"])
        sync.assert_not_called()
        install.assert_called_once()
        self.assertEqual(install.call_args.args[0], 42)
        self.assertFalse(install.call_args.kwargs.get("automatic", False))


if __name__ == "__main__":
    unittest.main()
