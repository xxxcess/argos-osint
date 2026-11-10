"""Offline fixtures for the Fleetworks exact-revision scope planner."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from fleetworks_verify import ALL_SCOPES, path_scopes

PLANNER = Path(__file__).with_name("fleetworks_verify.py").resolve()


class PathScopesTests(unittest.TestCase):
    def test_scope_selection_by_path(self):
        cases = {
            "README.md": set(),
            "CHANGELOG.md": set(),
            "docs/wiki/code/ci.md": set(),
            "docs/diagrams/stack.svg": set(),
            ".planning/example/task_plan.md": set(),
            "apps/fleetworks-cli/src/jobs.rs": {"native"},
            "apps/venture/src/main.rs": {"native", "server", "compose"},
            "apps/venture/src/classifier.rs": {"native", "server", "compose", "bert"},
            "apps/web-desktop/server.mjs": {"server", "compose"},
            "deploy/Dockerfile": {"server", "compose"},
            "deploy/compose.atlas.yml": {"server", "compose", "bert"},
            "crates/argos-osint-core/src/storage/bootstrap.rs": {"server", "broker"},
            "crates/argos-api-host/src/main.rs": {"server", "broker"},
            "crates/argos-osint-api/src/lib.rs": {"server", "broker"},
            "crates/fleetworks-broker/src/lib.rs": {"server", "broker"},
            "scripts/fleetworks_atlas_smoke.py": {"server", "broker"},
            "crates/argos-api-host/src/classifier_service.rs": {"server", "broker", "bert"},
            "crates/argos-osint-core/src/classifier/model.rs": {"server", "broker", "bert"},
            "tools/atlas-bert/src/main.rs": {"bert"},
            "scripts/fixtures/atlas_bert": {"bert"},
            ".github/workflows/atlas-bert.yml": {"bert"},
            "Cargo.lock": {"native", "server", "compose", "broker"},
            "pnpm-lock.yaml": {"native", "server", "compose", "broker"},
            "crates/fleetworks-contracts/src/lib.rs": {"native", "server", "compose", "broker"},
            "packages/api-client/src/index.ts": {"native", "server", "compose", "broker"},
            ".github/workflows/fleetworks.yml": ALL_SCOPES,
            "scripts/fleetworks_verify.py": ALL_SCOPES,
            "new/unknown/build input": ALL_SCOPES,
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertEqual(path_scopes(path), expected)


class GitScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.git("init", "-q", "--object-format=sha1", "-b", "fixture")
        self.start = self.commit("README.md")
        self.output = self.repo / "scope-output"

    def git(self, *args):
        return subprocess.check_output(
            [
                "git", "-c", "user.email=fixture@example.invalid", "-c", "user.name=Scope Fixture",
                "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args,
            ],
            cwd=self.repo,
            text=True,
        ).strip()

    def commit(self, *paths):
        for path in paths:
            file = self.repo / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(path + "\n", encoding="utf-8")
        self.git("add", "--", *paths)
        self.git("commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def plan(self, base, event="push", head=None):
        self.output.unlink(missing_ok=True)
        command = [
            sys.executable, str(PLANNER), "plan", "--event", event, "--base", base,
            "--github-output", str(self.output),
        ]
        if head is not None:
            command += ["--head", head]
        return subprocess.run(command, cwd=self.repo, capture_output=True, text=True, check=False)

    def assert_scopes(self, run, expected):
        self.assertEqual(run.returncode, 0, run.stderr)
        expected_outputs = {
            name: str(name in expected).lower() for name in ALL_SCOPES
        }
        actual_outputs = dict(
            line.split("=", 1) for line in self.output.read_text(encoding="utf-8").splitlines()
        )
        self.assertEqual(actual_outputs, expected_outputs)

    def test_union_includes_each_changed_path_with_spaces(self):
        head = self.commit("apps/fleetworks-cli/src/job list.rs", "deploy/start.sh")
        uncommitted = self.repo / "unknown.txt"
        uncommitted.write_text("Ignore working tree changes\n", encoding="utf-8")
        self.git("add", str(uncommitted))
        self.assert_scopes(self.plan(self.start, head=head), {"native", "server", "compose"})

    def test_pr_uses_merge_base_and_push_compares_both_revisions(self):
        base = self.commit("apps/fleetworks-cli/src/old.rs")
        self.git("checkout", "-qb", "review", self.start)
        head = self.commit("docs/review.md")
        self.assert_scopes(self.plan(base, event="pull_request", head=head), set())
        self.assert_scopes(self.plan(base, event="push", head=head), {"native"})

    def test_rename_selects_both_paths(self):
        base = self.commit("apps/fleetworks-cli/jobs.rs")
        (self.repo / "deploy").mkdir()
        self.git("mv", "apps/fleetworks-cli/jobs.rs", "deploy/jobs.rs")
        self.git("commit", "-qm", "rename fixture")
        self.assert_scopes(self.plan(base), {"native", "server", "compose"})

    def test_unavailable_comparison_selects_all_scopes(self):
        for base in ("", "0" * 40, "1" * 40, "HEAD^"):
            with self.subTest(base=base):
                self.assert_scopes(self.plan(base), ALL_SCOPES)
        self.assert_scopes(self.plan(self.start, event="workflow_dispatch"), ALL_SCOPES)

    def test_checkout_mismatch_fails_without_outputs(self):
        self.commit("docs/other.md")
        run = self.plan(self.start, head=self.start)
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("Checkout does not match the requested head", run.stderr)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
