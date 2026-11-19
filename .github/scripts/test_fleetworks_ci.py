"""Offline contracts for first-pass and second-pass verification routing."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import fleetworks_ci as ci

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import fleetworks_verify as planner


class VerificationPassTests(unittest.TestCase):
    def scopes(self, *paths):
        with patch.object(planner, "git_paths", return_value=[*paths, ""]):
            return {"scope": ci.compare(planner, "a" * 40, "b" * 40)}

    def test_native_implementation_and_artwork_use_only_native_linux_first(self):
        plan = self.scopes(
            "apps/fleetworks-desktop/src/startup.rs",
            "assets/brand/orbit/fleetworks-core-texture.rgba",
            "docs/design/matte-canvas/mockups/02-setup-gateway.png",
        )
        first = ci.verification("first", plan)
        self.assertTrue(first["desktop_linux"])
        self.assertFalse(first["desktop_macos"])
        self.assertFalse(first["server"])
        self.assertFalse(first["images"])
        self.assertFalse(first["web"])

    def test_backend_change_selects_linux_native_parity_and_scoped_checks(self):
        plan = self.scopes("crates/argos-osint-core/src/storage/setup_jobs.rs")
        first = ci.verification("first", plan)
        self.assertTrue(first["desktop_linux"])
        for selected in ("server", "storage", "queue", "images"):
            self.assertTrue(first[selected], selected)
        self.assertFalse(first["web"])
        self.assertEqual(first["backend_arch"], "amd64")
        self.assertEqual(first["image_matrix"][0]["targets"], "runtime")
        self.assertEqual(len(first["image_matrix"]), 1)
        self.assertTrue(first["review_images"])
        self.assertEqual(first["review_image_matrix"], [{**first["image_matrix"][0], "dev": False}])
        self.assertFalse(first["full"])

    def test_workflow_only_pr_exercises_native_candidate_instead_of_all_services(self):
        first = ci.verification("first", self.scopes(".github/workflows/fleetworks.yml"))
        self.assertTrue(first["desktop_linux"])
        for skipped in ("desktop_macos", "server", "images", "storage", "compose", "web"):
            self.assertFalse(first[skipped], skipped)

    def test_web_only_first_pass_selects_no_unrelated_builds(self):
        plan = self.scopes("package.json", "packages/intel/src/views/bulletin/SearchResults.tsx")
        first = ci.verification("first", plan)
        for skipped in ("desktop_linux", "desktop_macos", "server", "compose", "images", "web"):
            self.assertFalse(first[skipped], skipped)
        self.assertEqual(first["image_matrix"], [])
        self.assertFalse(first["review_images"])
        second = ci.verification("second", plan)
        self.assertTrue(second["web"])
        self.assertEqual({entry["targets"] for entry in second["image_matrix"]}, {"web"})
        self.assertEqual({entry["targets"] for entry in second["review_image_matrix"]}, {"web"})

    def test_second_pass_defers_only_the_remaining_architectures_and_web(self):
        plan = self.scopes("Cargo.lock", "package.json")
        second = ci.verification("second", plan)
        self.assertTrue(second["desktop_macos"])
        self.assertFalse(second["desktop_linux"])
        self.assertTrue(second["web"])
        self.assertEqual(second["backend_arch"], "arm64")
        images = {entry["arch"]: entry for entry in second["image_matrix"]}
        self.assertEqual(images["amd64"]["targets"], "web")
        self.assertFalse(images["amd64"]["compose"])
        self.assertEqual(images["arm64"]["targets"], "runtime,search-index,web")

    def test_standard_verification_keeps_trusted_release_matrix(self):
        plan = self.scopes("Cargo.lock", "package.json")
        plan["publish"] = True
        plan["scope"]["full"] = True
        standard = ci.verification("standard", plan)
        self.assertTrue(standard["desktop_linux"])
        self.assertTrue(standard["desktop_macos"])
        self.assertTrue(standard["web"])
        self.assertTrue(standard["full"])
        self.assertEqual(standard["backend_arch"], "both")
        self.assertEqual(standard["image_targets"], "runtime,search-index,web")
        self.assertFalse(standard["review_images"])
        self.assertEqual(standard["review_image_matrix"], [])
        self.assertTrue(plan["publish"])
        self.assertEqual(plan["scope"]["image_arches"], ["amd64", "arm64"])

    def test_compose_only_checks_use_the_review_builder_on_each_pass(self):
        plan = self.scopes("deploy/compose.yml")
        first = ci.verification("first", plan)
        self.assertFalse(first["images"])
        self.assertEqual(first["image_targets"], "")
        self.assertTrue(first["review_images"])
        self.assertEqual(first["review_image_matrix"], [{
            "arch": "amd64", "runner": "ubuntu-24.04",
            "targets": "runtime,search-index,surrealdb", "compose": True, "dev": False,
        }])
        second = ci.verification("second", plan)
        self.assertEqual(second["review_image_matrix"], [{
            "arch": "arm64", "runner": "ubuntu-24.04-arm",
            "targets": "runtime,search-index,surrealdb,web", "compose": True, "dev": False,
        }])

    def test_compose_only_release_still_checks_both_loaded_images(self):
        plan = self.scopes("deploy/compose.yml")
        plan["publish"] = True
        standard = ci.verification("standard", plan)
        self.assertFalse(standard["images"])
        self.assertEqual(standard["image_targets"], "")
        self.assertTrue(standard["review_images"])
        review = standard["review_image_matrix"]
        self.assertEqual([entry["arch"] for entry in review], ["amd64", "arm64"])
        self.assertEqual({entry["targets"] for entry in review}, {"runtime,search-index,surrealdb,web"})
        self.assertTrue(all(entry["compose"] and not entry["dev"] for entry in review))

    def test_full_review_smokes_dev_only_for_amd64_when_selected(self):
        plan = self.scopes("deploy/compose.yml", "Cargo.lock")
        plan["scope"]["full"] = True
        standard = ci.verification("standard", plan)
        self.assertEqual(standard["image_targets"], "runtime,search-index")
        self.assertTrue(standard["review_images"])
        amd64, arm64 = standard["review_image_matrix"]
        self.assertEqual((amd64["arch"], amd64["targets"], amd64["dev"]), ("amd64", "runtime,search-index,web", True))
        self.assertEqual((arm64["arch"], arm64["targets"], arm64["dev"]), ("arm64", "runtime,search-index", False))

    def test_reviewer_pass_uses_real_three_dot_diff_and_ignores_empty_path(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", *args], text=True, cwd=directory).strip()

            git("init", "--quiet")
            git("config", "user.name", "Fleetworks test")
            git("config", "user.email", "ci@example.invalid")
            git("commit", "--quiet", "--allow-empty", "-m", "base")
            base = git("rev-parse", "HEAD")
            path = "apps/fleetworks-desktop/src/startup.rs"
            record = Path(directory, path)
            record.parent.mkdir(parents=True)
            record.write_text("native fixture", encoding="utf-8")
            git("add", path)
            git("commit", "--quiet", "-m", "change native only")
            head = git("rev-parse", "HEAD")

            def paths(*args):
                return planner.git_paths(*args, cwd=directory)

            with patch.object(ci, "path_selection", wraps=ci.path_selection) as select:
                trusted = SimpleNamespace(
                    git_paths=paths, path_scopes=planner.path_scopes, images_for=planner.images_for,
                )
                result = ci.compare(trusted, base, head)
            self.assertEqual(result["scopes"], ["desktop", "local_backend"])
            self.assertEqual(result["image_targets"], [])
            self.assertEqual(select.call_args_list[-1].args[1], "")


class SecondPassPromotionTests(unittest.TestCase):
    def setUp(self):
        self.repository = "example/fleetworks"
        self.branch = "refs/heads/feature"
        self.head = "a" * 40
        self.base = "b" * 40
        self.pr = {
            "state": "open",
            "head": {"repo": {"full_name": self.repository}, "sha": self.head, "ref": "feature"},
            "base": {"ref": "nightly", "sha": self.base},
        }

    def promotion(self, runs, *, revision=None):
        with patch.object(ci, "github_json", side_effect=[self.pr, {"workflow_runs": runs}]):
            return ci.promote(
                self.repository, self.branch, "7", self.head, revision or self.head, "", "auto",
            )

    def first_run(self, *, number=7, base=None, conclusion="success"):
        return {
            "id": 42,
            "conclusion": conclusion,
            "pull_requests": [{"number": number, "base": {"sha": base or self.base}}],
        }

    def test_exact_open_pr_and_successful_first_pass_allow_promotion(self):
        self.assertEqual(self.promotion([self.first_run()]), (self.base, "nightly", 42))

    def test_stale_branch_head_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "exact branch head"):
            self.promotion([self.first_run()], revision="c" * 40)

    def test_other_pr_or_changed_base_do_not_allow_promotion(self):
        with self.assertRaisesRegex(ValueError, "first pass must succeed"):
            self.promotion([self.first_run(number=8), self.first_run(base="c" * 40)])

    def test_latest_first_pass_failure_cannot_fall_back_to_older_green_run(self):
        with self.assertRaisesRegex(ValueError, "first pass must succeed"):
            self.promotion([self.first_run(conclusion="failure"), self.first_run()])

    def test_closed_pull_request_cannot_be_promoted(self):
        self.pr["state"] = "closed"
        with self.assertRaisesRegex(ValueError, "open same-repository PR branch"):
            self.promotion([self.first_run()])


if __name__ == "__main__":
    unittest.main()
