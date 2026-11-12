"""Offline contracts for the deployment smoke's current-proof handling."""

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import verify


class RegisteredSmokeTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
