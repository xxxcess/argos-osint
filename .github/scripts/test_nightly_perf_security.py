"""Offline tests for the Nightly Performance & Security static audit and PR validator."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import nightly_perf_security as audit

HEAD = "a" * 40
BASE = "b" * 40


class NightlyPerfSecurityTests(unittest.TestCase):
    def test_detects_workflow_docker_rust_and_web_regressions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wf = root / ".github/workflows/sample.yml"
            wf.parent.mkdir(parents=True)
            wf.write_text(
                "steps:\n"
                "  - uses: Swatinem/rust-cache@v2\n"
                "  - run: echo \"${{ inputs.expected-head }}\"\n",
                encoding="utf-8",
            )
            df = root / "deploy/Dockerfile.sample"
            df.parent.mkdir(parents=True)
            df.write_text("FROM debian:bookworm-slim AS base\nFROM base\n", encoding="utf-8")

            rs = root / "crates/argos-osint-core/src/sample.rs"
            rs.parent.mkdir(parents=True)
            rs.write_text(
                "pub fn bad() {\n"
                "    std::thread::sleep(std::time::Duration::from_millis(10));\n"
                "    let _q = \"SELECT * FROM atlas_article\";\n"
                "}\n"
                "#[cfg(test)]\n"
                "mod tests { fn ok() { std::thread::sleep(std::time::Duration::from_millis(1)); } }\n",
                encoding="utf-8",
            )

            ts = root / "packages/api-client/src/sample.tsx"
            ts.parent.mkdir(parents=True)
            ts.write_text(
                "export function f() { return useInfiniteQuery({ queryKey: ['a'] }); }\n"
                "export const x = <div dangerouslySetInnerHTML={{ __html: 'x' }} />;\n",
                encoding="utf-8",
            )

            report = audit.audit_files(
                [
                    ".github/workflows/sample.yml",
                    "deploy/Dockerfile.sample",
                    "crates/argos-osint-core/src/sample.rs",
                    "packages/api-client/src/sample.tsx",
                ],
                root,
            )

        rules = {item["rule"] for item in report["findings"]}
        self.assertEqual(
            rules,
            {
                "ci-unpinned-action",
                "ci-expr-injection",
                "docker-unpinned-from",
                "rust-blocking-sleep",
                "surreal-unbounded-select",
                "web-dangerous-sink",
                "query-unbounded-infinite-pages",
            },
        )
        # #[cfg(test)] sleep must not be flagged.
        self.assertEqual(sum(1 for item in report["findings"] if item["rule"] == "rust-blocking-sleep"), 1)
        self.assertEqual(report["status"], "attention_needed")
        self.assertLess(report["static_deltas"]["security"], 0)
        self.assertLess(report["static_deltas"]["performance"], 0)
        self.assertLess(report["static_deltas"]["scalability"], 0)

        md = audit.render_markdown(report, head=HEAD, run_url="https://example.test/runs/1")
        self.assertIn(audit.COMMENT_MARKER, md)
        self.assertIn("gbot", md)
        self.assertIn("ci-unpinned-action", md)

    def test_clean_pinned_files_produce_zero_findings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wf = root / ".github/workflows/clean.yml"
            wf.parent.mkdir(parents=True)
            wf.write_text(
                "steps:\n"
                "  - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7\n"
                "  - env:\n"
                "      HEAD: ${{ inputs.head }}\n"
                "    run: |\n"
                "      echo \"$HEAD\"\n",
                encoding="utf-8",
            )
            df = root / "deploy/Dockerfile"
            df.parent.mkdir(parents=True)
            df.write_text(
                "FROM debian:bookworm-slim@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef AS runtime\n"
                "FROM scratch\n",
                encoding="utf-8",
            )
            report = audit.audit_files([".github/workflows/clean.yml", "deploy/Dockerfile"], root)
        self.assertEqual(report["findings"], [])
        self.assertEqual(report["status"], "clean")
        self.assertEqual(report["static_deltas"], {"performance": 0, "security": 0, "scalability": 0})

    def test_validate_pr_enforces_open_nightly_exact_head_and_same_repo(self):
        pr = {
            "state": "open",
            "title": "feat: sample",
            "html_url": "https://github.com/example/argos/pull/99",
            "head": {"sha": HEAD, "repo": {"full_name": "example/argos"}},
            "base": {"ref": "nightly", "sha": BASE, "repo": {"full_name": "example/argos"}},
        }
        with patch.object(audit, "api_json", return_value=pr), patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/argos"}):
            info = audit.validate_pr("example/argos", "99", HEAD, require_same_repo=True)
            self.assertEqual(info["pr"], 99)
            self.assertEqual(info["base"], BASE)

        fork_pr = {**pr, "head": {"sha": HEAD, "repo": {"full_name": "fork/argos"}}}
        with patch.object(audit, "api_json", return_value=fork_pr), patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/argos"}):
            # Read-only audit allows fork PRs; comment write rejects them.
            audit.validate_pr("example/argos", "99", HEAD, require_same_repo=False)
            with self.assertRaisesRegex(ValueError, "same-repository"):
                audit.validate_pr("example/argos", "99", HEAD, require_same_repo=True)

        main_pr = {**pr, "base": {"ref": "main", "sha": BASE, "repo": {"full_name": "example/argos"}}}
        with patch.object(audit, "api_json", return_value=main_pr), patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/argos"}):
            with self.assertRaisesRegex(ValueError, "nightly"):
                audit.validate_pr("example/argos", "99", HEAD, require_same_repo=False)

    def test_upsert_pr_comment_updates_existing_marker_comment(self):
        existing = [[{"id": 42, "body": f"{audit.COMMENT_MARKER}\nold"}]]
        with (
            patch.object(audit.subprocess, "check_output", return_value=json.dumps(existing)),
            patch.object(audit, "api_json", return_value={"html_url": "https://example.test/c/42"}) as api,
        ):
            url = audit.upsert_pr_comment("example/argos", "99", f"{audit.COMMENT_MARKER}\nnew")
        self.assertEqual(url, "https://example.test/c/42")
        api.assert_called_once_with(
            "-X", "PATCH", "repos/example/argos/issues/comments/42",
            "-f", f"body={audit.COMMENT_MARKER}\nnew",
        )


if __name__ == "__main__":
    unittest.main()
