#!/usr/bin/env python3
"""Poll open PRs, CI status, and Cursor security findings for the GSD workflow."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

SPEC_TITLE_RE = re.compile(r"^spec\s+implementation:\s*(.+)$", re.IGNORECASE)
SEVERITY_RE = re.compile(r"Severity:\s*([A-Z]+)", re.IGNORECASE)
STALE_MARKER = "<summary>Stale comment</summary>"
SKIP_BRANCH_PREFIXES = ("release-please--",)
CURSOR_LOGINS = {"cursor[bot]", "cursor-bot", "cursor"}


def run_gh(args: list[str]) -> str:
    proc = subprocess.run(
        ["gh", *args],
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)} failed ({proc.returncode}): {proc.stderr.strip()}")
    return proc.stdout


def gh_json_paginated(endpoint: str) -> list[dict]:
    """Fetch a paginated GitHub REST list endpoint safely across multiple pages."""
    raw = run_gh(["api", endpoint, "--paginate", "--slurp"])
    if not raw.strip():
        return []
    pages = json.loads(raw)
    items: list[dict] = []
    for page in pages:
        if isinstance(page, list):
            items.extend(page)
        elif isinstance(page, dict):
            items.append(page)
    return items


def is_cursor_author(item: dict) -> bool:
    login = ((item.get("user") or {}).get("login") or "").lower()
    return login in CURSOR_LOGINS or login.startswith("cursor[bot]")


def fetch_cursor_security_findings(repo: str, pr_number: int, addressed_ids: set[str]) -> list[dict]:
    """Collect active (non-stale) Cursor bot review/security comments for a PR."""
    findings: list[dict] = []
    endpoints = (
        ("review_comment", f"repos/{repo}/pulls/{pr_number}/comments"),
        ("review", f"repos/{repo}/pulls/{pr_number}/reviews"),
        ("issue_comment", f"repos/{repo}/issues/{pr_number}/comments"),
    )
    for source, endpoint in endpoints:
        for item in gh_json_paginated(endpoint):
            if not is_cursor_author(item):
                continue
            body = (item.get("body") or "").strip()
            if not body or STALE_MARKER in body:
                continue
            cid = f"{source}:{item.get('id')}"
            severity_match = SEVERITY_RE.search(body)
            findings.append({
                "uid": cid,
                "id": item.get("id"),
                "source": source,
                "path": item.get("path"),
                "line": item.get("line") or item.get("original_line"),
                "commit_id": item.get("commit_id"),
                "severity": severity_match.group(1).upper() if severity_match else "INFO",
                "url": item.get("html_url") or "",
                "unaddressed": cid not in addressed_ids,
                "body": body,
            })
    return findings


def summarize_checks(rollup: list[dict] | None) -> dict:
    counts = {"pass": 0, "fail": 0, "pending": 0, "skip": 0}
    failed = []
    pending = []
    for item in rollup or []:
        typename = item.get("__typename")
        if typename == "StatusContext":
            state = (item.get("state") or "").upper()
            name = item.get("context") or "status"
            url = item.get("targetUrl") or ""
            if state == "SUCCESS":
                counts["pass"] += 1
            elif state in {"FAILURE", "ERROR"}:
                counts["fail"] += 1
                failed.append({"name": name, "state": state, "url": url})
            else:
                counts["pending"] += 1
                pending.append({"name": name, "state": state, "url": url})
            continue

        name = item.get("name") or item.get("workflowName") or "check"
        status = (item.get("status") or "").upper()
        conclusion = (item.get("conclusion") or "").upper()
        url = item.get("detailsUrl") or ""
        if status != "COMPLETED":
            counts["pending"] += 1
            pending.append({"name": name, "state": status or "PENDING", "url": url})
        elif conclusion == "SUCCESS":
            counts["pass"] += 1
        elif conclusion in {"SKIPPED", "NEUTRAL"}:
            counts["skip"] += 1
        else:
            counts["fail"] += 1
            failed.append({"name": name, "state": conclusion or "FAILURE", "url": url})

    return {"counts": counts, "failed": failed, "pending": pending}


def verification_passes(rollup: list[dict] | None) -> dict:
    checks = rollup or []
    return {
        stage: summarize_checks([
            check for check in checks
            if (check.get("name") or check.get("context")) == name
        ])
        for stage, name in (
            ("first", "Fleetworks required"),
            ("second", "Fleetworks second pass"),
        )
    }


def load_state(path: Path, hours: float, reset: bool) -> dict:
    now = datetime.now(timezone.utc)
    if path.exists() and not reset:
        data = json.loads(path.read_text(encoding="utf-8"))
    else:
        end = now + timedelta(hours=hours)
        data = {
            "started_at": now.isoformat(),
            "end_at": end.isoformat(),
            "cycles": 0,
            "prs": {},
        }
    data["last_poll_at"] = now.isoformat()
    data["cycles"] = int(data.get("cycles", 0)) + 1
    return data


def main() -> int:
    parser = argparse.ArgumentParser(description="Poll open PRs for GSD")
    parser.add_argument("--repo", required=True, help="owner/repo")
    parser.add_argument("--state", required=True, type=Path, help="Path to gsd-state.json")
    parser.add_argument("--hours", type=float, default=1.0, help="Window duration in hours")
    parser.add_argument("--reset", action="store_true", help="Reset state timer")
    args = parser.parse_args()

    fields = (
        "number,title,headRefName,headRefOid,baseRefName,"
        "isDraft,isCrossRepository,url,statusCheckRollup,updatedAt"
    )
    prs = json.loads(run_gh(["pr", "list", "-R", args.repo, "--state", "open", "--json", fields]))

    state = load_state(args.state, args.hours, args.reset)
    now = datetime.now(timezone.utc)
    end_at = datetime.fromisoformat(state["end_at"])
    remaining_seconds = max(0, int((end_at - now).total_seconds()))

    report = {
        "now": now.isoformat(),
        "started_at": state["started_at"],
        "end_at": state["end_at"],
        "remaining_seconds": remaining_seconds,
        "window_elapsed": remaining_seconds == 0,
        "cycle": state["cycles"],
        "spec_prs": [],
        "babysit_prs": [],
        "skipped_prs": [],
    }

    for pr in prs:
        num = str(pr["number"])
        title = pr["title"]
        branch = pr["headRefName"]
        head_sha = pr["headRefOid"]
        spec_match = SPEC_TITLE_RE.match(title.strip())
        rollup = pr.get("statusCheckRollup")
        checks = summarize_checks(rollup)
        passes = verification_passes(rollup)
        prev = state["prs"].get(num, {})
        addressed_ids = set(prev.get("addressed_cursor_comments", []))

        if pr.get("isCrossRepository") or branch.startswith(SKIP_BRANCH_PREFIXES):
            report["skipped_prs"].append({
                "number": pr["number"],
                "title": title,
                "branch": branch,
                "reason": "fork" if pr.get("isCrossRepository") else "automated-release-branch",
            })
            continue

        cursor_findings = fetch_cursor_security_findings(args.repo, pr["number"], addressed_ids)
        unaddressed_cursor = [f for f in cursor_findings if f["unaddressed"]]

        entry = {
            "number": pr["number"],
            "title": title,
            "feature": spec_match.group(1).strip() if spec_match else None,
            "branch": branch,
            "base": pr["baseRefName"],
            "head_sha": head_sha,
            "draft": bool(pr.get("isDraft")),
            "url": pr["url"],
            "checks": checks["counts"],
            "verification_passes": passes,
            "failed_checks": checks["failed"],
            "pending_checks": len(checks["pending"]),
            "cursor_security_findings": cursor_findings,
            "unaddressed_cursor_count": len(unaddressed_cursor),
            "tracked_status": prev.get("status", "new"),
            "tracked_sha": prev.get("head_sha"),
            "needs_implementation": bool(
                spec_match and prev.get("implemented_sha") != head_sha and prev.get("pushed_sha") != head_sha
            ),
        }

        state["prs"][num] = {
            **prev,
            "title": title,
            "branch": branch,
            "head_sha": head_sha,
            "kind": "spec_impl" if spec_match else "babysit",
            "last_checks": checks["counts"],
            "verification_passes": passes,
            "last_unaddressed_cursor_count": len(unaddressed_cursor),
        }

        if spec_match:
            report["spec_prs"].append(entry)
        else:
            report["babysit_prs"].append(entry)

    args.state.parent.mkdir(parents=True, exist_ok=True)
    args.state.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
