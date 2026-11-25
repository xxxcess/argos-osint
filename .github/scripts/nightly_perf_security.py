#!/usr/bin/env python3
"""Static Performance, Security & Scalability audit and PR scorecard helper for nightly PRs."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HEAD_RE = re.compile(r"(?!0{40}\Z)[0-9a-f]{40}\Z")
SHA_REF_RE = re.compile(r"^[0-9a-f]{40}$")
USES_RE = re.compile(r"^\s*(?:-\s*)?uses:\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+)@([^\s#]+)")
EXPR_IN_RUN_RE = re.compile(r"\$\{\{\s*(inputs\.|github\.event\.(?:pull_request|issue|comment|head_commit|commits)|github\.head_ref)\b")
DOCKER_FROM_RE = re.compile(r"^\s*FROM\s+(?:--platform=\S+\s+)?([^\s]+)(?:\s+AS\s+([^\s]+))?", re.IGNORECASE)
THREAD_SLEEP_RE = re.compile(r"\bstd::thread::sleep\s*\(")
UNBOUNDED_SELECT_RE = re.compile(r"SELECT\s+\*\s+FROM\s+[A-Za-z_][A-Za-z0-9_]*", re.IGNORECASE)
DANGEROUS_WEB_RE = re.compile(r"\b(dangerouslySetInnerHTML|eval\s*\()")
INFINITE_QUERY_RE = re.compile(r"\buseInfiniteQuery\s*(?:<[^>]*>)?\s*\(")

COMMENT_MARKER = "<!-- gsd-perf-security-review -->"
BASELINE_SHA = "2d767d1"
BASELINE_SCORES = {
    "performance": 64,
    "security": 81,
    "scalability": 58,
    "composite": 68,
}


def fail(message: str) -> None:
    raise ValueError(message)


def api_json(*parts: str) -> dict:
    output = subprocess.check_output(["gh", "api", *parts], text=True)
    return json.loads(output)


def changed_files(base: str, head: str, root: Path) -> list[str]:
    if not HEAD_RE.fullmatch(base) or not HEAD_RE.fullmatch(head):
        fail("Audit comparison requires exact 40-character base and head SHAs")
    raw = subprocess.check_output(
        ["git", "diff", "--name-only", "--diff-filter=ACMR", "-z", f"{base}...{head}", "--"],
        cwd=root,
    )
    return sorted(p for p in (os.fsdecode(item) for item in raw.split(b"\0")) if p)


def validate_pr(repo: str, pr_number: str, head: str, *, require_same_repo: bool) -> dict:
    expected_repo = os.environ.get("GITHUB_REPOSITORY")
    if expected_repo and repo != expected_repo:
        fail("Review repository differs from the workflow repository")
    if not HEAD_RE.fullmatch(head) or not pr_number.isdecimal() or int(pr_number) < 1:
        fail("An open PR number and exact 40-character head SHA are required")
    pr = api_json(f"repos/{repo}/pulls/{pr_number}")
    if pr.get("state") != "open":
        fail("Review requires an open pull request")
    if (pr.get("base") or {}).get("ref") != "nightly":
        fail("Nightly Performance & Security reviews only PRs targeting nightly")
    if (pr.get("head") or {}).get("sha") != head:
        fail("PR head moved away from the requested exact SHA")
    if require_same_repo:
        head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
        base_repo = ((pr.get("base") or {}).get("repo") or {}).get("full_name")
        if head_repo != repo or base_repo != repo:
            fail("Comment writes require a same-repository pull request")
    return {
        "pr": int(pr_number),
        "head": head,
        "base": pr["base"]["sha"],
        "title": pr.get("title") or "",
        "url": pr.get("html_url") or "",
    }


def _scan_workflow(rel: str, lines: list[str]) -> list[dict]:
    findings: list[dict] = []
    in_run_block = False
    run_indent = 0
    for idx, line in enumerate(lines, 1):
        stripped = line.lstrip()
        indent = len(line) - len(stripped)
        if in_run_block and stripped and indent <= run_indent:
            in_run_block = False

        uses_match = USES_RE.match(line)
        if uses_match:
            action, ref = uses_match.group(1), uses_match.group(2)
            if not SHA_REF_RE.fullmatch(ref):
                findings.append({
                    "axis": "security",
                    "severity": "high",
                    "rule": "ci-unpinned-action",
                    "file": rel,
                    "line": idx,
                    "message": f"Action `{action}@{ref}` is not pinned to a 40-character commit SHA.",
                    "spec": "Pin third-party GitHub Actions to an immutable 40-char commit SHA with a version comment.",
                })

        if re.match(r"^\s*(?:-\s*)?run:\s*[|>]", line):
            in_run_block = True
            run_indent = indent
            continue
        if re.match(r"^\s*(?:-\s*)?run:\s+\S", line):
            if EXPR_IN_RUN_RE.search(line):
                findings.append({
                    "axis": "security",
                    "severity": "high",
                    "rule": "ci-expr-injection",
                    "file": rel,
                    "line": idx,
                    "message": "Untrusted GitHub expression interpolated directly inside shell `run:`.",
                    "spec": "Bind `${{ ... }}` to an `env:` variable and reference `\"$VAR\"` inside bash.",
                })
        elif in_run_block and EXPR_IN_RUN_RE.search(line):
            findings.append({
                "axis": "security",
                "severity": "high",
                "rule": "ci-expr-injection",
                "file": rel,
                "line": idx,
                "message": "Untrusted GitHub expression interpolated directly inside multiline `run:` block.",
                "spec": "Bind `${{ ... }}` to an `env:` variable and reference `\"$VAR\"` inside bash.",
            })
    return findings


def _scan_dockerfile(rel: str, lines: list[str]) -> list[dict]:
    findings: list[dict] = []
    stages: set[str] = set()
    for idx, line in enumerate(lines, 1):
        match = DOCKER_FROM_RE.match(line)
        if not match:
            continue
        image, alias = match.group(1), match.group(2)
        if alias:
            stages.add(alias.lower())
        if image.lower() == "scratch" or image.lower() in stages or image.startswith("$"):
            continue
        if "@sha256:" not in image:
            findings.append({
                "axis": "security",
                "severity": "medium",
                "rule": "docker-unpinned-from",
                "file": rel,
                "line": idx,
                "message": f"Base image `{image}` is not pinned by `@sha256:` digest.",
                "spec": "Pin external `FROM` base images to an immutable `@sha256:` digest for reproducible builds.",
            })
    return findings


def _scan_rust(rel: str, lines: list[str]) -> list[dict]:
    if "/tests/" in f"/{rel}" or rel.endswith("_tests.rs") or rel.endswith("tests.rs"):
        return []
    findings: list[dict] = []
    in_test_mod = False
    for idx, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith("#[cfg(test)]"):
            in_test_mod = True
        if in_test_mod or stripped.startswith("//"):
            continue
        if THREAD_SLEEP_RE.search(line):
            findings.append({
                "axis": "performance",
                "severity": "high",
                "rule": "rust-blocking-sleep",
                "file": rel,
                "line": idx,
                "message": "`std::thread::sleep` found in non-test Rust source; risks blocking Tokio workers.",
                "spec": "Use `tokio::time::sleep` in async code or isolate inside dedicated blocking workers.",
            })
        if UNBOUNDED_SELECT_RE.search(line) and "LIMIT" not in line.upper() and "COUNT(" not in line.upper():
            findings.append({
                "axis": "scalability",
                "severity": "medium",
                "rule": "surreal-unbounded-select",
                "file": rel,
                "line": idx,
                "message": "`SELECT * FROM ...` without `LIMIT` on this line; verify keyset/row bounds.",
                "spec": "Project only required columns and enforce explicit `LIMIT` + keyset continuation.",
            })
    return findings


def _scan_web(rel: str, lines: list[str]) -> list[dict]:
    if "__tests__" in rel or ".test." in rel or ".spec." in rel:
        return []
    findings: list[dict] = []
    joined = "\n".join(lines)
    for idx, line in enumerate(lines, 1):
        if line.strip().startswith("//"):
            continue
        if DANGEROUS_WEB_RE.search(line):
            findings.append({
                "axis": "security",
                "severity": "high",
                "rule": "web-dangerous-sink",
                "file": rel,
                "line": idx,
                "message": "Dangerous HTML/script execution sink (`dangerouslySetInnerHTML` or `eval`).",
                "spec": "Render structured React Native primitives with escaped text instead of raw HTML/eval.",
            })
        if INFINITE_QUERY_RE.search(line) and "maxPages" not in joined:
            findings.append({
                "axis": "scalability",
                "severity": "medium",
                "rule": "query-unbounded-infinite-pages",
                "file": rel,
                "line": idx,
                "message": "`useInfiniteQuery` without `maxPages` cap; refetch intervals can amplify DB load.",
                "spec": "Set an explicit `maxPages` window or separate head-revision polling from page fetches.",
            })
    return findings


def audit_files(files: list[str], root: Path) -> dict:
    findings: list[dict] = []
    scanned = 0
    for rel in files:
        path = root / rel
        if not path.is_file() or path.is_symlink():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        lines = text.splitlines()
        scanned += 1
        if rel.startswith(".github/workflows/") and rel.endswith((".yml", ".yaml")):
            findings.extend(_scan_workflow(rel, lines))
        elif Path(rel).name.startswith("Dockerfile"):
            findings.extend(_scan_dockerfile(rel, lines))
        elif rel.endswith(".rs") and rel.startswith(("crates/", "services/", "apps/fleetworks-desktop/", "tools/")):
            findings.extend(_scan_rust(rel, lines))
        elif rel.endswith((".ts", ".tsx", ".js", ".mjs")) and rel.startswith(("apps/web-desktop/", "packages/")):
            findings.extend(_scan_web(rel, lines))

    axis_counts = {
        axis: {
            sev: sum(1 for item in findings if item["axis"] == axis and item["severity"] == sev)
            for sev in ("high", "medium", "low")
        }
        for axis in ("performance", "security", "scalability")
    }
    deltas = {}
    for axis, counts in axis_counts.items():
        penalty = counts["high"] * 2 + counts["medium"]
        deltas[axis] = -penalty if penalty else 0
    projected = {
        axis: max(0, min(100, BASELINE_SCORES[axis] + deltas[axis]))
        for axis in ("performance", "security", "scalability")
    }
    projected["composite"] = round(
        (projected["performance"] + projected["security"] + projected["scalability"]) / 3
    )
    high_count = sum(counts["high"] for counts in axis_counts.values())
    return {
        "baseline_sha": BASELINE_SHA,
        "baseline_scores": BASELINE_SCORES,
        "scanned_files": scanned,
        "changed_files": files,
        "findings": findings,
        "axis_counts": axis_counts,
        "static_deltas": deltas,
        "projected_scores": projected,
        "high_findings": high_count,
        "status": "attention_needed" if findings else "clean",
    }


def render_markdown(report: dict, *, head: str, run_url: str = "") -> str:
    lines = [
        COMMENT_MARKER,
        f"### Nightly Performance & Security Review (`gbot`) · `{head[:12]}`",
        "",
    ]
    if run_url:
        lines.append(f"Automated static audit from [GitHub Actions run]({run_url}) against `nightly` baseline `{report['baseline_sha']}`.")
    else:
        lines.append(f"Static audit against `nightly` baseline `{report['baseline_sha']}`.")
    lines += [
        "",
        "| Tenet | Baseline | Static Delta | Projected | High | Medium |",
        "| --- | :---: | :---: | :---: | :---: | :---: |",
    ]
    for axis in ("performance", "security", "scalability"):
        base_score = report["baseline_scores"][axis]
        delta = report["static_deltas"][axis]
        delta_str = f"{delta:+d}" if delta else "0"
        proj = report["projected_scores"][axis]
        counts = report["axis_counts"][axis]
        lines.append(
            f"| **{axis.title()}** | {base_score}/100 | `{delta_str}` | {proj}/100 | {counts['high']} | {counts['medium']} |"
        )
    lines += [
        f"| **Composite** | {report['baseline_scores']['composite']}/100 | — | {report['projected_scores']['composite']}/100 | {report['high_findings']} | — |",
        "",
        f"Scanned **{report['scanned_files']}** changed file(s). "
        "See [scorecard & specs](docs/wiki/code/performance-security-scalability.md) for rubric and backlog.",
        "",
    ]
    findings = report["findings"]
    if findings:
        lines.append("#### Actionable findings & agent-ready specs")
        lines.append("")
        for item in findings[:30]:
            lines.append(
                f"- **[{item['axis'].upper()} · {item['severity'].upper()}]** "
                f"`{item['file']}:{item['line']}` (`{item['rule']}`): {item['message']} "
                f"**Spec:** {item['spec']}"
            )
        if len(findings) > 30:
            lines.append(f"- …and {len(findings) - 30} additional finding(s) in the JSON artifact.")
        lines.append("")
    else:
        lines += [
            "#### Static checks",
            "",
            "- No automated Performance, Security, or Scalability static regressions detected on the changed files.",
            "",
        ]
    lines += [
        "#### Reviewer notes",
        "- `gbot` / **Nightly Performance & Security** is review-only (does not deliver product code); assign accepted specs to **GSD Nightly: Core**.",
        "- Deeper KPI deltas, complementary feature proposals, architectural shifts, and CI run observations are recorded when an agent review pass inspects this revision.",
    ]
    return "\n".join(lines) + "\n"


def upsert_pr_comment(repo: str, pr_number: str, body: str) -> str:
    raw = subprocess.check_output(
        ["gh", "api", f"repos/{repo}/issues/{pr_number}/comments", "--paginate", "--slurp"],
        text=True,
    )
    comments = [item for page in json.loads(raw or "[]") for item in (page if isinstance(page, list) else [page])]
    existing = next((c for c in comments if COMMENT_MARKER in (c.get("body") or "")), None)
    if existing:
        updated = api_json(
            "-X", "PATCH", f"repos/{repo}/issues/comments/{existing['id']}",
            "-f", f"body={body}",
        )
        return updated.get("html_url", "")
    created = api_json(
        "-X", "POST", f"repos/{repo}/issues/{pr_number}/comments",
        "-f", f"body={body}",
    )
    return created.get("html_url", "")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    val = sub.add_parser("validate")
    val.add_argument("--repo", required=True)
    val.add_argument("--pr", required=True)
    val.add_argument("--head", required=True)
    val.add_argument("--require-same-repo", action="store_true")

    aud = sub.add_parser("audit")
    aud.add_argument("--root", type=Path, default=Path("."))
    aud.add_argument("--base")
    aud.add_argument("--head", required=True)
    aud.add_argument("--files", nargs="*")
    aud.add_argument("--run-url", default="")
    aud.add_argument("--json-output", type=Path)
    aud.add_argument("--markdown-output", type=Path)

    post = sub.add_parser("comment")
    post.add_argument("--repo", required=True)
    post.add_argument("--pr", required=True)
    post.add_argument("--head", required=True)
    post.add_argument("--body-file", type=Path, required=True)

    args = parser.parse_args()
    try:
        if args.command == "validate":
            info = validate_pr(args.repo, args.pr, args.head, require_same_repo=args.require_same_repo)
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
                    for key in ("pr", "head", "base"):
                        print(f"{key}={info[key]}", file=out)
            print(json.dumps(info))
        elif args.command == "audit":
            if not HEAD_RE.fullmatch(args.head):
                fail("Audit requires an exact 40-character head SHA")
            files = sorted(args.files) if args.files is not None else changed_files(args.base or "", args.head, args.root)
            report = audit_files(files, args.root)
            report["head"] = args.head
            report["base"] = args.base or ""
            markdown = render_markdown(report, head=args.head, run_url=args.run_url)
            if args.json_output:
                args.json_output.parent.mkdir(parents=True, exist_ok=True)
                args.json_output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            if args.markdown_output:
                args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
                args.markdown_output.write_text(markdown, encoding="utf-8")
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
                    print(f"findings_count={len(report['findings'])}", file=out)
                    print(f"high_findings={report['high_findings']}", file=out)
                    print(f"status={report['status']}", file=out)
            print(json.dumps({"scanned_files": report["scanned_files"], "findings": len(report["findings"]), "status": report["status"]}))
        else:
            validate_pr(args.repo, args.pr, args.head, require_same_repo=True)
            body = args.body_file.read_text(encoding="utf-8")
            if COMMENT_MARKER not in body:
                fail("Comment body is missing the Nightly Performance & Security marker")
            url = upsert_pr_comment(args.repo, args.pr, body)
            print(url)
    except (ValueError, subprocess.CalledProcessError) as error:
        print(f"nightly_perf_security: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
