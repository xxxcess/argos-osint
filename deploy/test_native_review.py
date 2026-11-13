"""Offline credential/provisioning contracts; no Docker or real workspace."""

import io
import json
import os
import re
import shlex
import stat
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path
from unittest import mock

import native_review
from native_review import NATIVE_PERMISSIONS, TOKEN_NAMES, expose_root_environment, provision, review_path


class NativeReviewTests(unittest.TestCase):
    def test_root_environment_is_private_and_review_alias_follows_atomic_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            target = root / "compose.env"
            alias = Path(receipt["workspace"]) / "compose.env"
            self.assertTrue(target.is_file())
            self.assertFalse(target.is_symlink())
            self.assertTrue(alias.is_symlink())
            self.assertEqual(alias.resolve(), target.resolve())
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
            replacement = root / "replacement.env"
            replacement.write_text(target.read_text().replace("'18091'", "'18093'"))
            replacement.chmod(0o600)
            replacement.replace(target)
            self.assertIn("FLEETWORKS_API_PORT='18093'", alias.read_text())

    def test_existing_root_environment_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "compose.env"
            target.write_text("existing operator settings\n")
            with self.assertRaises(FileExistsError):
                provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            self.assertEqual(target.read_text(), "existing operator settings\n")
            self.assertFalse(review_path(root, "local").exists())
            target.unlink()
            target.symlink_to(root / "missing.env")
            with self.assertRaises(FileExistsError):
                provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=True)
            self.assertTrue(target.is_symlink())
            self.assertFalse(review_path(root, "local").exists())

    def test_existing_review_can_expose_settings_without_changing_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = provision(root, "local", 18091, "linux/amd64", "a" * 40)
            workspace = Path(receipt["workspace"])
            before = {path.name: path.read_bytes() for path in (workspace / "secrets").iterdir()}
            original = (workspace / "compose.env").read_bytes()
            exposed = expose_root_environment(root, "local")
            self.assertEqual(exposed.read_bytes(), original)
            self.assertEqual({path.name: path.read_bytes() for path in (workspace / "secrets").iterdir()}, before)
            with self.assertRaises(FileExistsError):
                expose_root_environment(root, "local")
            self.assertEqual(exposed.read_bytes(), original)

    def test_private_distinct_purpose_files_and_stable_restart_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = provision(Path(directory), "fixture", 18091, "linux/arm64", "a" * 40)
            root = Path(receipt["workspace"])
            manifest = json.loads((root / "secrets/gateway-credentials.json").read_text())
            self.assertEqual(manifest["version"], 1)
            self.assertEqual([entry["purpose"] for entry in manifest["credentials"]], ["native", "web_relay", "internal_service", "operator"])
            self.assertTrue(all("token" not in entry and entry["token_file"].startswith("/run/secrets/") for entry in manifest["credentials"]))
            web = manifest["credentials"][1]
            self.assertFalse(set(web["permissions"]) & {"register_workstation", "maintain_backend", "manage_configuration", "rebind_workstation"})
            operator = next(entry for entry in manifest["credentials"] if entry["purpose"] == "operator")
            self.assertEqual(operator["id"], "operator-review")
            self.assertEqual(operator["token_file"], "/run/secrets/gateway_operator_token")
            self.assertEqual(set(operator["permissions"]), set(NATIVE_PERMISSIONS))
            self.assertTrue({"read_backend", "register_workstation", "manage_configuration"}.issubset(operator["permissions"]))
            self.assertNotIn("resolve_web_session", operator["permissions"])
            tokens = {(root / "secrets" / name).read_text().strip() for name in TOKEN_NAMES}
            self.assertEqual(len(tokens), len(TOKEN_NAMES))
            env = receipt["environment"]
            self.assertEqual(env["FLEETWORKS_ACCESS_SCOPE_ID"], manifest["scope_id"])
            self.assertEqual(env["FLEETWORKS_ACCESS_EPOCH"], manifest["access_epoch"])
            self.assertTrue(env["FLEETWORKS_SEARCH_SOURCE_EPOCH"])
            self.assertTrue(all(token not in (root / "review.json").read_text() for token in tokens))
            if os.name == "posix":
                for path in [root, root / "secrets", root / "deployment", root / "storage/search-index"]:
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
                for path in (root / "secrets").iterdir():
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            with self.assertRaises(FileExistsError):
                provision(Path(directory), "fixture", 18091, "linux/arm64", "b" * 40)
            self.assertEqual(json.loads((root / "review.json").read_text()), receipt)
            self.assertEqual({(root / "secrets" / name).read_text().strip() for name in TOKEN_NAMES}, tokens)

    def test_operator_secret_is_mounted_only_into_gateway(self):
        template = (Path(__file__).parent / "compose.native-review.yml").read_text()
        inherited = template.split("x-runtime: &runtime\n", 1)[1].split("\nx-health:", 1)[0]
        inherited_secrets = re.search(r"(?m)^  secrets: \[([^\]]+)\]$", inherited).group(1).split(", ")
        services = template.split("\nservices:\n", 1)[1].split("\nsecrets:\n", 1)[0]
        blocks = re.split(r"(?m)^  ([a-z][a-z0-9-]*):\n", services)
        recipients = []
        for name, body in zip(blocks[1::2], blocks[2::2]):
            own = re.search(r"(?m)^    secrets: \[([^\]]+)\]$", body)
            secrets_for_service = own.group(1).split(", ") if own else inherited_secrets
            if "gateway_operator_token" in secrets_for_service:
                recipients.append(name)
        self.assertEqual(recipients, ["api"])
        self.assertIn("  gateway_operator_token:\n    file: ${FLEETWORKS_HOST_WORKSPACE}/secrets/operator-token\n", template)

    def test_init_prints_pairing_approval_command_without_secret_bytes(self):
        with tempfile.TemporaryDirectory(prefix="native review ") as directory:
            root = Path(directory)
            output = io.StringIO()
            with mock.patch.object(native_review, "ROOT", root), \
                 mock.patch("sys.argv", ["native_review.py", "init", "--name", "pairing", "--api-port", "18091", "--platform", "linux/amd64"]), \
                 mock.patch.object(native_review.subprocess, "check_output", return_value="a" * 40), \
                 redirect_stdout(output):
                self.assertEqual(native_review.main(), 0)
            text = output.getvalue()
            command = next(line for line in text.splitlines() if line.startswith("python3 "))
            self.assertEqual(shlex.split(command), [
                "python3", "scripts/fleetworks_pairing.py", "--origin", "http://127.0.0.1:18091",
                "--operator-token-file", str(review_path(root, "pairing") / "secrets/operator-token"),
                "approve", "--pairing-id", "<pairing-id>", "--approval-code", "<approval-code>",
            ])
            self.assertIn("Setup Gateway", text)
            self.assertNotIn("--provision-gateway-credential", text)
            for name in TOKEN_NAMES:
                self.assertNotIn((review_path(root, "pairing") / "secrets" / name).read_text().strip(), text)

    def test_rejects_escaping_symlink_and_invalid_platform_without_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("../other", "/absolute", "Nested/path", ""):
                with self.assertRaises(ValueError):
                    review_path(root, name)
            with self.assertRaises(ValueError):
                provision(root, "fixture", 8080, "darwin/arm64", "a" * 40)
            (root / ".agent-scratch").symlink_to(root, target_is_directory=True)
            with self.assertRaises(ValueError):
                provision(root, "fixture", 18091, "linux/amd64", "a" * 40)


    def test_optional_web_is_explicit_and_keeps_image_overrides_in_restart_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            images = {"FLEETWORKS_RUNTIME_IMAGE": "fleetworks-candidate-runtime:" + "a" * 40 + "-amd64"}
            receipt = provision(Path(directory), "web", 18091, "linux/amd64", "a" * 40,
                                web_port=18092, images=images)
            environment = receipt["environment"]
            self.assertEqual(environment["FLEETWORKS_WEB_ORIGIN"], "http://127.0.0.1:18092")
            self.assertEqual(environment["FLEETWORKS_WEB_PORT"], "18092")
            self.assertEqual(environment["FLEETWORKS_RUNTIME_IMAGE"], images["FLEETWORKS_RUNTIME_IMAGE"])
            self.assertIn("native-review-web-", environment["FLEETWORKS_WEB_IMAGE"])
            stored = json.loads((Path(receipt["workspace"]) / "review.json").read_text())
            self.assertEqual(stored, receipt)
            self.assertTrue((Path(receipt["workspace"]) / "secrets/internal-token").is_file())

    def test_invalid_web_or_image_settings_cannot_create_a_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for options in ({"web_port": 18091}, {"web_port": 80},
                            {"images": {"FLEETWORKS_ACCESS_EPOCH": "forged"}},
                            {"images": {"FLEETWORKS_RUNTIME_IMAGE": "unversioned"}}):
                with self.subTest(options=options), self.assertRaises(ValueError):
                    provision(root, "fixture", 18091, "linux/amd64", "a" * 40, **options)
                self.assertFalse(review_path(root, "fixture").exists())


class PrepareReviewTests(unittest.TestCase):
    def test_native_only_recipe_omits_optional_web_url(self):
        root = Path(__file__).parent
        self.assertNotIn("FLEETWORKS_INTEL_WEB_URL:", (root / "compose.native-review.yml").read_text())
        self.assertIn("FLEETWORKS_INTEL_WEB_URL:", (root / "compose.yml").read_text())

    def fixture(self, root, *, legacy=False, root_env=True):
        receipt = provision(root, "local", 18091, "linux/amd64", "a" * 40, root_env=root_env)
        (root / "version.txt").write_text("0.2.1\n")
        workspace = Path(receipt["workspace"])
        if legacy:
            manifest = workspace / "secrets/gateway-credentials.json"
            data = json.loads(manifest.read_text())
            data["credentials"] = [c for c in data["credentials"] if c["purpose"] != "operator"]
            manifest.write_text(json.dumps(data))
            (workspace / "secrets/operator-token").unlink()
        return workspace

    def test_prepare_upgrades_legacy_once_without_rotating_any_existing_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            manifest = workspace / "secrets/gateway-credentials.json"
            previous = json.loads(manifest.read_text())
            files = {p.name: p.read_bytes() for p in (workspace / "secrets").iterdir()}
            with mock.patch.object(native_review.subprocess, "run") as docker:
                result = native_review.prepare(root, "local")
                docker.assert_not_called()
            current = json.loads(manifest.read_text())
            self.assertEqual(current["credentials"][:-1], previous["credentials"])
            self.assertEqual(current["scope_id"], previous["scope_id"])
            self.assertEqual(current["access_epoch"], previous["access_epoch"])
            for name, data in files.items():
                if name != manifest.name:
                    self.assertEqual((workspace / "secrets" / name).read_bytes(), data)
            token = result["operator"]
            self.assertEqual(stat.S_IMODE(token.stat().st_mode), 0o600)
            self.assertNotIn(token.read_bytes(), files.values())
            identity = (manifest.stat().st_ino, token.stat().st_ino, token.read_bytes())
            native_review.prepare(root, "local")
            self.assertEqual((manifest.stat().st_ino, token.stat().st_ino, token.read_bytes()), identity)

    def test_prepare_reads_live_root_settings_and_prints_self_contained_redacted_commands(self):
        with tempfile.TemporaryDirectory(prefix="review commands ") as directory:
            root = Path(directory)
            workspace = self.fixture(root)
            env_file = root / "compose.env"
            env_file.write_text(env_file.read_text().replace("'18091'", "'18093'")
                                .replace("fleetworks-runtime:native-review-local-" + "a" * 12,
                                         "custom-runtime:edited"))
            output = io.StringIO()
            with mock.patch.object(native_review, "ROOT", root), \
                 mock.patch("sys.argv", ["native_review.py", "prepare", "--name", "local"]), \
                 mock.patch.object(native_review.subprocess, "run") as docker, redirect_stdout(output):
                self.assertEqual(native_review.main(), 0)
                docker.assert_not_called()
            text = output.getvalue()
            build = shlex.split(next(line for line in text.splitlines() if "buildx" in line))
            self.assertIn("runtime.tags=custom-runtime:edited", build)
            self.assertIn("PLATFORM=linux/amd64", build)
            self.assertIn("VERSION=0.2.1", build)
            self.assertIn("--progress=plain", build)
            self.assertIn("http://127.0.0.1:18093", text)
            self.assertNotIn("export ", text)
            restart = shlex.split(next(line for line in text.splitlines() if " && " in line))
            self.assertIn(str(workspace / "compose.env"), restart)
            self.assertIn("fleetworks-native-review-local", restart)
            self.assertIn("--force-recreate", restart)
            for key in ("FLEETWORKS_ACCESS_SCOPE_ID", "FLEETWORKS_ACCESS_EPOCH", "FLEETWORKS_API_PORT"):
                self.assertIn(["-u", key], [restart[i:i + 2] for i in range(len(restart) - 1)])
            manifest = json.loads((workspace / "secrets/gateway-credentials.json").read_text())
            for value in (manifest["scope_id"], manifest["access_epoch"]):
                self.assertNotIn(value, text)
            for name in TOKEN_NAMES:
                self.assertNotIn((workspace / "secrets" / name).read_text().strip(), text)

    def test_existing_operator_with_missing_file_is_not_silently_rotated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root)
            token = workspace / "secrets/operator-token"
            manifest = workspace / "secrets/gateway-credentials.json"
            previous = manifest.read_bytes()
            token.unlink()
            with self.assertRaises(FileNotFoundError):
                native_review.prepare(root, "local")
            self.assertFalse(token.exists())
            self.assertEqual(manifest.read_bytes(), previous)

    def test_unsafe_operator_files_are_rejected_without_rewriting_manifest(self):
        for kind in ("public", "symlink", "hardlink", "fifo"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                workspace = self.fixture(root)
                token = workspace / "secrets/operator-token"
                manifest = workspace / "secrets/gateway-credentials.json"
                previous = manifest.read_bytes()
                if kind == "public":
                    token.chmod(0o644)
                elif kind == "fifo":
                    token.unlink()
                    os.mkfifo(token, 0o600)
                else:
                    token.unlink()
                    target = root / "outside-token"
                    target.write_text("x" * 64)
                    target.chmod(0o600)
                    if kind == "symlink":
                        token.symlink_to(target)
                    else:
                        os.link(target, token)
                with self.assertRaises((ValueError, OSError)):
                    native_review.prepare(root, "local")
                self.assertEqual(manifest.read_bytes(), previous)

    def test_orphan_operator_cannot_reuse_native_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            manifest = workspace / "secrets/gateway-credentials.json"
            before = manifest.read_bytes()
            native_review.private_file(workspace / "secrets/operator-token",
                                       (workspace / "secrets/native-token").read_text())
            with self.assertRaises(ValueError):
                native_review.prepare(root, "local")
            self.assertEqual(manifest.read_bytes(), before)

    def test_epoch_drift_rejected_before_operator_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            manifest = workspace / "secrets/gateway-credentials.json"
            before = manifest.read_bytes()
            env_file = root / "compose.env"
            data = json.loads(before)
            env_file.write_text(env_file.read_text().replace(data["access_epoch"], "different-epoch"))
            with self.assertRaises(ValueError):
                native_review.prepare(root, "local")
            self.assertFalse((workspace / "secrets/operator-token").exists())
            self.assertEqual(manifest.read_bytes(), before)

    def test_unsupported_environment_alias_cannot_touch_another_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            alias = workspace / "compose.env"
            outside = root / "outside.env"
            outside.write_bytes((root / "compose.env").read_bytes())
            outside.chmod(0o600)
            alias.unlink()
            alias.symlink_to(outside)
            with self.assertRaises(ValueError):
                native_review.prepare(root, "local")
            self.assertFalse((workspace / "secrets/operator-token").exists())

    def test_interrupted_manifest_update_reuses_private_orphan_on_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            manifest = workspace / "secrets/gateway-credentials.json"
            previous = manifest.read_bytes()
            with mock.patch.object(native_review.os, "replace", side_effect=OSError("injected replacement failure")):
                with self.assertRaises(OSError):
                    native_review.prepare(root, "local")
            token = workspace / "secrets/operator-token"
            original_token = token.read_bytes()
            self.assertEqual(manifest.read_bytes(), previous)
            self.assertEqual(list((workspace / "secrets").glob(".gateway-*.tmp")), [])
            native_review.prepare(root, "local")
            self.assertEqual(token.read_bytes(), original_token)
            self.assertEqual(json.loads(manifest.read_text())["credentials"][-1]["purpose"], "operator")


    def test_up_builds_before_restart_with_streaming_output_and_exact_review_scope(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = self.fixture(root, legacy=True)
            output = io.StringIO()
            with mock.patch.object(native_review, "ROOT", root), \
                 mock.patch.object(native_review.sys, "platform", "darwin"), \
                 mock.patch("sys.argv", ["native_review.py", "up", "--name", "local"]), \
                 mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                 mock.patch.object(native_review.subprocess, "check_output", side_effect=[
                     "desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/x86_64\n"]), \
                 mock.patch.object(native_review.subprocess, "run") as docker, redirect_stdout(output):
                self.assertEqual(native_review.main(), 0)
            commands = [call.args[0] for call in docker.call_args_list]
            self.assertEqual(len(commands), 6)
            self.assertEqual(commands[0][-2:], ["config", "--quiet"])
            self.assertIn("bake", commands[1])
            self.assertIn(["--builder", "desktop-linux"],
                          [commands[1][i:i+2] for i in range(len(commands[1])-1)])
            self.assertEqual(commands[2][-1], "stop")
            self.assertEqual(commands[3][-7:], ["up", "--no-build", "--pull", "never", "--force-recreate", "--detach", "--wait"])
            self.assertEqual(commands[4][-1], "ps")
            self.assertEqual(commands[5], [native_review.sys.executable,
                str(root.resolve() / "scripts/fleetworks_candidate.py"),
                "build-native", "--target", "host", "--profile", "debug", "--open"])
            for command in (commands[0], *commands[2:5]):
                self.assertIn(str(workspace / "compose.env"), command)
                self.assertIn("fleetworks-native-review-local", command)
                self.assertIn(["--context", "desktop-linux"],
                              [command[i:i+2] for i in range(len(command)-1)])
            for call in docker.call_args_list:
                self.assertEqual(call.kwargs, {"cwd": root.resolve(), "check": True})
            self.assertIn("[2/6]", output.getvalue())
            self.assertIn("[6/6]", output.getvalue())
            self.assertIn("http://127.0.0.1:18091", output.getvalue())
            self.assertNotIn("No images built", output.getvalue())
            for name in TOKEN_NAMES:
                secret = (workspace / "secrets" / name).read_text().strip()
                self.assertNotIn(secret, output.getvalue())
                self.assertNotIn(secret, repr(commands))

    def test_up_stops_after_each_failed_stage_and_retains_data(self):
        for index, label in enumerate(("config", "build", "stop", "startup", "status", "native")):
            with self.subTest(stage=label), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                workspace = self.fixture(root)
                sentinel = workspace / "storage" / "keep-data"
                sentinel.write_text("retained")
                credentials = {p.name: p.read_bytes() for p in (workspace / "secrets").iterdir()}
                calls = []
                def execute(command, **kwargs):
                    calls.append(command)
                    if len(calls) == index + 1:
                        raise native_review.subprocess.CalledProcessError(17, command)
                error = io.StringIO()
                with mock.patch.object(native_review, "ROOT", root), \
                     mock.patch.object(native_review.sys, "platform", "darwin"), \
                     mock.patch("sys.argv", ["native_review.py", "up", "--name", "local"]), \
                     mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                     mock.patch.object(native_review.subprocess, "check_output", side_effect=[
                         "desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/x86_64\n"]), \
                     mock.patch.object(native_review.subprocess, "run", side_effect=execute), \
                     redirect_stdout(io.StringIO()), redirect_stderr(error), \
                     self.assertRaises(SystemExit) as stopped:
                    native_review.main()
                self.assertEqual(stopped.exception.code, 17)
                self.assertEqual(len(calls), index + 1)
                self.assertIn("No later stage ran", error.getvalue())
                if label == "native":
                    self.assertIn("Backend services remain running", error.getvalue())
                self.assertEqual(sentinel.read_text(), "retained")
                self.assertEqual({p.name: p.read_bytes() for p in (workspace / "secrets").iterdir()}, credentials)

    def test_up_rejects_remote_or_mismatched_daemon_before_any_build_or_stop(self):
        for responses in (["remote\n", '"ssh://example.test"\n'],
                          ["desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/aarch64\n"]):
            with self.subTest(responses=responses), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                with mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                     mock.patch.object(native_review.subprocess, "check_output", side_effect=responses), \
                     mock.patch.object(native_review.subprocess, "run") as docker, \
                     redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                    native_review.run_review(root, "local")
                docker.assert_not_called()

    def test_up_refuses_changed_settings_before_stopping_running_services(self):
        for key in ("FLEETWORKS_API_PORT", "FLEETWORKS_UID", "FLEETWORKS_GID", "FLEETWORKS_SEARCH_SOURCE_EPOCH"):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory:
                self.check_changed_setting(Path(directory), key)

    def check_changed_setting(self, root, key):
        with self.subTest(setting=key):
            self.fixture(root)
            calls = []
            def execute(command, **kwargs):
                calls.append(command)
                if "bake" in command:
                    env = root / "compose.env"
                    env.write_text(re.sub(r"(?m)^" + key + r"=.*$", key + "='changed'", env.read_text()))
            with mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                 mock.patch.object(native_review.subprocess, "check_output", side_effect=[
                     "desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/x86_64\n"]), \
                 mock.patch.object(native_review.subprocess, "run", side_effect=execute), \
                 redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                native_review.run_review(root, "local")
            self.assertEqual(len(calls), 2)
            self.assertFalse(any(command[-1] == "stop" for command in calls))


    def test_up_backend_only_and_non_macos_do_not_build_or_open_native(self):
        for platform, flags in (("darwin", ["--backend-only"]), ("linux", [])):
            with self.subTest(platform=platform, flags=flags), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                output = io.StringIO()
                with mock.patch.object(native_review, "ROOT", root), \
                     mock.patch.object(native_review.sys, "platform", platform), \
                     mock.patch("sys.argv", ["native_review.py", "up", "--name", "local"] + flags), \
                     mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                     mock.patch.object(native_review.subprocess, "check_output", side_effect=[
                         "desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/x86_64\n"]), \
                     mock.patch.object(native_review.subprocess, "run") as processes, redirect_stdout(output):
                    self.assertEqual(native_review.main(), 0)
                self.assertEqual(processes.call_count, 5)
                self.assertEqual(processes.call_args.args[0][-1], "ps")
                self.assertFalse(any("build-native" in call.args[0] for call in processes.call_args_list))
                self.assertIn("http://127.0.0.1:18091", output.getvalue())
                if platform == "linux":
                    self.assertIn("requires macOS; skipped", output.getvalue())

    def test_up_cancelled_build_never_stops_running_services(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            calls = []
            def execute(command, **kwargs):
                calls.append(command)
                if "bake" in command:
                    raise KeyboardInterrupt
            with mock.patch.object(native_review, "ROOT", root), \
                 mock.patch("sys.argv", ["native_review.py", "up", "--name", "local"]), \
                 mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "desktop-linux"}), \
                 mock.patch.object(native_review.subprocess, "check_output", side_effect=[
                     "desktop-linux\n", '"unix:///var/run/docker.sock"\n', "linux/x86_64\n"]), \
                 mock.patch.object(native_review.subprocess, "run", side_effect=execute), \
                 redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as stopped:
                native_review.main()
            self.assertEqual(stopped.exception.code, 130)
            self.assertEqual(len(calls), 2)
            self.assertFalse(any(command[-1] == "stop" for command in calls))


if __name__ == "__main__":
    unittest.main()
