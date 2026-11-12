"""Offline contracts for the deployment smoke's current-proof handling."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import verify


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

    def test_fixture_is_a_real_contained_child_with_independent_markers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registration = verify.native_fixture(root)
            child = root / "synthetic-native-home/.fleetworks" / registration["workspace_name"]
            self.assertTrue(child.is_dir())
            self.assertEqual(child.resolve().parent, (root / "synthetic-native-home/.fleetworks").resolve())
            self.assertNotEqual(registration["installation_marker"], registration["workspace_marker"])
            self.assertFalse(set(registration) & {"owner", "origin", "scope_id", "deployment_id"})

    def test_cookie_login_keeps_proof_out_of_body_and_uses_confirmed_revision(self):
        context = {"connect_id":"192.0.2.61_fixture","registration_id":"registration","context_revision":4}
        cookie = "fleetworks_connect=" + "a" * 64 + "; Path=/; HttpOnly; SameSite=Strict"
        with patch.object(verify,"exchange",side_effect=[(307,None,{}),(200,{"context":context},{"Set-Cookie":cookie})]), patch.object(verify,"request",side_effect=[(401,{}),(200,{})]) as request:
            headers = verify.web_connect(18091,context)
        self.assertEqual(headers["Cookie"],"fleetworks_connect="+"a"*64)
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
