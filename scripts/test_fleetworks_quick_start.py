"""Exercise Quick Start rebuilds and pack admission with offline fixtures."""

from contextlib import ExitStack, contextmanager, nullcontext, redirect_stdout
from io import BytesIO, StringIO
import json
from pathlib import Path
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


class InboxArtifactTest(unittest.TestCase):
    def test_pack_compatibility_ignores_setup_documentation(self):
        revision = "a" * 40
        with patch.object(launcher, "command", side_effect=["deploy/inbox-qa.md", ""]):
            self.assertTrue(launcher.build_inputs_match(revision, local_desktop=True))
        with patch.object(launcher, "command", side_effect=["services/api-host/src/qa_worker.rs", ""]):
            self.assertFalse(launcher.build_inputs_match(revision, local_desktop=True))

    def exercise_install(self, target, *, smoke_attempt=1, smoke_digest=None, provide_smoke=True):
        revision = "a" * 40
        run_id = 23456
        run = {"id": run_id, "head_sha": revision, "status": "completed", "conclusion": "failure",
               "path": ".github/workflows/fleetworks.yml", "run_attempt": smoke_attempt}
        run_info = {"id": run_id, "head_sha": revision, "head_repository_id": 23, "repository_id": 23}
        receipt = {"target": target, "source_sha": revision, "run_id": str(run_id), "run_attempt": 1,
                   "archive_bytes": 4, "archive_sha256": "b" * 64, "manifest_sha256": "c" * 64}
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


if __name__ == "__main__":
    unittest.main()
