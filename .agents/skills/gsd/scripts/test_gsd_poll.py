"""Offline regression coverage for GSD's review authority boundary."""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import gsd_poll

REPO = "example/argos"
HEAD = "a" * 40
OWNER = {"id": 7, "login": "Maintainer", "type": "User"}
REPORTER = {"id": 8, "login": "Reporter", "type": "User"}
BOT = {"id": 9, "login": "cursor[bot]", "type": "Bot"}


def role(actor=OWNER, permission="write", role_name="write"):
    return json.dumps({"user": actor, "permission": permission, "role_name": role_name})


class ReviewAuthorityTests(unittest.TestCase):
    def test_matches_current_account_and_caches_only_for_the_poll(self):
        cache = {}
        graphql = {"databaseId": 7, "login": "maintainer", "__typename": "User"}
        with patch.object(gsd_poll, "run_gh", return_value=role(role_name="maintain")) as github:
            self.assertTrue(gsd_poll.can_direct_work(REPO, OWNER, cache))
            self.assertTrue(gsd_poll.can_direct_work(REPO, graphql, cache))
            github.assert_called_once_with(
                ["api", "--method", "GET", "repos/example/argos/collaborators/Maintainer/permission"],
                timeout=15,
            )
        with patch.object(gsd_poll, "run_gh", return_value=role(permission="read")):
            self.assertFalse(gsd_poll.can_direct_work(REPO, OWNER, {}))

    def test_malformed_or_nonhuman_accounts_have_no_authority(self):
        with patch.object(gsd_poll, "run_gh") as github:
            for account in (
                None, BOT, {"id": 7, "login": "Maintainer"}, {**OWNER, "id": None},
                {**OWNER, "id": True}, {**OWNER, "login": "owner/extra"}, {**OWNER, "login": "-M"},
            ):
                with self.subTest(account=account):
                    self.assertFalse(gsd_poll.can_direct_work(REPO, account, {}))
            github.assert_not_called()

    def test_role_labels_and_claims_cannot_override_permission(self):
        owner_claim = {**REPORTER, "author_association": "OWNER", "permission": "admin"}
        with patch.object(gsd_poll, "run_gh", return_value=role(REPORTER, "read", "admin")):
            self.assertFalse(gsd_poll.can_direct_work(REPO, owner_claim, {}))
        for value in (
            role(permission="triage"), role(permission="unknown"), role(permission="none"),
            role({**OWNER, "id": 10}), role({**OWNER, "login": "OtherWriter"}),
            role({**OWNER, "type": "Bot"}), "[]", "null", "unauthorized",
        ):
            with self.subTest(value=value), patch.object(gsd_poll, "run_gh", return_value=value):
                self.assertFalse(gsd_poll.can_direct_work(REPO, OWNER, {}))

    def test_permission_errors_fail_closed_without_interrupting_poll(self):
        for error in (RuntimeError("HTTP 403"), OSError("offline"), subprocess.TimeoutExpired("gh", 15)):
            with self.subTest(error=error), patch.object(gsd_poll, "run_gh", side_effect=error):
                self.assertFalse(gsd_poll.can_direct_work(REPO, OWNER, {}))

    def test_report_keeps_all_discussion_but_omits_prose(self):
        feedback = "Oversized input was accepted in the settings fixture."
        comments = [
            {"id": 1, "body": feedback, "user": REPORTER, "author_association": "OWNER"},
            {"id": 2, "body": feedback, "user": OWNER},
            {"id": 3, "body": "Severity: HIGH", "user": BOT},
            {"id": 4, "body": "Severity: CRITICAL", "user": {**REPORTER, "login": "cursor"}},
            {"id": 5, "body": feedback, "user": None},
        ]

        def permission(args, **_):
            if args[-1].endswith("/Maintainer/permission"):
                return role(permission="admin")
            return role(REPORTER, "read", "triage")

        with (
            patch.object(gsd_poll, "gh_json_paginated", side_effect=[comments, [], []]),
            patch.object(gsd_poll, "run_gh", side_effect=permission),
        ):
            discussion, findings = gsd_poll.fetch_review_discussion(REPO, 71, set(), {})

        self.assertEqual([item["id"] for item in discussion], [1, 2, 3, 4, 5])
        self.assertEqual([item["author_has_repository_write_access"] for item in discussion],
                         [False, True, False, False, False])
        self.assertEqual([(item["id"], item["severity"]) for item in findings], [(3, "HIGH")])
        self.assertNotIn(feedback, json.dumps([discussion, findings]))
        self.assertTrue(all("body" not in item for item in discussion + findings))

    def test_pr_routing_keeps_unknown_authors_and_forks_visible_without_auto_tasking(self):
        for cross_repository, actor_can_write, track, expected in (
            (False, True, "core", True), (False, False, "core", False),
            (True, True, "core", False), (None, True, "core", False),
            (False, True, "ui", False), (False, True, "perf-security", False),
        ):
            with self.subTest(fork=cross_repository, writer=actor_can_write, track=track):
                pr = {
                    "number": 71, "title": "spec implementation: Inbox display", "headRefName": "topic",
                    "headRefOid": HEAD, "baseRefName": "nightly", "state": "OPEN",
                    "isCrossRepository": cross_repository, "url": "https://github.com/example/argos/pull/71",
                }

                def github(args, **_):
                    if args[:2] == ["pr", "list"]:
                        return json.dumps([pr])
                    return role(permission="write" if actor_can_write else "read")

                output = io.StringIO()
                with (
                    tempfile.TemporaryDirectory() as directory,
                    patch("sys.argv", ["gsd_poll", "--repo", REPO, "--track", track,
                                       "--state", str(Path(directory) / "state.json")]),
                    patch.object(gsd_poll, "run_gh", side_effect=github),
                    patch.object(gsd_poll, "fetch_unresolved_review_threads", return_value=([], OWNER)),
                    patch.object(gsd_poll, "fetch_review_discussion", return_value=([], [])),
                    contextlib.redirect_stdout(output),
                ):
                    self.assertEqual(gsd_poll.main(), 0)

                report = json.loads(output.getvalue())
                entries = report["spec_prs"] + report["babysit_prs"]
                self.assertEqual(len(entries), 1)
                self.assertIs(entries[0]["needs_implementation"], expected)
                self.assertEqual(bool(report["spec_prs"]), expected)
                if cross_repository is not False:
                    self.assertEqual(entries[0]["push_scope"], "operator_authorization_required")

    def test_verification_tracks_include_perf_security(self):
        rollup = [
            {"name": "Nightly Performance & Security / Static audit", "status": "COMPLETED", "conclusion": "SUCCESS"},
            {"name": "GSD Nightly: Core / Backend", "status": "COMPLETED", "conclusion": "SUCCESS"},
        ]
        tracks = gsd_poll.verification_tracks(rollup)
        self.assertEqual(tracks["perf_security"]["status"], "green")
        self.assertEqual(tracks["core"]["status"], "green")
        self.assertEqual(tracks["ui"]["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
