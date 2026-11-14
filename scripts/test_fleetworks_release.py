"""Offline Git fixtures for source-bound channel version and scope planning."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from fleetworks_release import artifact_identity, commit_impact, main, plan_release


class ReleasePlanTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init", "--quiet", "--initial-branch=dev")
        self.git("config", "user.name", "Synthetic Release Fixture")
        self.git("config", "user.email", "release@example.invalid")
        (self.root / "version.txt").write_text("0.2.1\n")
        self.git("add", "version.txt")
        self.git("commit", "--quiet", "-m", "chore: fixture baseline")
        self.baseline = self.head()
        self.git("tag", "v0.2.1")
        self.initial_artifact = artifact_identity("dev", (0, 2, 1), self.baseline, 1)["tag"]
        self.git("tag", self.initial_artifact)

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True, stderr=subprocess.PIPE).strip()

    def head(self):
        return self.git("rev-parse", "HEAD")

    def commit(self, message):
        self.git("commit", "--quiet", "--allow-empty", "-m", message)
        return self.head()

    def scope(self, *, head=None, base=None, event="push", scopes=None, images=None):
        return {
            "format": 1, "head": head or self.head(), "base": base if base is not None else self.baseline,
            "event": event, "scopes": scopes if scopes is not None else ["desktop", "images", "server"],
            "image_targets": images if images is not None else ["runtime", "search-index"],
            "full": False, "image_arches": ["amd64", "arm64"], "components": "auto", "reason": None,
        }

    def plan(self, *, branch="dev", event="push", base=None, scope=None):
        return plan_release(self.root, event=event, ref=f"refs/heads/{branch}", head=self.head(),
                            base=base if base is not None else self.baseline,
                            scope=scope if scope is not None else self.scope(event=event, base=base))

    def publish_tag(self, plan):
        self.git("tag", plan["tag"])

    def test_feature_fix_and_breaking_follow_workspace_policy(self):
        for message, baseline, impact in [
            ("feat(native): add a capability", (0, 2, 1), "minor"),
            ("fix(api): correct behavior", (0, 2, 1), "patch"),
            ("perf: reduce allocations", (0, 2, 1), "patch"),
            ("ci: maintain checks", (0, 2, 1), "none"),
            ("feat(api)!: replace a contract\n\nBREAKING CHANGE: Consumers migrate.", (0, 2, 1), "minor"),
            ("fix(api)!: replace a contract\n\nBREAKING-CHANGE: Consumers migrate.", (1, 2, 1), "major"),
            ("docs: changed contract\n\nBREAKING CHANGE: Consumers migrate.", (1, 2, 1), "major"),
        ]:
            with self.subTest(message=message):
                self.assertEqual(commit_impact(message, baseline), impact)
        for message in ("feat!: broken", "feat: invalid\n\nBREAKING CHANGE:"):
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, "BREAKING CHANGE"):
                commit_impact(message, (0, 2, 1))
        with self.assertRaisesRegex(ValueError, "Conventional Commits"):
            commit_impact("WIP ordinary commit", (0, 2, 1))
        self.assertEqual(commit_impact("Merge pull request #1 from example/feature", (0, 2, 1), merge=True), "none")
        self.assertEqual(commit_impact("feat: preserve conventional merge title", (0, 2, 1), merge=True), "minor")
        self.assertEqual(commit_impact("Merge pull request #1 from example/feature\n\nfeat: preserved PR title", (0, 2, 1), merge=True), "minor")
        self.assertEqual(commit_impact("Merge branch 'feature'\n\nBREAKING CHANGE: Callers migrate.", (1, 0, 0), merge=True), "major")
        with self.assertRaisesRegex(ValueError, "BREAKING CHANGE"):
            commit_impact("Merge pull request #1 from example/feature\n\nfeat!: missing migration footer", (0, 2, 1), merge=True)

    def test_untyped_legacy_history_has_conservative_patch_impact_and_a_count(self):
        self.commit("Keep diagnostic headroom for a server fixture")
        self.commit("Preserve a historical workflow correction")
        result = self.plan()
        self.assertEqual(result["impact"], "patch")
        self.assertTrue(result["version"].startswith("0.2.2+dev.g"))
        self.assertEqual(result["untyped_commits"], 2)
        self.assertLessEqual(result["untyped_commits"], result["commits"])
        self.assertEqual(self.git("log", "-1", "--format=%s"), "Preserve a historical workflow correction")

    def test_untyped_legacy_breaking_footer_is_not_downgraded_to_patch(self):
        (self.root / "version.txt").write_text("1.0.0\n")
        self.git("add", "version.txt")
        self.commit("Historical contract replacement\n\nBREAKING CHANGE: Callers must migrate.")
        result = self.plan()
        self.assertEqual(result["impact"], "major")
        self.assertTrue(result["version"].startswith("2.0.0+dev.g"))
        self.assertEqual(result["untyped_commits"], 1)

    def test_real_mechanical_merge_preserves_pr_title_and_breaking_body(self):
        self.git("checkout", "--quiet", "-b", "feature")
        self.commit("fix: prepare a replacement contract")
        self.git("checkout", "--quiet", "dev")
        self.commit("docs: keep both branch parents")
        self.git("merge", "--quiet", "--no-ff", "feature", "-m",
                 "Merge pull request #42 from example/feature\n\nfeat(api)!: replace contract\n\nBREAKING CHANGE: Callers migrate.")
        self.assertEqual(len(self.git("show", "-s", "--format=%P", "HEAD").split()), 2)
        result = self.plan()
        self.assertEqual(result["impact"], "minor")
        self.assertTrue(result["version"].startswith("0.3.0+dev.g"))
        self.assertEqual(result["untyped_commits"], 0)

    def test_stable_artifact_version_and_retry_bind_exact_source(self):
        head = self.commit("feat: selected native capability")
        self.commit("fix: selected service correction")
        head = self.head()
        first = self.plan()
        self.assertTrue(first["publish"])
        self.assertEqual(first["version"], f"0.3.0+dev.g{head}")
        self.assertEqual(first["image_tag"], f"stable-dev-0.3.0-g{head}")
        self.assertEqual(first["channel"], "stable")
        self.assertFalse(first["prerelease"])
        self.assertTrue(first["native"] and first["backend"])
        self.assertFalse(first["web"])
        self.assertEqual(first["commits"], 2)
        self.assertEqual(self.plan(), first, "Retry before publication is deterministic")
        self.publish_tag(first)
        retry = self.plan()
        for key in ("tag", "version", "image_tag", "channel", "head"):
            self.assertEqual(retry[key], first[key])
        self.assertEqual(retry["impact"], "reused")
        self.assertEqual((self.root / "version.txt").read_text(), "0.2.1\n")

    def test_dev_to_main_promotion_does_not_bump_already_released_commits(self):
        published = self.commit("feat: shared capability")
        dev = self.plan()
        self.publish_tag(dev)
        self.git("checkout", "--quiet", "-b", "main")
        self.commit("chore: promote reviewed dev work")
        main_plan = self.plan(branch="main", base=published)
        self.assertTrue(main_plan["version"].startswith("0.3.0+main.g"))
        self.assertEqual(main_plan["impact"], "none")
        self.assertEqual(main_plan["commits"], 1)
        self.assertNotEqual(main_plan["tag"], dev["tag"])
        self.publish_tag(main_plan)
        base = self.head()
        self.commit("fix: correct main behavior")
        self.assertTrue(self.plan(branch="main", base=base)["version"].startswith("0.3.1+main.g"))

    def test_nightly_is_prerelease_and_does_not_hide_features_from_stable(self):
        self.commit("feat: preview capability")
        nightly = self.plan(branch="nightly")
        self.assertTrue(nightly["version"].startswith("0.3.0-nightly.2.g"))
        self.assertEqual(nightly["channel"], "nightly")
        self.assertTrue(nightly["prerelease"])
        self.publish_tag(nightly)
        stable = self.plan()
        self.assertTrue(stable["version"].startswith("0.3.0+dev.g"))
        self.assertEqual(stable["impact"], "minor")
        self.publish_tag(stable)
        before = self.head()
        self.commit("ci: improve the runtime build")
        later = self.plan(branch="nightly", base=before)
        self.assertTrue(later["version"].startswith("0.3.0-nightly.3.g"))
        self.assertNotEqual(later["version"], nightly["version"])

    def test_publishing_requires_trusted_push_and_selected_artifacts(self):
        self.commit("ci: update checks")
        for event in ("pull_request", "workflow_dispatch", "schedule"):
            with self.subTest(event=event):
                self.assertFalse(self.plan(event=event)["publish"])
        self.assertFalse(self.plan(branch="feat/example")["publish"])
        self.assertFalse(self.plan(scope=self.scope(scopes=[], images=[]))["publish"])
        # A server check caused by web/API dependencies is not a Docker release.
        server = self.plan(scope=self.scope(scopes=["server"], images=[]))
        self.assertFalse(server["publish"])
        web = self.plan(scope=self.scope(scopes=["web", "images"], images=["web"]))
        self.assertTrue(web["publish"] and web["web"])
        self.assertFalse(web["backend"] or web["native"])

    def test_unreachable_and_other_channel_tags_cannot_inflate_stable(self):
        self.git("checkout", "--quiet", "-b", "unrelated")
        elsewhere = self.commit("feat!: isolated branch\n\nBREAKING CHANGE: Synthetic migration.")
        self.git("tag", artifact_identity("main", (8, 0, 0), elsewhere, 2)["tag"])
        self.git("checkout", "--quiet", "dev")
        self.commit("fix: local correction")
        result = self.plan()
        self.assertTrue(result["version"].startswith("0.2.2+dev.g"))

    def test_initial_untagged_release_uses_bootstrap_without_replaying_history(self):
        self.git("tag", "--delete", "v0.2.1")
        self.git("tag", "--delete", self.initial_artifact)
        (self.root / "release-please-config.json").write_text(json.dumps({"bootstrap-sha": self.baseline}))
        self.commit("fix: first tracked change")
        result = self.plan()
        self.assertTrue(result["version"].startswith("0.2.2+dev.g"))
        self.assertEqual(result["commits"], 1)

    def test_pending_replaced_push_is_included_in_authoritative_release_scope(self):
        native = self.root / "apps/fleetworks-desktop/src/main.rs"
        native.parent.mkdir(parents=True)
        native.write_text("synthetic native source")
        self.git("add", str(native.relative_to(self.root)))
        self.git("commit", "--quiet", "-m", "fix(native): pending native change")
        pending = self.head()
        backend = self.root / "services/api-host/src/main.rs"
        backend.parent.mkdir(parents=True)
        backend.write_text("synthetic service source")
        self.git("add", str(backend.relative_to(self.root)))
        self.git("commit", "--quiet", "-m", "fix(api): later service change")
        event_scope = self.scope(base=pending, scopes=["images", "server"], images=["runtime"])
        result = self.plan(base=pending, scope=event_scope)
        self.assertTrue(result["native"], "A replaced pending native build remains required")
        self.assertTrue(result["backend"])
        self.assertEqual(result["release_base"], self.baseline)
        self.assertEqual(result["base"], pending, "Event provenance remains exact")
        self.assertIn("desktop", result["scope"]["scopes"])

    def test_first_artifact_covers_production_targets_and_test_only_web_stays_checked(self):
        self.git("tag", "--delete", self.initial_artifact)
        self.commit("ci: enable channel artifacts")
        first = self.plan(scope=self.scope(scopes=[], images=[]))
        self.assertTrue(first["native"] and first["backend"] and first["web"])
        self.assertEqual(set(first["image_targets"]), {"runtime", "surrealdb", "search-index", "web"})
        self.publish_tag(first)
        before = self.head()
        self.commit("test: improve browser contract")
        scope = self.scope(base=before, scopes=["web"], images=[])
        result = self.plan(base=before, scope=scope)
        self.assertFalse(result["web"], "Web verification alone does not publish an image")
        self.assertFalse(result["publish"])
        output = self.root / "check-outputs"
        from fleetworks_release import write_outputs
        write_outputs(result, output)
        lines = output.read_text().splitlines()
        self.assertIn("web=true", lines)
        self.assertIn("release_web=false", lines)

    def test_missing_or_mismatched_history_scope_and_identity_fail_closed(self):
        self.commit("feat: scope validation")
        for field, value in [
            ("head", "f" * 40), ("event", "pull_request"), ("base", "e" * 40),
            ("image_targets", ["retired-client"]), ("scopes", ["retired-client"]),
            ("image_targets", ["runtime", "runtime"]),
            ("full", "false"), ("image_arches", ["amd64"]), ("format", True),
        ]:
            scope = self.scope()
            scope[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.plan(scope=scope)
        with self.assertRaisesRegex(ValueError, "exact Git SHA"):
            self.plan(base="anything\npublish=true")
        self.git("tag", artifact_identity("dev", (0, 3, 0), "f" * 40, 2)["tag"])
        with self.assertRaisesRegex(ValueError, "actual source commit"):
            self.plan()

    def test_nonancestor_push_is_not_published_and_workspace_is_a_floor(self):
        self.git("checkout", "--quiet", "-b", "other")
        unrelated = self.commit("fix: other branch")
        self.git("checkout", "--quiet", "dev")
        self.commit("fix: selected branch")
        with self.assertRaisesRegex(ValueError, "ancestor"):
            self.plan(base=unrelated)
        (self.root / "version.txt").write_text("1.0.0\n")
        self.commit("fix!: explicit v1 break\n\nBREAKING CHANGE: Migrate the caller.")
        self.assertTrue(self.plan()["version"].startswith("2.0.0+dev.g"))

    def test_cli_outputs_are_machine_readable_and_no_source_refs_are_changed(self):
        self.commit("fix: publishable change")
        scope = self.root / "scope.json"
        scope.write_text(json.dumps(self.scope()))
        plan_path, output = self.root / "release.json", self.root / "github-output"
        before = self.git("show-ref")
        with mock.patch("fleetworks_release.ROOT", self.root), mock.patch("sys.stdout"):
            self.assertEqual(main(["plan", "--event", "push", "--ref", "refs/heads/dev",
                                   "--head", self.head(), "--base", self.baseline,
                                   "--scope-plan", str(scope), "--json-output", str(plan_path),
                                   "--github-output", str(output)]), 0)
        result = json.loads(plan_path.read_text())
        lines = output.read_text().splitlines()
        self.assertIn("publish=true", lines)
        self.assertIn("image_targets=runtime,search-index", lines)
        self.assertIn("version=" + result["version"], lines)
        self.assertEqual(self.git("show-ref"), before)


if __name__ == "__main__":
    unittest.main()
