"""Offline contracts for the deployment smoke's current-proof handling."""

import json
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import verify


def registration_snapshot():
    receipt = {"deployment_id": "deployment-fixture", "workstation_id": "workstation-fixture",
               "registration_id": "registration-fixture", "connect_id": "workstation-fixture",
               "workstation_ip": "192.0.2.61", "status": "active", "revision": 2}
    context = {key: receipt[key] for key in ("deployment_id", "workstation_id", "registration_id", "connect_id", "workstation_ip")}
    context.update(registration_revision=2, context_revision=3, scope_revision=4,
                   configuration_epoch="configuration-fixture", access_epoch="access-fixture")
    return receipt, context


class RegisteredSmokeTests(unittest.TestCase):
    def test_fresh_explicit_zero_revision_is_valid_and_missing_or_forged_revision_is_not(self):
        proof = verify.ProofHeaders({}, {"registration_id": "registration", "registration_revision": 2, "context_revision": 3})
        with patch.object(verify, "request", return_value=(200, {"configuration_revision": 0})):
            context = verify.submission_context(18091, None, proof)
        self.assertEqual(context["expected_configuration_revision"], 0)
        for value in ({}, {"configuration_revision": None}, {"configuration_revision": False}, {"configuration_revision": -1}, {"configuration_revision": "0"}):
            with self.subTest(value=value), patch.object(verify, "request", return_value=(200, value)):
                with self.assertRaisesRegex(AssertionError, "revision unavailable"):
                    verify.submission_context(18091, None, proof)

    def test_fixture_has_only_a_protected_contained_installation_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registration = verify.native_fixture(root)
            container = root / "synthetic-native-home/.fleetworks"
            marker = container / "fixture-installation.json"
            self.assertEqual(container.resolve().parent, (root / "synthetic-native-home").resolve())
            self.assertEqual(set(registration), {"request_id", "workstation_ip", "installation_marker"})
            self.assertRegex(registration["installation_marker"], r"^[0-9a-f]{64}$")
            self.assertEqual(list(container.iterdir()), [marker])
            self.assertTrue(stat.S_ISREG(marker.lstat().st_mode))
            self.assertEqual(stat.S_IMODE(marker.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(container.stat().st_mode), 0o700)
            self.assertEqual(marker.stat().st_nlink, 1)
            self.assertEqual(json.loads(marker.read_text()), {"installation_marker": registration["installation_marker"]})
            identity = marker.stat().st_ino
            with self.assertRaises(FileExistsError):
                verify.native_fixture(root)
            self.assertEqual(marker.stat().st_ino, identity)
            self.assertEqual(json.loads(marker.read_text())["installation_marker"], registration["installation_marker"])

    def test_fixture_isolates_installations_and_never_follows_an_existing_home(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second, outside = (root / name for name in ("first", "second", "outside"))
            for path in (first, second, outside):
                path.mkdir()
            a = verify.native_fixture(first)
            b = verify.native_fixture(second)
            self.assertNotEqual(a["installation_marker"], b["installation_marker"])
            linked = root / "linked"
            linked.mkdir()
            (linked / "synthetic-native-home").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(FileExistsError):
                verify.native_fixture(linked)
            self.assertEqual(list(outside.iterdir()), [])

    def test_registration_replays_exact_receipt_and_resolves_its_v2_context(self):
        registration = {"request_id": "register-fixture", "installation_marker": "a" * 64}
        receipt, context = registration_snapshot()
        for status in (200, 201):
            with self.subTest(status=status), patch.object(verify, "request", side_effect=[
                (status, receipt), (200, receipt), (200, {"context": context}),
            ]) as request:
                self.assertEqual(verify.register_native(18091, "synthetic-native", registration), context)
                self.assertEqual(request.call_args_list[0].args[3], registration)
                self.assertEqual(request.call_args_list[1].args[3], registration)
                self.assertEqual(request.call_args_list[2].args[3], {
                    "registration_id": receipt["registration_id"], "expected_revision": receipt["revision"],
                })

    def test_registration_rejects_legacy_or_inconsistent_receipts_and_contexts(self):
        receipt, context = registration_snapshot()
        cases = [
            ({**receipt, "workspace_name": "legacy"}, context),
            ({**receipt, "connect_id": "legacy-ip_folder"}, context),
            ({**receipt, "revision": False}, context),
            ({**receipt, "revision": 0}, context),
            ({**receipt, "status": "retired"}, context),
            (receipt, {**context, "workspace_id": "legacy"}),
            (receipt, {**context, "workstation_id": "another-workstation"}),
            (receipt, {**context, "deployment_id": "another-deployment"}),
            (receipt, {**context, "registration_revision": 3}),
            (receipt, {**context, "registration_revision": True}),
            (receipt, {**context, "context_revision": 0}),
            (receipt, {**context, "scope_revision": False}),
            (receipt, {**context, "access_epoch": ""}),
            (receipt, {**context, "workstation_ip": "192.0.2.62"}),
        ]
        for index, (returned, resolved) in enumerate(cases):
            with self.subTest(case=index), patch.object(verify, "request", side_effect=[
                (201, returned), (200, returned), (200, {"context": resolved}),
            ]):
                with self.assertRaisesRegex(AssertionError, "inconsistent|differs"):
                    verify.register_native(18091, "synthetic-native", {})
        with patch.object(verify, "request", side_effect=[(201, receipt), (200, {**receipt, "revision": 3})]):
            with self.assertRaisesRegex(AssertionError, "replay changed identity"):
                verify.register_native(18091, "synthetic-native", {})

    def test_unexpected_status_reports_only_bounded_status_evidence(self):
        secret = "synthetic-token-must-not-be-logged"
        with patch.object(verify, "request", return_value=(422, {"error": secret, "body": "x" * 10000})):
            with self.assertRaises(AssertionError) as error:
                verify.register_native(18091, secret, {"installation_marker": secret})
        self.assertEqual(str(error.exception), "native ensure: expected HTTP 200/201, received 422")
        self.assertNotIn(secret, str(error.exception))

    def test_purpose_denials_use_valid_v2_inputs_and_current_missing_context_status(self):
        registration = {"request_id": "register-fixture", "installation_marker": "a" * 64}
        with patch.object(verify, "request", side_effect=[(401, {}), (403, {}), (403, {}), (401, {})]) as request:
            verify.check_purpose_isolation(18091, "native", "relay", "health", registration)
        self.assertEqual(request.call_args_list[1].args[3], registration)
        self.assertEqual(request.call_args_list[2].args[3], {
            "request_id": "denied-health", "tool_id": "dork_generate", "inputs": {"objective": "Synthetic fixture"},
        })
        self.assertEqual(request.call_args_list[3].args, (18091, "/api/v1/jobs", "native"))

    def test_activity_keeps_the_legacy_workspace_query_rejection(self):
        _, context = registration_snapshot()
        page = {"rows": [{"id": "job-fixture"}], "activity_scope": {"workstation_id": context["workstation_id"]}}
        detail = {"private_details_available": True, "job": {"id": "job-fixture"}}
        replies = [(200, page), (200, {"rows": []}), (200, page), (200, detail), (404, {}), (404, {}), (400, {})]
        with patch.object(verify, "request", side_effect=replies) as request:
            verify.exercise_activity(18091, 18092, "native", context, {}, "job-fixture")
        self.assertIn("workspace_id=", request.call_args.args[1])

    def test_cookie_login_keeps_proof_out_of_body_and_uses_confirmed_revision(self):
        context = {"connect_id":"workstation-fixture","registration_id":"registration","context_revision":4}
        cookie = "fleetworks_connect_v2=" + "a" * 64 + "; Path=/; HttpOnly; SameSite=Strict"
        with patch.object(verify,"exchange",side_effect=[(307,None,{}),(200,{"context":context},{"Set-Cookie":cookie})]), patch.object(verify,"request",side_effect=[(401,{}),(200,{})]) as request:
            headers = verify.web_connect(18091,context)
        self.assertEqual(headers["Cookie"],"fleetworks_connect_v2="+"a"*64)
        self.assertEqual(headers["x-fleetworks-expected-context-revision"],"4")
        self.assertEqual(request.call_args.kwargs["headers"],headers)
        with patch.object(verify,"exchange",side_effect=[(307,None,{}),(200,{"context":context},{"Set-Cookie":cookie.replace("HttpOnly; ","")})]), patch.object(verify,"request",return_value=(401,{})):
            with self.assertRaisesRegex(AssertionError,"protected cookie"):
                verify.web_connect(18091,context)

    def test_tools_wait_for_durable_queued_work_and_keep_registration_proof(self):
        proof=verify.ProofHeaders({"x-fleetworks-registration":"registration","x-fleetworks-registration-revision":"2"},
            {"registration_id":"registration","registration_revision":2,"context_revision":3})
        receipt={"call_id":"fleet-tool-"+"b"*64,"job_id":"fleet-tool-"+"b"*64}
        complete={**receipt,"status":"completed","result":{"observations":["synthetic"],"credits_charged":0}}
        replies=[(200,{"configuration_revision":7}),(202,receipt),(202,receipt),(200,{"status":"queued"}),(200,complete)]
        with patch.object(verify,"request",side_effect=replies) as request, patch.object(verify,"wait_job") as waited, patch.object(verify.time,"sleep"):
            result,value=verify.exercise_tools(18091,None,proof,"fixture-request")
        self.assertEqual(value,complete)
        self.assertTrue(result.endswith(receipt["call_id"]))
        self.assertTrue(all(call.kwargs["headers"]==proof for call in request.call_args_list))
        self.assertEqual(request.call_args_list[1].args[3]["context"]["expected_configuration_revision"],7)
        waited.assert_called_once_with(18091,None,proof,receipt["job_id"],15)


class StandaloneDeliveryTests(unittest.TestCase):
    def test_overlays_are_explicit_and_share_the_private_review_env(self):
        root = Path("/synthetic/review")
        base = verify.compose_command(root)
        self.assertEqual(base[:4], ["docker", "compose", "--env-file", str(root / "compose.env")])
        self.assertEqual(base[-1], str(verify.native_review.ROOT / "deploy/compose.native-review.yml"))
        selected = verify.compose_command(root, web=True, dev=True, atlas=True)
        self.assertEqual([Path(selected[index + 1]).name for index, item in enumerate(selected) if item == "-f"],
                         ["compose.native-review.yml", "compose.yml", "compose.dev.yml", "compose.atlas.yml"])
        with self.assertRaisesRegex(ValueError, "requires --web"):
            verify.compose_command(root, dev=True)

    def test_inherited_authority_and_provider_secrets_never_configure_the_fixture(self):
        original = {"PATH": "/usr/bin", "DOCKER_HOST": "unix:///synthetic/socket",
                    "COMPOSE_PROJECT_NAME": "user-stack", "FLEETWORKS_HOST_WORKSPACE": "/real/home",
                    "ARGOS_HOME": "/real/home", "SURREAL_PASS": "private", "NEWS_API_KEY": "private"}
        self.assertEqual(verify.isolated_environment(original),
                         {"PATH": "/usr/bin", "DOCKER_HOST": "unix:///synthetic/socket"})
        self.assertEqual(original["COMPOSE_PROJECT_NAME"], "user-stack")

    def test_image_identity_checks_only_containers_selected_by_the_owned_project(self):
        command = verify.compose_command(Path("/synthetic/review"))
        expected = {"api": "sha256:" + "a" * 64, "bootstrap": "sha256:" + "a" * 64}
        output = "\n".join(f"{service} {identity}" for service, identity in expected.items())
        with patch.object(verify.subprocess, "check_output", side_effect=["c1\nc2\n", output]) as docker:
            verify.check_container_images(command, {}, expected)
        self.assertEqual(docker.call_args_list[0].args[0], [*command, "ps", "--all", "--quiet"])
        self.assertEqual(docker.call_args_list[1].args[0][-2:], ["c1", "c2"])
        with patch.object(verify.subprocess, "check_output", side_effect=["c1\nc2\n", output]):
            with self.assertRaisesRegex(RuntimeError, "downloaded image IDs"):
                verify.check_container_images(command, {}, {**expected, "other": "sha256:" + "b" * 64})

    def test_existing_project_is_rejected_without_build_start_or_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(verify.native_review, "ROOT", root), \
                 patch.object(verify, "fixture_ports", return_value=[18091, 18092]), \
                 patch.object(verify.subprocess, "check_output", side_effect=["a" * 40, "linux/amd64\n", "foreign-container\n"]) as read, \
                 patch.object(verify.subprocess, "run") as mutate, \
                 patch.object(verify, "native_fixture") as fixture:
                with self.assertRaisesRegex(ValueError, "project already has containers"):
                    verify.verify("collision", "linux/amd64")
            mutate.assert_not_called()
            fixture.assert_not_called()
            self.assertEqual(read.call_args.args[0], [
                "docker", "ps", "--all", "--filter",
                "label=com.docker.compose.project=fleetworks-native-review-collision", "--quiet",
            ])
            self.assertEqual(read.call_args.kwargs["env"]["COMPOSE_PROJECT_NAME"],
                             "fleetworks-native-review-collision")
            # The fresh local intent is retained; an existing project is never owned.
            self.assertTrue((verify.native_review.review_path(root, "collision") / "review.json").is_file())

    def test_nonmatching_daemon_is_rejected_without_build_start_or_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(verify.native_review, "ROOT", Path(temporary)), \
                 patch.object(verify, "fixture_ports", return_value=[18091, 18092]), \
                 patch.object(verify.subprocess, "check_output", side_effect=["a" * 40, "linux/arm64\n"]) as read, \
                 patch.object(verify.subprocess, "run") as mutate, \
                 patch.object(verify, "native_fixture") as fixture:
                with self.assertRaisesRegex(ValueError, "daemon does not match"):
                    verify.verify("wrong-daemon", "linux/amd64")
            mutate.assert_not_called()
            fixture.assert_not_called()
            self.assertEqual(read.call_count, 2)
            self.assertEqual(read.call_args.args[0], [
                "docker", "version", "--format", "{{.Server.Os}}/{{.Server.Arch}}",
            ])

    def test_start_failure_stops_only_the_fresh_project_and_preserves_owned_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            marker = workspace / "preserve"
            marker.write_text("fixture evidence")
            receipt = {"workspace": str(workspace), "image_tag": "synthetic", "environment": {
                "COMPOSE_PROJECT_NAME": "fleetworks-native-review-fixture", "PLATFORM": "linux/amd64"}}
            failure = subprocess.CalledProcessError(17, ["docker", "compose", "up"])
            with patch.object(verify, "fixture_ports", return_value=[18091, 18092]), \
                 patch.object(verify.native_review, "provision", return_value=receipt), \
                 patch.object(verify.subprocess, "check_output", side_effect=["a" * 40, "linux/amd64\n", ""]), \
                 patch.object(verify.subprocess, "run", side_effect=[failure, subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 0)]) as run:
                with self.assertRaises(subprocess.CalledProcessError) as error:
                    verify.verify("fixture", "linux/amd64", no_build=True)
            self.assertIs(error.exception, failure)
            self.assertEqual(marker.read_text(), "fixture evidence")
            prefix = verify.compose_command(workspace)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(commands[0][:len(prefix)], prefix)
            self.assertIn("--no-build", commands[0])
            self.assertEqual(commands[0][commands[0].index("--pull") + 1], "never")
            self.assertEqual(commands[-1], [*prefix, "down"])
            self.assertTrue(all(call.kwargs["env"]["COMPOSE_PROJECT_NAME"] == "fleetworks-native-review-fixture"
                                for call in run.call_args_list))
            self.assertFalse(any(item in {"--volumes", "prune", "--rmi"} for command in commands for item in command))


if __name__ == "__main__":
    unittest.main()
