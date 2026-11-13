"""Offline fixtures for the Fleetworks exact-revision scope planner."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from fleetworks_verify import ALL_IMAGES, ALL_SCOPES, BACKEND_IMAGES, RUST_IMAGES, component_plan, path_images, path_scopes, select_scopes

PLANNER = Path(__file__).with_name("fleetworks_verify.py").resolve()


class PathScopesTests(unittest.TestCase):
    def test_explicit_native_is_mac_only_and_backend_excludes_web_and_clients(self):
        self.assertEqual(component_plan("native", ALL_SCOPES, ALL_IMAGES), ({"desktop"}, set()))
        scopes, images = component_plan("backend", set(), set())
        self.assertFalse(scopes & {"desktop", "web", "web_dev"})
        self.assertEqual(images, set(BACKEND_IMAGES))
        self.assertEqual(component_plan("all", set(), set()), (set(ALL_SCOPES), set(ALL_IMAGES)))

    def test_scope_selection_by_path(self):
        cases = {
            "README.md": set(),
            "CHANGELOG.md": set(),
            "docs/wiki/code/ci.md": set(),
            "docs/diagrams/stack.svg": set(),
            ".planning/example/task_plan.md": set(),
            "apps/fleetworks-desktop/src/views/brain.rs": {"desktop"},
            "apps/fleetworks-desktop/resources/Info.plist": {"desktop"},
            "scripts/fleetworks_native_verify.py": {"desktop"},
            "scripts/test_fleetworks_native_verify.py": {"desktop"},
            ".github/workflows/fleetworks-native.yml": {"desktop"},
            "apps/web-desktop/server.mjs": {"web", "web_dev", "server", "compose", "images"},
            "apps/web-desktop/app/page.tsx": {"web", "server", "compose", "images"},
            "apps/web-desktop/app/api/v1/route.ts": {"web", "web_dev", "server", "compose", "images"},
            "tests/web/intel.spec.ts": {"web", "images"},
            "deploy/Dockerfile": {"server", "compose", "images"},
            "deploy/docker-bake.hcl": {"server", "compose", "images"},
            "deploy/Dockerfile.dockerignore": {"server", "compose", "images"},
            "deploy/compose.yml": {"web", "web_dev", "server", "compose"},
            "deploy/compose.dev.yml": {"web", "web_dev", "server", "compose"},
            "deploy/compose.native-review.yml": {"server", "compose"},
            "deploy/start-search-index.sh": {"server", "compose", "images"},
            "deploy/compose.atlas.yml": {"server", "compose", "bert"},
            "crates/argos-osint-core/src/storage/bootstrap.rs": {"server", "storage", "queue", "images"},
            "services/api-host/src/main.rs": {"server", "storage", "queue", "images"},
            "services/api-host/src/actor_seed.rs": {"server", "storage", "queue", "images"},
            "services/api-host/src/atlas_worker.rs": {"server", "storage", "queue", "images"},
            "services/search-index/src/schema.rs": {"server", "images"},
            "services/search-index/src/query.rs": {"server", "images"},
            "services/search-index/tests/index_contract.rs": {"server", "images"},
            "crates/argos-osint-api/src/lib.rs": {"server", "queue", "images"},
            "crates/fleetworks-jobs/src/queue.rs": {"server", "storage", "queue", "images"},
            "scripts/fleetworks_atlas_smoke.py": {"server", "queue"},
            "services/api-host/src/classifier_service.rs": {"server", "queue", "bert", "images"},
            "crates/argos-osint-core/src/classifier/model.rs": {"server", "queue", "bert", "images"},
            "tools/atlas-bert/src/main.rs": {"bert"},
            "scripts/fixtures/atlas_bert": {"bert", "server", "queue", "web", "web_dev"},
            ".github/workflows/atlas-bert.yml": {"bert"},
            "Cargo.lock": {"desktop", "server", "storage", "compose", "queue", "images"},
            "pnpm-lock.yaml": {"web", "web_dev", "server", "compose", "images"},
            "crates/fleetworks-contracts/src/lib.rs": {"desktop", "server", "storage", "compose", "queue", "images"},
            "crates/fleetworks-workstation/src/native/mod.rs": {"desktop", "server", "storage", "compose", "queue", "images"},
            "crates/fleetworks-workspace/src/native/mod.rs": {"desktop", "server", "storage", "compose", "queue", "images"},
            "packages/api-client/src/index.ts": {"web", "server", "compose", "images"},
            "scripts/surrealdb-release.json": {"server", "storage", "queue"},
            ".github/workflows/fleetworks.yml": set(),
            ".github/workflows/embedded.yml": set(),
            "scripts/fleetworks_verify.py": set(),
            "scripts/fleetworks_config_reset.py": set(),
            "scripts/test_fleetworks_config_reset.py": set(),
            "new/unknown/build input": set(ALL_SCOPES),
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertEqual(path_scopes(path), expected)

    def test_images_are_backend_and_optional_web_only(self):
        self.assertEqual(ALL_IMAGES, {"runtime", "surrealdb", "search-index", "web"})
        self.assertNotIn("native", ALL_SCOPES)
        self.assertEqual(path_images("apps/web-desktop/app/page.tsx"), {"web"})
        self.assertEqual(path_images("deploy/Dockerfile"), ALL_IMAGES)
        self.assertEqual(path_images("README.md"), set())
        self.assertEqual(path_images("apps/fleetworks-desktop/src/views/brain.rs"), set())

    def test_search_and_backend_inputs_select_only_affected_linux_images(self):
        for path in ("services/search-index/src/engine.rs", "services/search-index/src/schema.rs", "services/search-index/tests/index_contract.rs", "deploy/check-search.sh", "deploy/start-search-index.sh"):
            with self.subTest(path=path):
                self.assertEqual(path_images(path), {"search-index"})
        self.assertEqual(path_images("services/api-host/src/actor_index_worker.rs"), {"runtime"})
        self.assertEqual(path_images("crates/argos-osint-core/src/storage/actor_search_outbox.rs"), {"runtime"})
        self.assertEqual(path_images("deploy/check-api.sh"), {"runtime"})
        self.assertEqual(path_images("deploy/start-surreal.sh"), {"surrealdb"})
        self.assertEqual(path_images("Cargo.lock"), set(RUST_IMAGES))
        self.assertEqual(path_images("crates/fleetworks-contracts/src/actor_index.rs"), set(RUST_IMAGES))
        for crate in ("fleetworks-workstation", "fleetworks-workspace"):
            self.assertEqual(path_images(f"crates/{crate}/src/native/mod.rs"), set(RUST_IMAGES))
        self.assertEqual(path_images("deploy/Dockerfile.dockerignore"), ALL_IMAGES)
        self.assertEqual(component_plan("backend", set(), set())[1], {"runtime", "surrealdb", "search-index"})


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

    def plan(self, base, event="push", head=None, mode="plan"):
        self.output.unlink(missing_ok=True)
        command = [
            sys.executable, str(PLANNER), mode, "--event", event, "--base", base,
            "--github-output", str(self.output),
        ]
        if head is not None:
            command += ["--head", head]
        return subprocess.run(command, cwd=self.repo, capture_output=True, text=True, check=False)

    def assert_scopes(self, run, expected, *, full=False, images=()):
        self.assertEqual(run.returncode, 0, run.stderr)
        expected_outputs = {
            name: str(name in expected).lower() for name in ALL_SCOPES
        }
        expected_outputs["full"] = str(full).lower()
        expected_outputs["image_targets"] = ",".join(sorted(images))
        expected_outputs["image_arch_json"] = json.dumps(["amd64", "arm64"] if full else ["amd64"])
        actual_outputs = dict(
            line.split("=", 1) for line in self.output.read_text(encoding="utf-8").splitlines()
        )
        self.assertEqual(actual_outputs, expected_outputs)

    def test_union_includes_each_changed_path_with_spaces(self):
        head = self.commit("services/api-host/src/job list.rs", "deploy/start.sh")
        uncommitted = self.repo / "unknown.txt"
        uncommitted.write_text("Ignore working tree changes\n", encoding="utf-8")
        self.git("add", str(uncommitted))
        self.assert_scopes(
            self.plan(self.start, head=head),
            {"server", "queue", "compose", "images"},
            images={"runtime"},
        )

    def test_pr_uses_merge_base_and_push_compares_both_revisions(self):
        base = self.commit("services/api-host/src/old.rs")
        self.git("checkout", "-qb", "review", self.start)
        head = self.commit("docs/review.md")
        self.assert_scopes(self.plan(base, event="pull_request", head=head), set())
        self.assert_scopes(self.plan(base, event="push", head=head), {"server", "queue", "images"}, images={"runtime"})

    def test_rename_selects_both_paths(self):
        base = self.commit("services/api-host/jobs.rs")
        (self.repo / "deploy").mkdir()
        self.git("mv", "services/api-host/jobs.rs", "deploy/jobs.rs")
        self.git("commit", "-qm", "rename fixture")
        self.assert_scopes(self.plan(base), {"server", "queue", "compose", "images"}, images={"runtime"})

    def test_unavailable_comparison_selects_all_scopes(self):
        for base in ("", "0" * 40, "1" * 40, "HEAD^"):
            with self.subTest(base=base):
                self.assert_scopes(self.plan(base), ALL_SCOPES, images=ALL_IMAGES)
        self.assert_scopes(self.plan(self.start, event="workflow_dispatch"), ALL_SCOPES, images=ALL_IMAGES)
        self.assert_scopes(self.plan(self.start, event="schedule"), ALL_SCOPES, images=ALL_IMAGES)

    def test_unknown_committed_build_input_selects_checks_and_all_images(self):
        head = self.commit("new-engine/Cargo.toml")
        self.assert_scopes(self.plan(self.start, head=head), ALL_SCOPES, images=ALL_IMAGES)

    def test_pr_and_full_produce_distinct_hosted_plans(self):
        head = self.commit("apps/web-desktop/app/page.tsx")
        self.assert_scopes(self.plan(self.start, head=head, mode="pr"), {"web", "server", "compose", "images"}, images={"web"})
        self.assert_scopes(self.plan(self.start, head=head, mode="full"), ALL_SCOPES, full=True, images=ALL_IMAGES)

    def test_workflow_edit_does_not_select_unrelated_lanes(self):
        head = self.commit(".github/workflows/fleetworks.yml", ".github/workflows/embedded.yml")
        self.assert_scopes(self.plan(self.start, head=head), set())

    def test_search_schema_selects_its_image_without_native_or_web_actions(self):
        head = self.commit("services/search-index/src/schema.rs", "services/search-index/tests/index_contract.rs")
        self.assert_scopes(self.plan(self.start, head=head), {"server", "images"}, images={"search-index"})

    def test_local_selection_adds_worktree_changes(self):
        file = self.repo / "apps/fleetworks-desktop/src/jobs.rs"
        file.parent.mkdir(parents=True)
        file.write_text("local changes\n", encoding="utf-8")
        current = Path.cwd()
        try:
            os.chdir(self.repo)
            self.assertEqual(select_scopes("push", self.start, self.start), (set(), None))
            self.assertEqual(select_scopes("push", self.start, self.start, worktree=True), ({"desktop"}, None))
        finally:
            os.chdir(current)

    def test_timing_preserves_failed_exit_and_records_revision(self):
        run = subprocess.run(
            [sys.executable, str(PLANNER), "timed", "--phase", "fixture", "--", sys.executable, "-c", "raise SystemExit(3)"],
            cwd=self.repo,
            capture_output=True,
            text=True,
            env={**os.environ, "RUNNER_TEMP": str(self.repo), "FLEETWORKS_HEAD": self.start},
        )
        self.assertEqual(run.returncode, 3, run.stderr)
        timing = json.loads((self.repo / "fleetworks-timings.jsonl").read_text(encoding="utf-8"))
        self.assertEqual((timing["revision"], timing["phase"], timing["exit"]), (self.start, "fixture", 3))
        self.assertGreaterEqual(timing["seconds"], 0)

    def test_dispatch_pins_explicit_ref_and_revision(self):
        bin_dir = self.repo / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\"\n", encoding="utf-8")
        gh.chmod(0o755)
        command = [sys.executable, str(PLANNER), "dispatch", "pr", "--ref", "fixture"]
        env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"}
        run = subprocess.run([*command, "--base", self.start], cwd=self.repo, capture_output=True, text=True, env=env)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(
            run.stdout.splitlines()[1:],
            ["workflow", "run", "fleetworks.yml", "--ref=fixture", "-f", "mode=pr", "-f", f"head={self.start}", "-f", f"base={self.start}"],
        )
        run = subprocess.run(command, cwd=self.repo, capture_output=True, text=True, env=env)
        self.assertEqual(run.returncode, 1)
        self.assertEqual(run.stdout, "")

    def test_checkout_mismatch_fails_without_outputs(self):
        self.commit("docs/other.md")
        run = self.plan(self.start, head=self.start)
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("Checkout does not match the requested head", run.stderr)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
