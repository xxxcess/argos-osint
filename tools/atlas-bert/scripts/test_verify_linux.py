#!/usr/bin/env python3
"""Finite synthetic parser/artifact fixtures; no engine, compiler or model run."""
import copy
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import finetune
import verify_linux as verify

AMD = "x86_64-unknown-linux-gnu"
ARM = "aarch64-unknown-linux-gnu"
REVISION = "a" * 40


class LinuxWorkerVerification(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="atlas-bert-verification-fixture-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def fake_elf(self, path, target=AMD):
        path.parent.mkdir(parents=True, exist_ok=True)
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", header, 18, verify.TARGETS[target]["elf"])
        path.write_bytes(header)
        path.chmod(0o755)
        return path

    def inference(self):
        election, research = [0.9, .1, .1, .1, .1, .2], [.2, .1, .1, .1, .1, .9]
        return json.loads(json.dumps([{"protocol": "argos-atlas-bert-v1"}, {"scores": [election, research]},
                {"scores": [research]}, {"scores": [election]}, {"scores": [election, research]}]))

    def output(self, value):
        return "\n".join(json.dumps(line) for line in value)

    def test_official_cpu_wheel_pins_cover_both_native_architectures(self):
        self.assertEqual(set(verify.TARGETS), {AMD, ARM})
        for target in verify.TARGETS:
            url = verify.wheel_url(target)
            self.assertTrue(url.startswith("https://download.pytorch.org/whl/cpu/torch-2.1.0"))
            self.assertIn("cp311-cp311", url)
            self.assertRegex(url, r"#sha256=[a-f0-9]{64}$")
        self.assertIn("%2Bcpu", verify.wheel_url(AMD))
        self.assertIn("aarch64", verify.wheel_url(ARM))

    def test_host_guard_rejects_mac_cross_and_newer_glibc_without_running_tools(self):
        release = 'ID=debian\nVERSION_ID="12"\n'
        for target, machine in ((AMD, "x86_64"), (ARM, "aarch64")):
            verify.host_guard(target, system="Linux", machine=machine, release=release, glibc="glibc 2.36")
        for changed in ({"system": "Darwin"}, {"machine": "aarch64"}, {"glibc": "glibc 2.39"},
                        {"release": 'ID=ubuntu\nVERSION_ID="24.04"\n'}):
            values = {"system": "Linux", "machine": "x86_64", "release": release, "glibc": "glibc 2.36", **changed}
            with self.assertRaises(verify.VerifyError):
                verify.host_guard(AMD, **values)

    def test_cargo_executable_honors_reported_cache_path_and_rejects_missing_or_ambiguous(self):
        selected = self.root / "override/debug" / verify.WORKER
        artifact = {"reason": "compiler-artifact", "target": {"name": verify.WORKER, "kind": ["bin"]}, "executable": str(selected)}
        decoy = {**artifact, "target": {"name": "convert-tensor", "kind": ["bin"]}, "executable": str(self.root / "converter")}
        self.assertEqual(verify.executable(self.output([decoy, artifact])), selected)
        for events in ([], [decoy], [artifact, {**artifact, "executable": str(self.root / "other-worker")}],
                       [{**artifact, "target": {"name": verify.WORKER, "kind": ["example"]}}]):
            with self.assertRaises(verify.VerifyError):
                verify.executable(self.output(events))

    def test_zero_and_ignored_worker_tests_are_not_success(self):
        self.assertEqual(verify.unit_result("test result: ok. 3 passed; 0 failed; 0 ignored;"), 3)
        for output in ("", "test result: ok. 0 passed; 0 failed; 0 ignored;",
                       "test result: ok. 3 passed; 0 failed; 1 ignored;", "test result: FAILED. 0 passed; 1 failed;"):
            with self.assertRaises(verify.VerifyError):
                verify.unit_result(output)

    def test_real_inference_receipt_requires_semantics_and_batch_repeat_invariance(self):
        value = self.inference()
        self.assertEqual(verify.inference_result(self.output(value))["requests"], 4)
        variants = []
        for mutate in (lambda item: item[0].update(protocol="wrong"),
                       lambda item: item.pop(),
                       lambda item: item[1]["scores"][0].pop(),
                       lambda item: item[1]["scores"][0].__setitem__(0, float("nan")),
                       lambda item: item[2]["scores"][0].__setitem__(0, .8),
                       lambda item: item[4]["scores"][0].__setitem__(0, .7)):
            item = copy.deepcopy(value)
            mutate(item)
            variants.append(item)
        # Maintain batch/repeat agreement while invalidating domain semantics.
        inverted = copy.deepcopy(value)
        for response in inverted[1:]:
            for row in response["scores"]:
                row[0], row[5] = row[5], row[0]
        variants.append(inverted)
        for item in variants:
            with self.assertRaises(verify.VerifyError):
                verify.inference_result(self.output(item))

    def test_elf_and_actual_symbol_versions_fail_wrong_arch_or_newer_runtime(self):
        path = self.fake_elf(self.root / "worker")
        verify.elf(path, AMD)
        with self.assertRaises(verify.VerifyError):
            verify.elf(path, ARM)
        self.assertEqual(verify.symbol_versions("GLIBC_2.36 GLIBC_2.17 GLIBCXX_3.4.30 CXXABI_1.3.13")["GLIBC"], "2.36")
        for requirement in ("GLIBC_2.38", "GLIBCXX_3.4.32", "CXXABI_1.3.15"):
            with self.assertRaises(verify.VerifyError):
                verify.symbol_versions(requirement)

    def test_bundled_linkage_rejects_missing_or_build_environment_torch_library(self):
        bundle = self.root / "bundle"
        torch = bundle / "python/torch/lib/libtorch_cpu.so"
        c10 = bundle / "python/torch/lib/libc10.so"
        for path in (torch, c10):
            self.fake_elf(path)
        output = f"libtorch_cpu.so => {torch} (0x123)\nlibc10.so => {c10} (0x456)\n"
        self.assertEqual(verify.linked_libraries(output, bundle), ["libc10.so", "libtorch_cpu.so"])
        for invalid in (output.replace(str(torch), "/build-wheel/torch/lib/libtorch_cpu.so"),
                        "libtorch_cpu.so => not found", "libc.so.6 => /lib/libc.so.6 (0x1)"):
            with self.assertRaises(verify.VerifyError):
                verify.linked_libraries(invalid, bundle)

    def test_system_openmp_dependency_is_packaged_while_baseline_libc_is_not(self):
        bundle = self.root / "bundle"
        (bundle / "python/torch/lib").mkdir(parents=True)
        gomp = self.fake_elf(self.root / "system/libgomp.so.1")
        copied, origins = verify.copy_external_libraries(f"libgomp.so.1 => {gomp} (0x1)\nlibc.so.6 => /lib/libc.so.6 (0x2)", bundle, AMD)
        self.assertEqual([path.name for path in copied], ["libgomp.so.1"])
        self.assertEqual(origins, {"libgomp.so.1": str(gomp)})
        self.assertFalse((bundle / "python/torch/lib/libc.so.6").exists())

    def test_archive_unpack_rejects_traversal_links_duplicates_and_special_objects(self):
        for index, kind in enumerate(("traversal", "symlink", "hardlink", "duplicate", "device")):
            archive = self.root / f"unsafe-{index}.tar.gz"
            with tarfile.open(archive, "w:gz") as opened:
                member = tarfile.TarInfo("../escape" if kind == "traversal" else "payload")
                if kind in ("symlink", "hardlink"):
                    member.type = tarfile.SYMTYPE if kind == "symlink" else tarfile.LNKTYPE
                    member.linkname = "/outside"
                elif kind == "device":
                    member.type = tarfile.CHRTYPE
                opened.addfile(member)
                if kind == "duplicate":
                    opened.addfile(member)
            with self.assertRaises(verify.VerifyError):
                verify.safe_unpack(archive, self.root / f"out-{index}")
        self.assertFalse((self.root / "escape").exists())

    def test_bundle_manifest_checks_exact_source_payload_hashes_and_executable(self):
        bundle = self.root / "bundle"
        binary = self.fake_elf(bundle / "bin" / verify.WORKER)
        manifest = {"format_version": 1, "status": "runtime_verified", "source_sha": REVISION, "target": AMD,
                    "files": {"bin/" + verify.WORKER: {"bytes": binary.stat().st_size, "sha256": verify.sha(binary)}}}
        verify.save(bundle / "manifest.json", manifest)
        checks = bundle / "SHA256SUMS"
        checks.write_text("".join(verify.sha(path) + "  " + str(path.relative_to(bundle)) + "\n" for path in verify.payload_files(bundle)))
        verify.check_bundle(bundle, AMD, REVISION)
        with self.assertRaises(verify.VerifyError):
            verify.check_bundle(bundle, AMD, "b" * 40)
        binary.write_bytes(binary.read_bytes() + b"changed")
        with self.assertRaises(verify.VerifyError):
            verify.check_bundle(bundle, AMD, REVISION)


class FineTuningContract(unittest.TestCase):
    def test_only_exact_manual_fleetworks_backend_run_selects_amd64_fitting(self):
        with tempfile.TemporaryDirectory() as temporary:
            event = Path(temporary) / "event.json"
            inputs = {"stage": "standard", "mode": "pr", "components": "backend",
                      "base": "b" * 40, "head": REVISION}
            verify.save(event, {"inputs": inputs})
            env = {
                "GITHUB_ACTIONS": "true",
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "GITHUB_EVENT_PATH": str(event),
                "GITHUB_REF": "refs/heads/feature/bert",
                "GITHUB_REPOSITORY": "owner/repo",
                "GITHUB_SHA": REVISION,
                "GITHUB_WORKFLOW_REF": "owner/repo/.github/workflows/fleetworks.yml@refs/heads/feature/bert",
            }
            self.assertTrue(verify.managed_finetune(AMD, REVISION, env))
            self.assertFalse(verify.managed_finetune(ARM, REVISION, env))
            self.assertFalse(verify.managed_finetune(AMD, REVISION, {**env, "GITHUB_EVENT_NAME": "pull_request"}))
            self.assertFalse(verify.managed_finetune(AMD, REVISION, {**env, "GITHUB_EVENT_NAME": "push"}))
            self.assertFalse(verify.managed_finetune(AMD, REVISION, {**env, "GITHUB_WORKFLOW_REF": "other"}))
            with self.assertRaisesRegex(verify.VerifyError, "exact dispatched head"):
                verify.managed_finetune(AMD, REVISION, {**env, "GITHUB_SHA": "d" * 40})
            for override in ({"stage": "second"}, {"mode": "full"}, {"components": "auto"}):
                verify.save(event, {"inputs": {**inputs, **override}})
                self.assertFalse(verify.managed_finetune(AMD, REVISION, env))

    def test_candidate_payloads_are_bounded_and_explicitly_opted_in(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle = root / "bundle"
            candidate = bundle / "proof/candidate/rust_model.ot"
            candidate.parent.mkdir(parents=True)
            candidate.write_bytes(b"model")
            with self.assertRaisesRegex(verify.VerifyError, "Unexpected candidate"):
                verify.payload_files(bundle)
            self.assertEqual(verify.payload_files(bundle, proof=True), [candidate])
            archive = root / "bundle.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(candidate, arcname=candidate.relative_to(bundle))
            with self.assertRaisesRegex(verify.VerifyError, "Unexpected candidate"):
                verify.safe_unpack(archive, root / "baseline")
            with patch.object(verify, "MAX_CANDIDATE_PROOF", 2):
                with self.assertRaisesRegex(verify.VerifyError, "exceeds"):
                    verify.safe_unpack(archive, root / "oversized", proof=True)
            verify.safe_unpack(archive, root / "proof", proof=True)
            self.assertEqual((root / "proof/proof/candidate/rust_model.ot").read_bytes(), b"model")

    def test_pinned_corpus_fitting_partitions_and_worker_hypotheses(self):
        partitions = ("train", "development", "calibration")
        corpus = finetune.read_articles(finetune.read_corpus(), partitions)
        self.assertEqual({key: len(value) for key, value in corpus.items()}, {
            "train": 440, "development": 165, "calibration": 230,
        })
        self.assertTrue(all(article.text.count("\n") == 1 for rows in corpus.values() for article in rows))
        contract = finetune.runtime_contract()
        self.assertEqual(contract["domains"], list(finetune.DOMAINS))
        self.assertEqual(len(set(contract["hypotheses"])), 6)
        self.assertEqual(contract["max_tokens"], 192)

    def test_rust_admission_gates_preserve_cap_and_ambiguity(self):
        for scores, expected in (
            ([.99, .98, .97, .1, .1, .1], ("governance", "security")),
            ([.97, .98, .99, .1, .1, .1], ("economic", "security")),
            ([.99, .98, .98, .1, .1, .1], ("governance",)),
            ([.99, .99, .98, .1, .1, .1], ()),
            ([.99, .99, .1, .1, .1, .1], ("governance", "security")),
            ([.79, .1, .1, .1, .1, .1], ()),
        ):
            with self.subTest(scores=scores):
                self.assertEqual(finetune.select_domains(scores, .8), expected)

    def test_reports_full_membership_and_does_not_hide_false_admissions(self):
        articles = [
            finetune.Article("complete triad", frozenset({"governance", "security", "economic"})),
            finetune.Article("complete pair", frozenset({"security", "economic"})),
            finetune.Article("out of scope", frozenset()),
        ]
        scores = [
            [.99, .98, .97, .1, .1, .1],
            [.1, .98, .99, .1, .1, .1],
            [.99, .1, .1, .1, .1, .1],
        ]
        unsafe = finetune.metrics(articles, scores, .8)
        self.assertAlmostEqual(unsafe["micro_f1"], .8)
        self.assertEqual(unsafe["out_of_scope_rows"], 1)
        self.assertEqual(unsafe["false_out_of_scope_admissions"], 1)
        self.assertEqual(unsafe["per_domain"]["economic"]["fn"], 1)
        self.assertEqual(unsafe["top_two"]["valid_triad_pairs"], 1)
        self.assertEqual(unsafe["top_two"]["exact_pair_rows"], 1)
        safe = finetune.metrics(articles, [*scores[:2], [.1] * 6], .8)
        self.assertTrue(finetune.regression_passes(unsafe, safe))
        self.assertFalse(finetune.regression_passes(safe, unsafe))

    def test_calibration_excludes_higher_f1_threshold_with_extra_false_admission(self):
        articles = [finetune.Article(f"positive {index}", frozenset({"governance"})) for index in range(3)]
        articles.append(finetune.Article("out of scope", frozenset()))
        baseline = [[score, .1, .1, .1, .1, .1] for score in (.9, .9, .1, .1)]
        candidate = [[score, .1, .1, .1, .1, .1] for score in (.975, .954, .704, .724)]
        report = finetune.select_threshold(articles, baseline, candidate)
        self.assertEqual(report["status"], "qualified")
        self.assertEqual(report["baseline_threshold"], .8)
        self.assertEqual(report["tested_thresholds"], 50)
        self.assertEqual(report["best_overall"]["threshold"], .7)
        self.assertGreater(report["best_overall"]["metrics"]["micro_f1"], report["baseline"]["micro_f1"])
        self.assertEqual(report["best_overall"]["metrics"]["false_out_of_scope_admissions"], 1)
        self.assertEqual(report["selected"]["threshold"], .95)
        self.assertTrue(finetune.regression_passes(report["baseline"], report["selected"]["metrics"]))

    def test_rejected_calibration_writes_evidence_and_never_freezes_candidate(self):
        articles = [finetune.Article("positive", frozenset({"governance"})),
                    finetune.Article("out of scope", frozenset())]
        baseline = [[.9, .1, .1, .1, .1, .1], [.1, .1, .1, .1, .1, .1]]
        candidate = [[.7, .1, .1, .1, .1, .1], [.9, .1, .1, .1, .1, .1]]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination, checkpoint, worker = (root / name for name in ("candidate", "model", "worker"))
            checkpoint.mkdir()
            worker.write_bytes(b"worker")
            for name in finetune.MODEL_FILES:
                (checkpoint / name).write_text(name)

            def train(*args):
                args[-1].write_bytes(b"pytorch weights")

            def convert(command, **options):
                command[3].with_name("rust_model.ot").write_bytes(b"converted weights")

            phase = {"train": [], "development": [], "calibration": articles}
            with (
                patch.object(finetune, "validate_prepared", return_value=(checkpoint, root)),
                patch.object(finetune, "read_corpus", return_value=b"test data"),
                patch.object(finetune, "read_articles", return_value=phase) as read,
                patch.object(finetune, "train_checkpoint", side_effect=train),
                patch.object(finetune.subprocess, "run", side_effect=convert),
                patch.object(finetune, "score_with_worker", side_effect=(baseline, candidate)) as score,
            ):
                self.assertIsNone(finetune.fit(root, destination, worker, REVISION))
            read.assert_called_once_with(b"test data", ("train", "development", "calibration"))
            self.assertEqual(score.call_count, 2)
            self.assertEqual(score.call_args_list[0].args[:3], (worker, checkpoint, articles))
            self.assertEqual(score.call_args_list[1].args[2], articles)
            report = json.loads(finetune.calibration_receipt_path(destination).read_text())
            self.assertEqual(report["status"], "rejected")
            self.assertEqual(report["qualifying_thresholds"], 0)
            self.assertIsNone(report["selected"])
            self.assertFalse(destination.exists())

    def test_malformed_worker_responses_are_rejected(self):
        first = '{"protocol":"argos-atlas-bert-v1"}\n'
        for response in ('{"scores":[[true,0,0,0,0,0]]}', '{"scores":[[NaN,0,0,0,0,0]]}',
                         '{"scores":[[0,0,0,0,0]]}', '{"scores":[]}'):
            with self.subTest(response=response), self.assertRaises(verify.VerifyError):
                finetune.parse_worker_scores(first + response, [1], "argos-atlas-bert-v1")

    def test_holdout_refuses_mutated_weights_before_reading_test_articles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            candidate = root / "candidate"
            baseline = root / "baseline"
            worker = root / "worker"
            worker.write_bytes(b"worker")
            for directory in (candidate, baseline):
                directory.mkdir()
                for name in finetune.MODEL_FILES:
                    (directory / name).write_text(name)
            path = candidate / finetune.FROZEN_NAME
            frozen = {
                "schema_version": "atlas-bert-frozen-v1",
                "dataset_sha256": finetune.DATASET_SHA256,
                "split_counts": finetune.SPLIT_COUNTS,
                "base_checkpoint_revision": finetune.MODEL_COMMIT,
                "assets": finetune.model_assets(candidate),
                "threshold": .8,
                "provenance": {
                    "baseline_assets": finetune.model_assets(baseline),
                    "runtime_contract": finetune.runtime_contract(),
                    "calibration_worker_sha256": verify.sha(worker),
                },
            }
            finetune.write_new(path, frozen)
            finetune.load_frozen(candidate, baseline, worker)
            (candidate / "rust_model.ot").write_bytes(b"mutated")
            with patch.object(finetune, "read_corpus") as corpus:
                with self.assertRaisesRegex(verify.VerifyError, "Frozen model"):
                    finetune.evaluate(candidate, baseline, worker)
                corpus.assert_not_called()

    def test_frozen_calibration_qualification_binds_policy_identity_and_selected_metrics(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            candidate, baseline, worker = (root / name for name in ("candidate", "baseline", "worker"))
            worker.write_bytes(b"worker")
            for directory in (candidate, baseline):
                directory.mkdir()
                for name in finetune.MODEL_FILES:
                    (directory / name).write_text(name)
            candidate_assets, baseline_assets = map(finetune.model_assets, (candidate, baseline))
            articles = [finetune.Article("fixture", frozenset({"governance"}))] * 230
            scores = [[.945, .1, .1, .1, .1, .1]] * 230
            qualification = finetune.select_threshold(articles, scores, scores)
            qualification.update({"dataset_sha256": finetune.DATASET_SHA256, "source_revision": REVISION,
                                  "worker_sha256": verify.sha(worker), "baseline_assets": baseline_assets,
                                  "candidate_assets": candidate_assets})
            frozen = {
                "schema_version": "atlas-bert-frozen-v1", "dataset_sha256": finetune.DATASET_SHA256,
                "split_counts": finetune.SPLIT_COUNTS, "base_checkpoint_revision": finetune.MODEL_COMMIT,
                "assets": candidate_assets, "threshold": qualification["selected"]["threshold"],
                "provenance": {
                    "baseline_assets": baseline_assets, "runtime_contract": finetune.runtime_contract(),
                    "calibration_worker_sha256": verify.sha(worker), "source_revision": REVISION,
                    "calibration": copy.deepcopy(qualification["selected"]["metrics"]),
                    "calibration_qualification": qualification,
                },
            }
            path = candidate / finetune.FROZEN_NAME
            verify.save(path, frozen)
            finetune.load_frozen(candidate, baseline, worker)
            for key, value in (("status", "rejected"), ("worker_sha256", "mismatch"), ("qualifying_thresholds", 0)):
                changed = copy.deepcopy(frozen)
                changed["provenance"]["calibration_qualification"][key] = value
                verify.save(path, changed)
                with self.subTest(key=key), self.assertRaisesRegex(verify.VerifyError, "calibration qualification"):
                    finetune.load_frozen(candidate, baseline, worker)


if __name__ == "__main__":
    unittest.main()
