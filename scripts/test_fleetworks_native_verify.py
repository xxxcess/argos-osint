"""Offline native selection/build/evidence contracts; no Rust/GPU invocation."""

import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest import mock
import zlib

import fleetworks_native_verify as native
from fleetworks_candidate import cargo_executable
from fleetworks_platform import PlatformInfo


def options(*argv):
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    native.add_parser(commands)
    return parser.parse_args(["native-ui", *argv])


def png(width, height):
    def chunk(kind, value):
        return struct.pack(">I", len(value)) + kind + value + struct.pack(">I", zlib.crc32(kind + value) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress((b"\0" + b"\x80\x80\x80\xff" * width) * height)) + chunk(b"IEND", b"")


class NativeSelectionTests(unittest.TestCase):
    def test_debug_host_build_reuses_implicit_graph_and_explicit_target_is_reviewable(self):
        command = native.build_command(agent=True)
        self.assertIn("agent_cargo.py", command[1])
        self.assertIn("--locked", command)
        self.assertNotIn("--release", command)
        self.assertNotIn("--target", command)
        self.assertNotIn("run", command)
        self.assertIn("--target", native.build_command(target="x86_64-apple-darwin"))

    def test_example_artifact_uses_same_package_and_success_fences_as_packager(self):
        valid = {"reason": "compiler-artifact", "package_id": "path+file:///repo/apps/fleetworks-desktop#fleetworks-desktop@0.2.0",
                 "target": {"name": "native_fixture", "kind": ["example"]}, "executable": "/cache with spaces/native_fixture"}
        irrelevant = {**valid, "package_id": "registry+https://index#unrelated@0.2.0", "executable": "/other/native_fixture"}
        output = "\n".join(map(json.dumps, [irrelevant, valid, {"reason": "build-finished", "success": True}]))
        self.assertEqual(cargo_executable(output, name="native_fixture", kind="example"), Path(valid["executable"]))
        for messages in ([valid], [{**valid, "target": {"name": "native_fixture", "kind": ["bin"]}}, {"reason": "build-finished", "success": True}],
                         [valid, {**valid, "executable": "/second/native_fixture"}, {"reason": "build-finished", "success": True}]):
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                cargo_executable("\n".join(map(json.dumps, messages)), name="native_fixture", kind="example")

    def test_zero_unknown_inventory_invalid_profiles_and_excess_capture_requests_fail(self):
        for values in ("", "page-one\npage-one\n", "Scenes:\npage-one\n", "../escape\n"):
            with self.subTest(values=values), self.assertRaises(ValueError):
                native.inventory(values)
        self.assertEqual(native.inventory("future-scene\nanother_scene\n"), ["future-scene", "another_scene"])
        with self.assertRaisesRegex(ValueError, "at least one"):
            native.selections(options(), Path.cwd())
        for size, scale in (("0x800", "1"), ("4097x800", "1"), ("4096x4096", "2"), ("64x64", "nan"), ("64x64", "2.5")):
            with self.subTest(size=size, scale=scale), self.assertRaises(ValueError):
                native.parse_profiles([size], [scale])
        values = options(*[item for index in range(129) for item in ("--scene", f"scene-{index}")], "--size", "64x64", "--scale", "1")
        with self.assertRaisesRegex(ValueError, "128"):
            native.selections(values, Path.cwd())


class NativeEvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.95.0"\n')
        (self.root / "Cargo.lock").write_text("\n".join(f'[[package]]\nname = "{name}"\nsource = "git+https://fixture.invalid/rune#{"2" * 40}"' for name in ("rune", "rune-kit")))
        self.binary = self.root / "actual cargo directory with spaces" / "native_fixture"
        self.binary.parent.mkdir()
        self.binary.write_bytes(struct.pack("<8I", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0))
        self.binary.chmod(0o755)
        self.info = PlatformInfo("Darwin", "x86_64", "x86_64", False, "x86_64-apple-darwin", "1.95.0", "x86_64-apple-darwin", "linux/amd64")
        self.commands = []
        self.fail = ""
        self.environment = mock.patch.dict(os.environ, {"CI": "", "CARGO_BUILD_TARGET": ""})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def arguments(self, *extra):
        return options("--scene", "page-one", "--size", "64x64", "--scale", "1", "--agent", "--output", str(self.root / "evidence"), *extra)

    def runner(self, command, **kwargs):
        self.commands.append(list(command))
        if command[:3] == ["git", "rev-parse", "HEAD"]:
            return subprocess.CompletedProcess(command, 0, "1" * 40 + "\n", "")
        if command[:2] == ["git", "status"]:
            return subprocess.CompletedProcess(command, 0, "", "")
        self.assertEqual(Path(kwargs["cwd"]), self.root)
        if "--example" in command:
            messages = [{"reason": "compiler-artifact", "package_id": "path+file:///repo/apps/fleetworks-desktop#fleetworks-desktop@0.2.0",
                         "target": {"name": "native_fixture", "kind": ["example"]}, "executable": str(self.binary)},
                        {"reason": "build-finished", "success": True}]
            kwargs["stdout"].write("\n".join(map(json.dumps, messages)).encode())
        elif "--list-scenes" in command:
            self.assertNotEqual(kwargs["env"]["HOME"], str(Path.home()))
            self.assertNotIn("FLEETWORKS_API_TOKEN_FILE", kwargs["env"])
            kwargs["stdout"].write(b"page-one\nfuture-scene\n")
        else:
            def value(flag):
                return command[command.index(flag) + 1]
            width, height = map(int, value("--size").split("x"))
            scale = int(value("--scale"))
            if self.fail != "missing-png":
                image = png(width * scale + (1 if self.fail == "dimensions" else 0), height * scale)
                Path(value("--snap")).write_bytes(image[:-8] if self.fail == "truncated-png" else image)
            if self.fail != "missing-tree":
                keys = ["fixture-marker"] + (["dossier-name"] if "--script" in command and self.fail != "script-key" else [])
                tree = {"viewport": [width, height], "scale": scale, "elements": [{"key": key} for key in keys], "accessibility": {"key": "root"}}
                Path(value("--tree")).write_text(json.dumps(tree) if self.fail != "invalid-tree" else "{}")
            if "--trace" in command:
                trace = [{"type": "header", "clock": "virtual", "width": width, "height": height, "scale": scale}, {"type": "frame"}]
                Path(value("--trace")).write_text("\n".join(map(json.dumps, trace)))
        return subprocess.CompletedProcess(command, 7 if self.fail == "renderer-exit" and "--snap" in command else 0)

    def execute(self, args):
        return native.execute(args, root=self.root, runner=self.runner, info=self.info)

    def test_builds_once_resolves_actual_path_and_keeps_generated_inspection_gui_distinct(self):
        self.assertEqual(self.execute(self.arguments("--scene", "future-scene")), 0)
        manifest = json.loads((self.root / "evidence/manifest.json").read_text())
        self.assertEqual(len([command for command in self.commands if "--example" in command]), 1)
        self.assertEqual(manifest["fixture"]["path"], str(self.binary))
        self.assertEqual(manifest["source_revision"], "1" * 40)
        self.assertEqual(manifest["rune_revision"], "2" * 40)
        self.assertEqual(manifest["state"], "generated")
        self.assertEqual(len(manifest["captures"]), 2)
        self.assertTrue(all(case["generated"] and case["artifacts"]["tree"]["sha256"] for case in manifest["captures"]))
        self.assertEqual(manifest["visual_inspection"], "not_performed")
        self.assertEqual(manifest["real_gui"], "not_run")
        self.assertEqual(manifest["unit_tests"], "not_run_by_this_command")
        self.assertFalse(any("--target" in command for command in self.commands))

    def test_unknown_selected_scene_fails_before_render_and_retains_failed_evidence(self):
        with self.assertRaisesRegex(ValueError, "absent"):
            self.execute(self.arguments("--scene", "unknown"))
        self.assertFalse(any("--snap" in command for command in self.commands))
        self.assertEqual(json.loads((self.root / "evidence/manifest.json").read_text())["state"], "failed")

    def test_missing_invalid_mismatched_and_failed_renderer_outputs_never_become_generated(self):
        for failure in ("missing-png", "missing-tree", "dimensions", "truncated-png", "invalid-tree", "renderer-exit"):
            with self.subTest(failure=failure):
                self.fail = failure
                args = self.arguments()
                args.output = self.root / failure
                with self.assertRaises(ValueError):
                    self.execute(args)
                manifest = json.loads((args.output / "manifest.json").read_text())
                self.assertEqual(manifest["state"], "failed")
                self.assertFalse(manifest["captures"][0]["generated"])

    def test_script_is_an_additional_case_and_requires_its_real_final_key(self):
        script = self.root / "selected.json"
        script.write_text('{"events":[{"t":0.2,"type":"click","key":"synthetic-hit"}]}')
        args = self.arguments("--script", "page-one=selected.json", "--script-expect-key", "page-one=dossier-name")
        self.assertEqual(self.execute(args), 0)
        manifest = json.loads((args.output / "manifest.json").read_text())
        self.assertEqual(len(manifest["captures"]), 2)
        self.assertIn("trace", manifest["captures"][1]["artifacts"])
        self.fail = "script-key"
        args.output = self.root / "missing-key"
        with self.assertRaisesRegex(ValueError, "script key"):
            self.execute(args)

    def test_listing_is_actual_inventory_only_and_stale_output_is_not_overwritten(self):
        args = options("--list-scenes", "--agent", "--output", str(self.root / "listing"))
        self.assertEqual(self.execute(args), 0)
        self.assertFalse(any("--snap" in command for command in self.commands))
        manifest = json.loads((args.output / "manifest.json").read_text())
        self.assertEqual(manifest["state"], "inventory_recorded")
        self.assertEqual(manifest["captures"], [])
        before = (args.output / "manifest.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "new or empty"):
            self.execute(args)
        self.assertEqual((args.output / "manifest.json").read_bytes(), before)

    def test_physical_mismatch_and_escaping_or_late_scripts_fail_before_any_build(self):
        args = self.arguments()
        args.target = "aarch64-apple-darwin"
        with self.assertRaisesRegex(ValueError, "physical"):
            self.execute(args)
        for specification in ("page-one=../outside.json", "unselected=inside.json"):
            with self.subTest(specification=specification), self.assertRaises(ValueError):
                native.selections(self.arguments("--script", specification), self.root)
        (self.root / "late.json").write_text('{"events":[{"t":5,"type":"click"}]}')
        with self.assertRaisesRegex(ValueError, "event time"):
            native.selections(self.arguments("--script", "page-one=late.json"), self.root)
        self.assertFalse(any("--example" in command for command in self.commands))

    def test_excess_command_output_is_bounded_and_never_passes(self):
        output=self.root / "bounded-logs"
        output.mkdir()
        records=[]
        def noisy(command,**kwargs):
            kwargs["stdout"].write(b"x"*129)
            return subprocess.CompletedProcess(command,0)
        with self.assertRaisesRegex(ValueError,"byte bound"):
            native.run_logged(["fixture-command"],output,"bounded",records,environment={},runner=noisy,timeout=1,stdout_limit=128,root=self.root)
        self.assertEqual((output / "bounded.stdout.log").stat().st_size,128)
        self.assertEqual(records[0]["truncated"]["original_bytes"],129)
        self.assertEqual(len(records[0]["logs"][0]["sha256"]),64)


if __name__ == "__main__":
    unittest.main()
