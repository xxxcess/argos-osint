"""Offline contracts for the immutable native CLI candidate archive."""

import io
import json
import os
import plistlib
import struct
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from fleetworks_candidate import BINARIES, NATIVE_APP, NATIVE_BINARY, NATIVE_MANIFEST
from fleetworks_candidate import apple_versions, cargo_executable, cargo_native_command
from fleetworks_candidate import checksum_line, extract, extract_native, native_output, package, package_native, sha256, tar_member


class CandidateArchiveTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.version = "0.2.1"
        self.target = "x86_64-unknown-linux-gnu"
        self.revision = "1" * 40
        self.binaries = self.root / "binaries"
        self.binaries.mkdir()
        for name in BINARIES:
            path = self.binaries / name
            path.write_bytes(f"fixture payload {name}\n".encode())
            path.chmod(0o755)
        self.license = self.root / "LICENSE"
        self.license.write_bytes(b"fixture license\n")

    def pack(self, name="archive"):
        return package(self.root / name, self.binaries, self.license, self.version, self.target, self.revision)

    def extract(self, archive, digest, revision=None):
        destination = self.root / "install"
        extract(archive, destination, digest, self.version, self.target, revision or self.revision)
        return destination

    def test_reproducible_archive_preserves_every_binary_and_its_mode(self):
        archive, digest = self.pack()
        duplicate, duplicate_digest = self.pack("duplicate")
        self.assertEqual(archive.read_bytes(), duplicate.read_bytes())
        self.assertEqual(digest, duplicate_digest)
        self.assertEqual(archive.with_suffix(".gz.sha256").read_text(), checksum_line(archive, digest))
        install = self.extract(archive, digest)
        self.assertEqual((install / "LICENSE").read_bytes(), self.license.read_bytes())
        for name in BINARIES:
            path = install / "bin" / name
            self.assertEqual(path.read_bytes(), (self.binaries / name).read_bytes())
            self.assertTrue(os.access(path, os.X_OK))

    def test_rejects_digest_sidecar_and_manifest_mismatches(self):
        archive, digest = self.pack()
        with self.assertRaisesRegex(ValueError, "packaging job's SHA-256"):
            self.extract(archive, "0" * 64)
        with self.assertRaisesRegex(ValueError, "manifest does not match"):
            self.extract(archive, digest, "2" * 40)
        archive.with_suffix(".gz.sha256").write_text("wrong\n")
        with self.assertRaisesRegex(ValueError, "sidecar"):
            self.extract(archive, digest)
        self.assertFalse((self.root / "install").exists())

    def test_rejects_unexpected_paths_before_extraction(self):
        archive, _ = self.pack()
        with tarfile.open(archive, "w:gz") as tar:
            tar.addfile(tar_member("../escape", 1, 0o644), io.BytesIO(b"x"))
        digest = sha256(archive)
        archive.with_suffix(".gz.sha256").write_text(checksum_line(archive, digest))
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.extract(archive, digest)
        self.assertFalse((self.root / "escape").exists())
        self.assertFalse((self.root / "install").exists())


class NativeAppTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        # macOS temporary paths may use /var -> /private/var. Match the
        # packager's canonical destination so fault injection hits replacement.
        self.root = Path(scratch.name).resolve()
        self.binary = self.root / "cargo output with spaces" / NATIVE_BINARY
        self.binary.parent.mkdir()
        self.binary.write_bytes(struct.pack("<8I", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0) + b"native fixture")
        self.binary.chmod(0o755)
        self.resources = self.root / "resources"
        self.resources.mkdir()
        with (self.resources / "Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleIdentifier": "dev.fleetworks.desktop", "LSMinimumSystemVersion": "13.0"}, output)
        (self.resources / "fixture.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
        self.destination = self.root / "review app output"
        self.commands = []

    def command_runner(self, command, **_kwargs):
        self.commands.append(command)
        self.assertEqual(command[:2], ["otool", "-L"])
        return subprocess.CompletedProcess(command, 0, f"{command[2]}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n", "")

    def pack(self, **overrides):
        arguments = dict(version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
                         profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
                         run_id="123", attempt="2", workflow="fixture", runner=self.command_runner)
        arguments.update(overrides)
        return package_native(self.destination, self.binary, self.resources, **arguments)

    def test_packages_complete_app_with_numeric_versions_and_honest_manifest(self):
        result = self.pack(archive=True)
        app = Path(result["app"])
        self.assertTrue((app / "Contents/MacOS" / NATIVE_BINARY).is_file())
        self.assertTrue(os.access(app / "Contents/MacOS" / NATIVE_BINARY, os.X_OK))
        self.assertEqual((app / "Contents/Resources/fixture.svg").read_bytes(), (self.resources / "fixture.svg").read_bytes())
        with (app / "Contents/Info.plist").open("rb") as source:
            plist = plistlib.load(source)
        self.assertEqual(plist["CFBundleVersion"], "1.2.1")
        self.assertEqual(plist["CFBundleShortVersionString"], "0.2.1")
        self.assertEqual(plist["LSMinimumSystemVersion"], "13.0")
        self.assertEqual(plist["CFBundleExecutable"], NATIVE_BINARY)
        manifest = json.loads(Path(result["manifest"]).read_text())
        self.assertTrue(manifest["dirty"])
        self.assertEqual(manifest["revision"], "1" * 40)
        self.assertEqual(manifest["rune_revision"], "2" * 40)
        self.assertEqual(manifest["verification"]["gui_launch"], "not_run")
        self.assertEqual(manifest["verification"]["minimum_os"], "not_tested")
        self.assertEqual(manifest["signing"]["status"], "unsigned-review")
        archive = Path(result["archive"])
        self.assertEqual(archive.name, "Fleetworks-0.2.1-111111111111-123.2-x86_64-apple-darwin.zip")
        self.assertEqual(archive.with_suffix(".zip.sha256").read_text(), checksum_line(archive, result["sha256"]))
        with zipfile.ZipFile(archive) as zipped:
            self.assertTrue(all(name == NATIVE_APP + "/" or name.startswith(NATIVE_APP + "/") for name in zipped.namelist()))
            executable = zipped.getinfo(NATIVE_APP + "/Contents/MacOS/" + NATIVE_BINARY)
            self.assertEqual((executable.external_attr >> 16) & 0o777, 0o755)
        self.assertEqual(len(self.commands), 1)

    def test_rejects_wrong_architecture_missing_icon_and_symlink_inputs(self):
        with self.assertRaisesRegex(ValueError, "wrong architecture"):
            self.pack(target="aarch64-apple-darwin")
        self.assertFalse(self.destination.exists())
        plist_path = self.resources / "Info.plist"
        with plist_path.open("rb") as source:
            plist = plistlib.load(source)
        plist["CFBundleIconFile"] = "missing.icns"
        with plist_path.open("wb") as output:
            plistlib.dump(plist, output)
        with self.assertRaisesRegex(ValueError, "icon is missing"):
            self.pack()
        plist.pop("CFBundleIconFile")
        with plist_path.open("wb") as output:
            plistlib.dump(plist, output)
        linked = self.resources / "outside.svg"
        linked.symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            self.pack()

    def test_failed_package_keeps_last_good_app_and_manifest(self):
        original = self.pack()
        old_app_bytes = (Path(original["app"]) / "Contents/MacOS" / NATIVE_BINARY).read_bytes()
        old_manifest = Path(original["manifest"]).read_bytes()
        self.binary.write_bytes(b"failed replacement")
        with self.assertRaisesRegex(ValueError, "Mach-O"):
            self.pack()
        self.assertEqual((self.destination / NATIVE_APP / "Contents/MacOS" / NATIVE_BINARY).read_bytes(), old_app_bytes)
        self.assertEqual((self.destination / NATIVE_MANIFEST).read_bytes(), old_manifest)

    def test_replacement_rollback_keeps_last_good_bundle_on_install_failure(self):
        original = self.pack()
        old_manifest = Path(original["manifest"]).read_bytes()
        self.binary.write_bytes(self.binary.read_bytes() + b"changed")
        real_replace = os.replace
        failed = False

        def replace(source, destination):
            nonlocal failed
            if Path(source).name == NATIVE_MANIFEST and Path(destination).parent == self.destination and not failed:
                failed = True
                raise OSError("fixture failed atomic replacement")
            return real_replace(source, destination)

        with mock.patch("fleetworks_candidate.os.replace", side_effect=replace):
            with self.assertRaisesRegex(OSError, "failed atomic replacement"):
                self.pack()
        self.assertEqual((self.destination / NATIVE_MANIFEST).read_bytes(), old_manifest)
        self.assertFalse((self.destination / NATIVE_APP / "Contents/MacOS" / NATIVE_BINARY).read_bytes().endswith(b"changed"))

    def test_archive_identities_are_immutable_and_unrelated_apps_are_not_overwritten(self):
        first = self.pack(archive=True)
        before = Path(first["archive"]).read_bytes()
        with self.assertRaisesRegex(ValueError, "immutable"):
            self.pack(archive=True)
        self.assertEqual(Path(first["archive"]).read_bytes(), before)
        (self.destination / NATIVE_MANIFEST).unlink()
        with self.assertRaisesRegex(ValueError, "owned app/manifest pair"):
            self.pack()
        self.assertTrue((self.destination / NATIVE_APP).is_dir())

    def test_rejects_unbundled_libraries_without_a_build_or_launch(self):
        def host_dependency(command, **_kwargs):
            return subprocess.CompletedProcess(command, 0, f"{command[2]}:\n\t/opt/homebrew/lib/fixture.dylib (compatibility version 1.0.0)\n", "")

        with self.assertRaisesRegex(ValueError, "unbundled/non-system"):
            self.pack(runner=host_dependency)
        self.assertFalse(self.destination.exists())

    def test_rejects_minimum_os_below_resource_floor_and_unsafe_outputs(self):
        with self.assertRaisesRegex(ValueError, "cannot be lower"):
            self.pack(minimum_os="12.0")
        for path in ("/Applications/review", str(Path.home() / ".fleetworks/review"), "/System/review"):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, "review directory"):
                native_output(path, "x86_64-apple-darwin", "debug")

    def test_cargo_artifact_selection_honors_absolute_custom_target_paths(self):
        for package_id in ("path+file:///repo/apps/fleetworks-desktop#0.2.1",
                           "path+file:///repo/apps/desktop#fleetworks-desktop@0.2.1",
                           "fleetworks-desktop 0.2.1 (path+file:///repo/apps/desktop)"):
            output = "\n".join(json.dumps(item) for item in [
                {"reason": "compiler-artifact", "target": {"name": NATIVE_BINARY, "kind": ["bin"]},
                 "package_id": package_id, "executable": str(self.binary)},
                {"reason": "build-finished", "success": True},
            ])
            self.assertEqual(cargo_executable(output), self.binary)
        with self.assertRaisesRegex(ValueError, "exactly one"):
            cargo_executable('{"reason":"build-finished","success":false}')
        with self.assertRaisesRegex(ValueError, "valid JSON"):
            cargo_executable("not a Cargo JSON message")

    def test_agent_build_uses_isolated_wrapper_and_numeric_apple_versions(self):
        command = cargo_native_command("x86_64-apple-darwin", "release", agent=True, root=self.root)
        self.assertEqual(command[1], str(self.root / "scripts/agent_cargo.py"))
        self.assertIn("--locked", command)
        self.assertIn("--release", command)
        self.assertNotIn("--workspace", command)
        host_debug = cargo_native_command("x86_64-apple-darwin", "debug", agent=True, root=self.root, implicit_host=True)
        self.assertIn("--locked", host_debug)
        self.assertNotIn("--target", host_debug)
        self.assertNotIn("--release", host_debug)
        explicit_debug = cargo_native_command("x86_64-apple-darwin", "debug", agent=True, root=self.root)
        self.assertEqual(explicit_debug[explicit_debug.index("--target") + 1], "x86_64-apple-darwin")
        self.assertEqual(apple_versions("0.2.1-beta.1+review"), ("0.2.1", "1.2.1"))
        with self.assertRaisesRegex(ValueError, "numeric bundle version"):
            apple_versions("0.2.1", "0.2.1-beta")

    def verify(self, result, **overrides):
        arguments = dict(expected_digest=result["sha256"], version="0.2.1",
                         target="x86_64-apple-darwin", revision="1" * 40,
                         rune_revision="2" * 40, runner=self.command_runner)
        arguments.update(overrides)
        return extract_native(result["archive"], self.root / "download check", **arguments)

    def test_downloaded_zip_checks_exact_identity_hashes_architecture_and_permissions(self):
        result = self.pack(archive=True)
        app, manifest = self.verify(result)
        self.assertEqual((app / "Contents/MacOS" / NATIVE_BINARY).read_bytes(), self.binary.read_bytes())
        self.assertEqual(manifest["revision"], "1" * 40)
        self.assertEqual(manifest["verification"]["gui_launch"], "not_run")
        self.assertEqual(len(self.commands), 2)

    def test_downloaded_zip_rejects_wrong_digest_pin_and_dirty_release_before_extraction(self):
        result = self.pack(archive=True)
        for overrides, message in [
            ({"expected_digest": "0" * 64}, "packaging job's SHA-256"),
            ({"rune_revision": "3" * 40}, "manifest does not match"),
            ({"require_clean": True}, "clean source identity"),
        ]:
            with self.subTest(overrides=overrides), self.assertRaisesRegex(ValueError, message):
                self.verify(result, **overrides)
            self.assertFalse((self.root / "download check").exists())

    def test_downloaded_zip_rejects_unsafe_paths_even_with_matching_outer_digest(self):
        result = self.pack(archive=True)
        archive = Path(result["archive"])
        with zipfile.ZipFile(archive, "w") as output:
            entry = zipfile.ZipInfo("../escape")
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            output.writestr(entry, b"unsafe")
        result["sha256"] = sha256(archive)
        archive.with_suffix(".zip.sha256").write_text(checksum_line(archive, result["sha256"]))
        with self.assertRaisesRegex(ValueError, "unsafe or unexpected path"):
            self.verify(result)
        self.assertFalse((self.root / "escape").exists())
        self.assertFalse((self.root / "download check").exists())

    def test_downloaded_zip_rejects_changed_binary_before_extraction(self):
        result = self.pack(archive=True)
        archive = Path(result["archive"])
        with zipfile.ZipFile(archive) as original:
            entries = [(entry, original.read(entry)) for entry in original.infolist()]
        with zipfile.ZipFile(archive, "w") as changed:
            for entry, payload in entries:
                if entry.filename.endswith("/MacOS/" + NATIVE_BINARY):
                    payload += b"changed"
                changed.writestr(entry, payload)
        result["sha256"] = sha256(archive)
        archive.with_suffix(".zip.sha256").write_text(checksum_line(archive, result["sha256"]))
        with self.assertRaisesRegex(ValueError, "file digest mismatch"):
            self.verify(result)
        self.assertFalse((self.root / "download check").exists())


if __name__ == "__main__":
    unittest.main()
