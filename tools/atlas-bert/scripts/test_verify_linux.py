#!/usr/bin/env python3
"""Finite synthetic parser/artifact fixtures; no engine, compiler or model run."""
import copy
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
