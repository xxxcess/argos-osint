"""Offline contracts for the native Mac candidate package."""

import io
import json
import os
import plistlib
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from fleetworks_candidate import (
    COMPANION_BINARY, COMPANION_PACKAGE, MAX_NATIVE_ARCHIVE_BYTES, MAX_NATIVE_UNPACKED_BYTES,
    NATIVE_APP, NATIVE_BINARY, NATIVE_MANIFEST,
)
from fleetworks_candidate import main, native_main, open_native, _run_cargo, apple_versions, cargo_companion_command, cargo_executable, cargo_native_command
from fleetworks_candidate import artifact_version, checksum_line, elf_architecture, extract_native, linux_runtime_preflight, native_output, package_native, sha256, verify_engine_free


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
        if command[:2] == ["strip", "-S"]:
            return subprocess.CompletedProcess(command, 0, "", "")
        self.assertEqual(command[:2], ["otool", "-L"])
        return subprocess.CompletedProcess(command, 0, f"{command[2]}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n", "")

    def pack(self, **overrides):
        arguments = dict(version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
                         profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
                         run_id="123", attempt="2", workflow="fixture", runner=self.command_runner)
        arguments.update(overrides)
        return package_native(self.destination, self.binary, self.resources, **arguments)

    def test_retired_terminal_archive_commands_are_not_supported(self):
        for command in ("pack", "verify"):
            with self.subTest(command=command), mock.patch.object(sys, "argv", ["fleetworks_candidate.py", command]), \
                 mock.patch("sys.stderr"), self.assertRaises(SystemExit) as exited:
                main()
            self.assertEqual(exited.exception.code, 2)

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
        self.assertEqual([cmd[:2] for cmd in self.commands], [["otool", "-L"], ["strip", "-S"]])

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

    def test_explicit_channel_version_is_bound_to_bundle_archive_and_verifier(self):
        version = "0.3.0+dev.g" + "1" * 40
        result = self.pack(version=version, archive=True)
        manifest = json.loads(Path(result["manifest"]).read_text())
        self.assertEqual(manifest["version"], version)
        self.assertIn("Fleetworks-" + version + "-", Path(result["archive"]).name)
        with (Path(result["app"]) / "Contents/Info.plist").open("rb") as source:
            self.assertEqual(plistlib.load(source)["CFBundleShortVersionString"], "0.3.0")
        archive = Path(result["archive"])
        destination = self.root / "channel download"
        command = ["fleetworks_candidate.py", "verify-native", "--archive", str(archive),
                   "--sha256", result["sha256"], "--revision", "1" * 40,
                   "--target", "x86_64-apple-darwin", "--rune-revision", "2" * 40,
                   "--version", version, "--directory", str(destination)]
        output = io.StringIO()
        with mock.patch.object(sys, "argv", command), mock.patch("sys.stdout", output), \
             mock.patch("fleetworks_candidate._run", side_effect=lambda args, **_: self.command_runner(args).stdout):
            self.assertEqual(main(), 0)
        self.assertEqual(json.loads(output.getvalue())["version"], version)
        self.assertTrue((destination / NATIVE_APP).is_dir())

    def test_explicit_version_is_strict_safe_and_preserves_source_baseline(self):
        baseline = self.root / "version.txt"
        baseline.write_text("0.2.1\n")
        self.assertEqual(artifact_version(root=self.root), "0.2.1")
        for version in ("0.3.0-nightly.2.g" + "1" * 40, "0.3.0+main.g" + "1" * 40):
            with self.subTest(version=version):
                self.assertEqual(artifact_version(version, root=self.root), version)
        for version in ("../0.3.0", "0.3.0\npublish=true", "01.2.3", "0.3.0-01", "0.3.0-..", "0.3.0+", "0.3.0+" + "x" * 161):
            with self.subTest(version=version), self.assertRaises(ValueError):
                artifact_version(version, root=self.root)
        self.assertEqual(baseline.read_text(), "0.2.1\n")

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
        self.assertEqual([cmd[:2] for cmd in self.commands], [["otool", "-L"], ["strip", "-S"], ["otool", "-L"]])

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


class NativeBuildProgressTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.binary = self.root / NATIVE_BINARY
        self.messages = "\n".join(json.dumps(message) for message in [
            {"reason": "compiler-artifact", "target": {"name": NATIVE_BINARY, "kind": ["bin"]},
             "package_id": "path+file:///fixture/apps/fleetworks-desktop#0.2.1", "executable": str(self.binary)},
            {"reason": "build-finished", "success": True},
        ])

    def test_cargo_progress_is_flushed_before_fake_compiler_exits_and_json_stays_exact(self):
        released = self.root / "release-fake-compiler"
        visible = threading.Event()
        output = io.StringIO()

        class ProgressStream:
            def write(self, value):
                output.write(value)

            def flush(self):
                if "Compiling fake native unit" in output.getvalue():
                    visible.set()

        # Deliberately omit a newline: progress must not wait for a line or EOF.
        script = """import pathlib, sys, time
sys.stderr.write('Compiling fake native unit'); sys.stderr.flush()
release = pathlib.Path(sys.argv[1])
deadline = time.monotonic() + 5
while not release.exists():
    if time.monotonic() > deadline: sys.exit(9)
    time.sleep(0.01)
sys.stdout.write(sys.argv[2]); sys.stdout.flush()
"""
        results = []
        errors = []

        def build():
            try:
                results.append(_run_cargo([sys.executable, "-u", "-c", script, str(released), self.messages], root=self.root, timeout=8))
            except BaseException as error:
                errors.append(error)

        with mock.patch("sys.stderr", ProgressStream()):
            worker = threading.Thread(target=build)
            worker.start()
            try:
                self.assertTrue(visible.wait(4), "compiler progress must be flushed before it is released to exit")
                self.assertTrue(worker.is_alive(), "visibility must precede child completion")
                self.assertEqual(results, [])
            finally:
                released.touch()
                worker.join(8)
        self.assertFalse(worker.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(results, [self.messages])
        self.assertEqual(cargo_executable(results[0]), self.binary)

    def test_failed_fake_compiler_preserves_live_diagnostics_and_bounded_error(self):
        diagnostic = "x" * 6000 + "fixture compiler error"
        script = "import sys; sys.stderr.write(sys.argv[1]); sys.stderr.flush(); sys.exit(7)"
        stream = io.StringIO()
        with mock.patch("sys.stderr", stream), self.assertRaises(ValueError) as failed:
            _run_cargo([sys.executable, "-u", "-c", script, diagnostic], root=self.root, timeout=5)
        self.assertEqual(stream.getvalue(), diagnostic)
        self.assertTrue(str(failed.exception).endswith("fixture compiler error"))
        self.assertLessEqual(len(str(failed.exception)), len(sys.executable) + len(" failed: ") + 4000)

    def test_fake_compiler_timeout_still_terminates_and_reports_timeout(self):
        script = "import sys, time; sys.stderr.write('building'); sys.stderr.flush(); time.sleep(10)"
        with mock.patch("sys.stderr", io.StringIO()), self.assertRaises(subprocess.TimeoutExpired):
            _run_cargo([sys.executable, "-u", "-c", script], root=self.root, timeout=0.1)

    def test_build_stages_are_visible_before_operations_and_stdout_keeps_artifact_interface(self):
        (self.root / "version.txt").write_text("0.2.1\n")
        (self.root / "Cargo.lock").write_text("synthetic lockfile\n")
        args = SimpleNamespace(command="build-native", target="host", profile="debug", open=True,
                               agent=False, output=None, dry_run=False, revision=None, require_clean=False,
                               resources=self.root / "resources", minimum_macos=None, bundle_version=None,
                               run_id=None, run_attempt="1", workflow="fixture", archive=False)
        target = "x86_64-apple-darwin"
        info = SimpleNamespace(system="Darwin", native_target=target)
        stderr, stdout = io.StringIO(), io.StringIO()
        result = {"app": str(self.root / NATIVE_APP), "manifest": str(self.root / NATIVE_MANIFEST), "identity": {}}
        commands = []

        def detect():
            self.assertIn("Inspecting host", stderr.getvalue())
            return info

        def run(command, **_kwargs):
            self.assertIn("Checking Xcode", stderr.getvalue())
            commands.append(command)
            return "/fixture-sdk"

        def cargo(command, **kwargs):
            self.assertIn("Building fleetworks-desktop", stderr.getvalue())
            self.assertIn("--message-format=json-render-diagnostics", stderr.getvalue())
            self.assertNotIn("--target", command, "implicit host keeps its existing cache graph")
            self.assertNotIn("ARGOS_EMBED", kwargs["environment"])
            return self.messages

        def package(*_args, **_kwargs):
            self.assertIn("Packaging and validating", stderr.getvalue())
            return result

        def launch(*_args):
            self.assertTrue(stdout.getvalue().startswith(result["app"] + "\n"))
            return "fixture launch requested"

        with mock.patch("sys.stderr", stderr), mock.patch("sys.stdout", stdout), \
             mock.patch.multiple("fleetworks_candidate", ROOT=self.root, detect=detect,
                                 resolve_native_target=lambda *_: target, validate_toolchain=lambda *_: "1.95.0",
                                 native_output=lambda *_: self.root, repository_identity=lambda **_: ("1" * 40, False),
                                 locked_rune_revision=lambda: "2" * 40, _run=run, _run_cargo=cargo,
                                 package_native=package, open_native=launch):
            self.assertEqual(native_main(args), 0)
        self.assertEqual(commands, [["xcode-select", "-p"], ["xcrun", "--sdk", "macosx", "--show-sdk-path"]])
        artifact_output = stdout.getvalue().removeprefix(result["app"] + "\n").removesuffix("fixture launch requested\n")
        self.assertEqual(json.loads(artifact_output), {key: value for key, value in result.items() if key != "identity"})

    def test_launch_stages_precede_fake_open_and_running_app_is_preserved(self):
        target = "x86_64-apple-darwin"
        info = SimpleNamespace(system="Darwin", native_target=target)
        stream = io.StringIO()
        commands = []

        def runner(command, **_kwargs):
            commands.append(command)
            if command[0] == "ps":
                self.assertIn("Checking whether Fleetworks", stream.getvalue())
            else:
                self.assertEqual(command[0], "open")
                self.assertIn("through Launch Services", stream.getvalue())
            return subprocess.CompletedProcess(command, 0, "", "")

        with mock.patch("sys.stderr", stream), mock.patch.dict(os.environ, {}, clear=True):
            self.assertIn("Requested Launch Services", open_native(self.root / NATIVE_APP, info, target, runner=runner))
        self.assertEqual(commands, [["ps", "-axo", "comm="], ["open", str(self.root / NATIVE_APP)]])
        with mock.patch("sys.stderr", io.StringIO()):
            response = open_native(self.root / NATIVE_APP, info, target, runner=lambda command, **_: subprocess.CompletedProcess(command, 0, "/fixture/fleetworks-desktop\n", ""))
        self.assertIn("already running", response)

    def test_local_review_open_forwards_only_socket_path(self):
        target = "x86_64-apple-darwin"
        info = SimpleNamespace(system="Darwin", native_target=target)
        commands = []

        def runner(command, **_kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 0, "", "")

        with mock.patch("sys.stderr", io.StringIO()), mock.patch.dict(
            os.environ, {"FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET": "/tmp/fw-review.sock"}
        ):
            open_native(self.root / NATIVE_APP, info, target, runner=runner)
        self.assertEqual(commands, [
            ["ps", "-axo", "comm="],
            ["open", "--env", "FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET=/tmp/fw-review.sock", str(self.root / NATIVE_APP)],
        ])

    def test_local_review_open_rejects_relative_socket(self):
        target = "x86_64-apple-darwin"
        info = SimpleNamespace(system="Darwin", native_target=target)
        commands = []

        def runner(command, **_kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 0, "", "")

        with mock.patch("sys.stderr", io.StringIO()), mock.patch.dict(
            os.environ, {"FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET": "review.sock"}
        ), self.assertRaisesRegex(ValueError, "absolute macOS UNIX socket"):
            open_native(self.root / NATIVE_APP, info, target, runner=runner)
        self.assertEqual(commands, [["ps", "-axo", "comm="]])


class LinuxNativeCandidateTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name).resolve()
        self.binary = self.root / "target output" / NATIVE_BINARY
        self.binary.parent.mkdir()
        elf_header = (
            b"\x7fELF"
            + bytes([2, 1, 1, 0, 0])
            + b"\x00" * 7
            + struct.pack("<HHI", 3, 62, 1)
            + b"\x00" * 40
        )
        self.binary.write_bytes(elf_header + b"linux native payload")
        self.binary.chmod(0o755)
        self.resources = self.root / "resources"
        self.resources.mkdir()
        (self.resources / "fleetworks.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
        self.destination = self.root / "linux bundle out"
        self.commands = []

    def runner(self, command, **_kwargs):
        self.commands.append(command)
        if command[:2] == ["strip", "--strip-debug"]:
            return subprocess.CompletedProcess(command, 0, "", "")
        self.assertEqual(command[:2], ["readelf", "-d"])
        return subprocess.CompletedProcess(
            command,
            0,
            " 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]\n"
            " 0x0000000000000001 (NEEDED)             Shared library: [libdbus-1.so.3]\n"
            " 0x0000000000000001 (NEEDED)             Shared library: [libvulkan.so.1]\n",
            "",
        )

    def pack(self, **overrides):
        kwargs = dict(
            target="x86_64-unknown-linux-gnu",
            profile="debug",
            revision="1" * 40,
            dirty=True,
            toolchain="1.95.0",
            rune_revision="2" * 40,
            resources=self.resources,
            runner=self.runner,
            version="0.2.1",
            archive=False,
        )
        kwargs.update(overrides)
        return package_native(self.destination, self.binary, **kwargs)

    def test_packages_relocatable_linux_tarball_without_xcode_and_verifies_roundtrip(self):
        result = self.pack(archive=True)
        archive = Path(result["archive"])
        self.assertTrue(archive.name.endswith("-x86_64-unknown-linux-gnu.tar.gz"))
        bundle = Path(result["app"])
        self.assertEqual(bundle.name, "Fleetworks-linux-x86_64")
        desktop = (bundle / "share/applications/fleetworks.desktop").read_text()
        self.assertIn(f"Exec={NATIVE_BINARY}", desktop)
        self.assertNotIn(str(self.destination), desktop)
        manifest = json.loads(Path(result["manifest"]).read_text())
        self.assertEqual(manifest["platform"], "linux")
        self.assertEqual(manifest["target"], "x86_64-unknown-linux-gnu")
        extracted_dir = self.root / "extracted review dir"
        app, verified = extract_native(
            archive,
            extracted_dir,
            expected_digest=result["sha256"],
            version="0.2.1",
            target="x86_64-unknown-linux-gnu",
            revision="1" * 40,
            rune_revision="2" * 40,
            runner=self.runner,
        )
        self.assertEqual(verified["revision"], "1" * 40)
        self.assertEqual((app / "bin" / NATIVE_BINARY).read_bytes(), self.binary.read_bytes())

    def test_rejects_disallowed_linux_shared_libraries_and_unsafe_tar_members(self):
        def bad_lib(command, **_kwargs):
            return subprocess.CompletedProcess(
                command,
                0,
                " 0x0000000000000001 (NEEDED)             Shared library: [libssl.so.3]\n",
                "",
            )

        with self.assertRaisesRegex(ValueError, "Native Linux binary links disallowed library"):
            self.pack(runner=bad_lib)
        result = self.pack(archive=True)
        archive = Path(result["archive"])
        with tarfile.open(archive, "w:gz") as output:
            info = tarfile.TarInfo("../escape")
            payload = b"bad"
            info.size = len(payload)
            output.addfile(info, io.BytesIO(payload))
        digest = sha256(archive)
        archive.with_name(archive.name + ".sha256").write_text(checksum_line(archive, digest))
        with self.assertRaisesRegex(ValueError, "unsafe or unexpected path"):
            extract_native(
                archive,
                self.root / "bad-tar",
                expected_digest=digest,
                version="0.2.1",
                target="x86_64-unknown-linux-gnu",
                revision="1" * 40,
                rune_revision="2" * 40,
                runner=self.runner,
            )

    def test_elf_arch_and_linux_open_preflight_preserve_running_instance(self):
        self.assertEqual(elf_architecture(self.binary), 62)
        empty_icd = self.root / "empty-icd"
        empty_icd.mkdir()
        icd_dir = self.root / "vulkan-icd"
        icd_dir.mkdir()
        (icd_dir / "lvp_icd.json").write_text("{}")
        with mock.patch("fleetworks_candidate.shutil.which", return_value=None):
            with self.assertRaisesRegex(ValueError, "WAYLAND_DISPLAY or DISPLAY"):
                linux_runtime_preflight({}, vulkan_icd_dirs=[empty_icd])
            with self.assertRaisesRegex(ValueError, "Vulkan"):
                linux_runtime_preflight({"DISPLAY": ":0"}, vulkan_icd_dirs=[empty_icd])
        with self.assertRaisesRegex(ValueError, "Vulkan"):
            linux_runtime_preflight(
                {"DISPLAY": ":0"},
                vulkan_icd_dirs=[empty_icd],
                runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 1, "", "vulkaninfo unavailable"),
            )
        self.assertEqual(
            linux_runtime_preflight(
                {"WAYLAND_DISPLAY": "wayland-0"},
                vulkan_icd_dirs=[empty_icd],
                runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "GPU0: llvmpipe", ""),
            ),
            {"display": "wayland", "dbus_session": False, "vulkan": True},
        )
        self.assertEqual(
            linux_runtime_preflight({"DISPLAY": ":0", "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus"}, vulkan_icd_dirs=[icd_dir]),
            {"display": "x11", "dbus_session": True, "vulkan": True},
        )

        target = "x86_64-unknown-linux-gnu"
        info = SimpleNamespace(system="Linux", native_target=target)
        bundle = Path(self.pack()["app"])
        launched = []
        with mock.patch("sys.stderr", io.StringIO()), \
             mock.patch.dict(os.environ, {"DISPLAY": ":0"}, clear=True), \
             mock.patch("fleetworks_candidate.subprocess.Popen", side_effect=lambda cmd, **_kw: launched.append(cmd) or SimpleNamespace(pid=42)):
            response = open_native(
                bundle,
                info,
                target,
                runner=lambda cmd, **_: launched.append(cmd) or subprocess.CompletedProcess(cmd, 0, "", ""),
                preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
            )
        self.assertIn("Launched Linux native executable in an isolated session", response)
        self.assertEqual(launched, [["ps", "-axo", "comm="], [str(bundle / "bin" / NATIVE_BINARY)]])
        launched.clear()
        with mock.patch("sys.stderr", io.StringIO()), \
             mock.patch.dict(os.environ, {"DISPLAY": ":0"}, clear=True), \
             mock.patch("fleetworks_candidate._run", return_value=""), \
             mock.patch("fleetworks_candidate.subprocess.Popen", side_effect=lambda cmd, **_kw: launched.append(cmd) or SimpleNamespace(pid=42)):
            open_native(
                bundle,
                info,
                target,
                preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
            )
        self.assertEqual(launched, [[str(bundle / "bin" / NATIVE_BINARY)]])
        with mock.patch("sys.stderr", io.StringIO()), \
             mock.patch.dict(os.environ, {"DISPLAY": ":0", "FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET": "/tmp/s.sock"}, clear=True):
            with self.assertRaisesRegex(ValueError, "macOS-only"):
                open_native(
                    bundle,
                    info,
                    target,
                    runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "", ""),
                    preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
                )

    def test_linux_native_main_skips_xcode_checks(self):
        (self.root / "version.txt").write_text("0.2.1\n")
        (self.root / "Cargo.lock").write_text("lock\n")
        target = "x86_64-unknown-linux-gnu"
        info = SimpleNamespace(system="Linux", native_target=target)
        args = SimpleNamespace(
            command="build-native", target="host", profile="debug", open=False,
            agent=False, output=None, dry_run=False, revision=None, require_clean=False,
            resources=self.resources, minimum_macos=None, bundle_version=None,
            run_id=None, run_attempt="1", workflow="fixture", archive=False,
        )
        run_calls = []
        with mock.patch("sys.stderr", io.StringIO()), mock.patch("sys.stdout", io.StringIO()), \
             mock.patch.multiple(
                 "fleetworks_candidate",
                 ROOT=self.root,
                 detect=lambda: info,
                 resolve_native_target=lambda *_: target,
                 validate_toolchain=lambda *_: "1.95.0",
                 native_output=lambda *_: self.destination,
                 repository_identity=lambda **_: ("1" * 40, False),
                 locked_rune_revision=lambda: "2" * 40,
                 _run=lambda cmd, **kw: run_calls.append(cmd) or "",
                 _run_cargo=lambda *_a, **_kw: json.dumps({
                     "reason": "compiler-artifact",
                     "target": {"name": NATIVE_BINARY, "kind": ["bin"]},
                     "package_id": "path+file:///fixture/apps/fleetworks-desktop#0.2.1",
                     "executable": str(self.binary),
                 }) + "\n" + json.dumps({"reason": "build-finished", "success": True}),
                 package_native=lambda *_a, **_kw: {"app": str(self.destination / "Fleetworks-linux-x86_64"), "manifest": str(self.destination / "manifest.json"), "identity": {}},
             ):
            self.assertEqual(native_main(args), 0)
        self.assertEqual(run_calls, [["readelf", "--version"]])

    def test_companion_packaging_and_extraction_verify_macos_and_linux_archives(self):
        mac_bin = self.root / "mac-desktop"
        mac_bin.write_bytes(struct.pack("<8I", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0) + b"native fixture")
        mac_bin.chmod(0o755)
        mac_comp = self.root / COMPANION_BINARY
        mac_comp.write_bytes(mac_bin.read_bytes())
        mac_comp.chmod(0o755)
        mac_res = self.root / "mac-resources"
        mac_res.mkdir()
        with (mac_res / "Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleIdentifier": "dev.fleetworks.desktop", "LSMinimumSystemVersion": "13.0"}, output)
        mac_runner = lambda cmd, **_: subprocess.CompletedProcess(
            cmd, 0, f"{cmd[2]}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n", ""
        )
        result = package_native(
            self.destination / "mac", mac_bin, mac_res,
            version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
            profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
            run_id="123", attempt="2", workflow="fixture", archive=True,
            companion_binary=mac_comp, runner=mac_runner,
        )
        manifest = result["identity"]
        self.assertEqual(manifest["companion"]["name"], COMPANION_BINARY)
        self.assertEqual(manifest["companion"]["relative_path"], f"Contents/MacOS/{COMPANION_BINARY}")
        self.assertEqual(manifest["companion"]["sha256"], sha256(mac_comp))
        app, verified = extract_native(
            result["archive"], self.root / "extracted-mac", expected_digest=result["sha256"],
            version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
            rune_revision="2" * 40, profile="debug", runner=mac_runner,
        )
        self.assertTrue((app / "Contents/MacOS" / COMPANION_BINARY).is_file())
        self.assertEqual(verified["companion"]["sha256"], sha256(mac_comp))

        # Verify Linux tar.gz packaging & extraction of companion executable.
        linux_comp = self.root / "linux-comp"
        linux_comp.write_bytes(self.binary.read_bytes())
        linux_comp.chmod(0o755)
        linux_runner = lambda cmd, **_: subprocess.CompletedProcess(
            cmd, 0, "  0x0000000000000001 (NEEDED) Shared library: [libc.so.6]\n", ""
        )
        linux_res = self.pack(
            companion_binary=linux_comp, archive=True,
            runner=linux_runner,
        )
        lapp, lmanifest = extract_native(
            linux_res["archive"], self.root / "extracted-linux", expected_digest=linux_res["sha256"],
            version="0.2.1", target="x86_64-unknown-linux-gnu", revision="1" * 40,
            rune_revision="2" * 40, profile="debug", runner=linux_runner,
        )
        self.assertTrue((lapp / "bin" / COMPANION_BINARY).is_file())
        self.assertEqual(lmanifest["companion"]["relative_path"], f"bin/{COMPANION_BINARY}")

    def test_cargo_companion_command_and_verify_engine_free_bounds(self):
        cmd = cargo_companion_command("aarch64-apple-darwin", "release", agent=True,
                                      implicit_host=True, features="embedded", no_default_features=True)
        self.assertIn(COMPANION_PACKAGE, cmd)
        self.assertIn("--no-default-features", cmd)
        self.assertEqual(cmd[-3:], ["--features", "embedded", "--release"])
        fake_msg = json.dumps({
            "reason": "compiler-artifact",
            "target": {"name": COMPANION_BINARY, "kind": ["bin"]},
            "package_id": "path+file:///repo/services/api-host#0.2.1",
            "executable": "/tmp/argos-api-host",
        }) + "\n" + json.dumps({"reason": "build-finished", "success": True})
        self.assertEqual(
            cargo_executable(fake_msg, name=COMPANION_BINARY, package=COMPANION_PACKAGE),
            Path("/tmp/argos-api-host"),
        )

        def clean_runner(command, **_kw):
            if "--invert" in command:
                return subprocess.CompletedProcess(command, 0, "0rusqlite v0.41.1\n1fleetworks-jobs v0.2.1\n", "")
            return subprocess.CompletedProcess(command, 0, "fleetworks-contracts v0.2.1\ntokio v1.53.3\n", "")

        summary = verify_engine_free(self.root, runner=clean_runner)
        self.assertTrue(summary["client_engine_free"])
        self.assertEqual(summary["rusqlite_parents"], ["fleetworks-jobs"])

        def leaky_client_runner(command, **_kw):
            return subprocess.CompletedProcess(command, 0, "fleetworks-desktop v0.2.1\nsurrealdb v3.3.4\n", "")

        with self.assertRaisesRegex(ValueError, "Engine-free native crates"):
            verify_engine_free(self.root, runner=leaky_client_runner)

        def leaky_sqlite_runner(command, **_kw):
            if "--invert" in command:
                return subprocess.CompletedProcess(command, 0, "0rusqlite v0.41.1\n1argos-osint-api v0.2.1\n", "")
            return subprocess.CompletedProcess(command, 0, "argos-api-host v0.2.1\n", "")

        with self.assertRaisesRegex(ValueError, "fleetworks-jobs"):
            verify_engine_free(self.root, runner=leaky_sqlite_runner)

    def test_strips_debug_sections_updates_companion_digest_and_enforces_size_limits(self):
        self.assertEqual(MAX_NATIVE_UNPACKED_BYTES, 1024 * 1024 * 1024)
        self.assertEqual(MAX_NATIVE_ARCHIVE_BYTES, 512 * 1024 * 1024)
        mac_bin = self.root / "strip-mac-bin"
        mac_bin.write_bytes(struct.pack("<8I", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0) + b"payloadDEBUG_DATA")
        mac_bin.chmod(0o755)
        mac_comp = self.root / f"{COMPANION_BINARY}-strip"
        mac_comp.write_bytes(mac_bin.read_bytes() + b"_MORE_DEBUG")
        mac_comp.chmod(0o755)
        mac_res = self.root / "strip-mac-res"
        mac_res.mkdir()
        with (mac_res / "Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleIdentifier": "dev.fleetworks.desktop", "LSMinimumSystemVersion": "13.0"}, output)

        strip_calls = []

        def stripping_runner(cmd, **_kwargs):
            if cmd[:2] == ["strip", "-S"]:
                strip_calls.append(cmd)
                staged = Path(cmd[2])
                staged.write_bytes(staged.read_bytes()[:40])
                return subprocess.CompletedProcess(cmd, 0, "", "")
            return subprocess.CompletedProcess(
                cmd, 0, f"{cmd[2]}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n", ""
            )

        res = package_native(
            self.destination / "stripped-mac", mac_bin, mac_res,
            version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
            profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
            archive=True, companion_binary=mac_comp, runner=stripping_runner,
        )
        self.assertEqual(len(strip_calls), 2)
        self.assertEqual(res["executable_bytes"], 40)
        self.assertEqual(res["companion_bytes"], 40)
        self.assertLess(res["executable_bytes"], mac_bin.stat().st_size)
        self.assertLess(res["companion_bytes"], mac_comp.stat().st_size)
        staged_comp = Path(res["companion"])
        self.assertEqual(res["identity"]["companion"]["sha256"], sha256(staged_comp))
        self.assertNotEqual(res["identity"]["companion"]["sha256"], sha256(mac_comp))
        self.assertGreater(res["unpacked_bytes"], 0)
        self.assertGreater(res["archive_bytes"], 0)

        with mock.patch("fleetworks_candidate.MAX_NATIVE_UNPACKED_BYTES", 10):
            with self.assertRaisesRegex(ValueError, "1 GiB unpacked limit"):
                package_native(
                    self.destination / "too-big-unpacked", mac_bin, mac_res,
                    version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
                    profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
                    runner=stripping_runner,
                )

        with mock.patch("fleetworks_candidate.MAX_NATIVE_ARCHIVE_BYTES", 10):
            with self.assertRaisesRegex(ValueError, "512 MiB compressed limit"):
                package_native(
                    self.destination / "too-big-archive", mac_bin, mac_res,
                    version="0.2.1", target="x86_64-apple-darwin", revision="1" * 40,
                    profile="debug", rune_revision="2" * 40, toolchain="1.95.0", dirty=True,
                    archive=True, runner=stripping_runner,
                )

    def test_diagnostic_foreground_launch_captures_early_crash_and_healthy_probe(self):
        bundle = Path(self.pack()["app"])
        info = SimpleNamespace(system="Linux", native_target="x86_64-unknown-linux-gnu")
        log_root = tempfile.TemporaryDirectory()
        self.addCleanup(log_root.cleanup)
        tempdir_patch = mock.patch("tempfile.tempdir", log_root.name)
        tempdir_patch.start()
        self.addCleanup(tempdir_patch.stop)

        class FakeProc:
            def __init__(self, rc, err_data=b"", out_data=b"", **popen_kwargs):
                self.pid = 777
                self._rc = rc
                # Mimic a child writing to the descriptors the probe hands it.
                for key, data in (("stdout", out_data), ("stderr", err_data)):
                    handle = popen_kwargs.get(key)
                    if data and hasattr(handle, "write"):
                        handle.write(data)
                        handle.flush()

            def poll(self):
                return self._rc

        popen_cmds = []

        def fail_popen(cmd, **kwargs):
            popen_cmds.append(cmd)
            return FakeProc(
                101,
                err_data=b"early_crash_marker: Authorization: Bearer topsecret-abc password=hunter2\n",
                out_data=b"api_token=tok_xyz_999\n",
                **kwargs,
            )

        with mock.patch("sys.stderr", io.StringIO()), \
             mock.patch.dict(os.environ, {"DISPLAY": ":0"}, clear=True):
            with self.assertRaises(ValueError) as early_ctx:
                open_native(
                    bundle, info, "x86_64-unknown-linux-gnu",
                    diagnostic=True, startup_probe_seconds=0.01,
                    runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "", ""),
                    preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
                    popen=fail_popen,
                )
        err_msg = str(early_ctx.exception)
        self.assertIn("early_crash_marker", err_msg)
        self.assertIn("process_started=true", err_msg)
        self.assertIn("window_readiness_observed=false", err_msg)
        self.assertNotIn("topsecret-abc", err_msg)
        self.assertNotIn("hunter2", err_msg)
        self.assertNotIn("tok_xyz_999", err_msg)
        self.assertIn("[REDACTED]", err_msg)
        self.assertEqual(popen_cmds, [[str(bundle / "bin" / NATIVE_BINARY), "--diagnostic"]])

        with mock.patch("sys.stderr", io.StringIO()), \
             mock.patch.dict(os.environ, {"DISPLAY": ":0"}, clear=True):
            healthy = open_native(
                bundle, info, "x86_64-unknown-linux-gnu",
                diagnostic=True, startup_probe_seconds=0.01,
                runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "", ""),
                preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
                popen=lambda *_a, **_kw: FakeProc(None),
            )
            clean_exit = open_native(
                bundle, info, "x86_64-unknown-linux-gnu",
                diagnostic=True, startup_probe_seconds=0.01,
                runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "", ""),
                preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
                popen=lambda *_a, **kw: FakeProc(0, err_data=b"diagnostic ok Bearer secret-clean\n", **kw),
            )
            with self.assertRaisesRegex(ValueError, "process_started=false, window_readiness_observed=false"):
                open_native(
                    bundle, info, "x86_64-unknown-linux-gnu",
                    diagnostic=True, startup_probe_seconds=0.01,
                    runner=lambda cmd, **_: subprocess.CompletedProcess(cmd, 0, "", ""),
                    preflight=lambda: {"display": "x11", "dbus_session": False, "vulkan": True},
                    popen=mock.Mock(side_effect=OSError("exec format error")),
                )
        self.assertIn("process_started=true", healthy)
        self.assertIn("pid=777", healthy)
        self.assertIn("window_readiness_observed=false", healthy)
        self.assertIn("exited cleanly (0", clean_exit)
        self.assertIn("process_started=true", clean_exit)
        self.assertIn("window_readiness_observed=false", clean_exit)
        self.assertNotIn("secret-clean", clean_exit)
        self.assertIn("[REDACTED]", clean_exit)

    def test_diagnostic_launch_real_child_cannot_block_and_keeps_delayed_errors(self):
        from fleetworks_candidate import _diagnostic_foreground_launch

        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            child = tmp / "chatty-child"
            # Writes ~256 KiB (well past a 64 KiB pipe buffer), then errors after the probe window.
            child.write_text(
                f"#!{sys.executable}\n"
                "import sys, time\n"
                "sys.stdout.write('x' * 262144)\n"
                "sys.stdout.flush()\n"
                "sys.stderr.write('started token=early-secret\\n')\n"
                "sys.stderr.flush()\n"
                "time.sleep(0.6)\n"
                "sys.stderr.write('delayed_error_marker\\n')\n"
                "sys.stderr.flush()\n"
                "sys.exit(3)\n"
            )
            child.chmod(0o755)
            log_dir = tmp / "logs"
            started = time.monotonic()
            message = _diagnostic_foreground_launch(child, tmp, startup_probe_seconds=0.3, log_dir=log_dir)
            self.assertLess(time.monotonic() - started, 5)
            self.assertIn("still running", message)
            self.assertIn(str(log_dir), message)
            self.assertNotIn("early-secret", message)
            deadline = time.monotonic() + 10
            while "delayed_error_marker" not in (log_dir / "stderr.log").read_text() and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertIn("delayed_error_marker", (log_dir / "stderr.log").read_text())
            self.assertEqual((log_dir / "stdout.log").stat().st_size, 262144)

    def test_diagnostic_launch_survives_unavailable_log_dir(self):
        from fleetworks_candidate import _diagnostic_foreground_launch

        with tempfile.TemporaryDirectory() as tmp:
            blocker = Path(tmp) / "not-a-dir"
            blocker.write_text("")

            class Proc:
                pid = 42

                def poll(self):
                    return None

            message = _diagnostic_foreground_launch(
                Path(tmp) / "bin", tmp, startup_probe_seconds=0.0,
                popen=lambda *_a, **_kw: Proc(), log_dir=blocker / "logs",
            )
        self.assertIn("still running", message)
        self.assertIn("diagnostic logs unavailable", message)


if __name__ == "__main__":
    unittest.main()
