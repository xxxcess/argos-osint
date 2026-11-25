#!/usr/bin/env python3
"""Poll open nightly PRs for GSD Core or UI review."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

SPEC_TITLE_RE = re.compile(r"^spec\s+implementation:\s*(.+)$", re.IGNORECASE)
SEVERITY_RE = re.compile(r"Severity:\s*(CRITICAL|HIGH|MEDIUM|LOW|INFO)\b", re.IGNORECASE)
LOGIN_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,99}")
STALE_MARKER = "<summary>Stale comment</summary>"
NIGHTLY = "nightly"
CURSOR_LOGINS = {"cursor[bot]", "cursor-bot", "cursor"}


REVIEW_THREADS = """
query($owner: String!, $repo: String!, $number: Int!, $endCursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      state
      baseRefName
      headRefOid
      author { __typename login ... on User { databaseId } }
      reviewThreads(first: 50, after: $endCursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          originalLine
          comments(first: 1) {
            nodes { databaseId url author { __typename login ... on User { databaseId } } }
          }
        }
      }
    }
  }
}
"""


def run_gh(args: list[str], *, timeout: float | None = None) -> str:
    proc = subprocess.run(
        ["gh", *args],
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
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


def can_direct_work(repo: str, actor: dict | None, permission_cache: dict) -> bool:
    """Check a human principal's current base-repo permission using GitHub metadata."""
    actor = actor if isinstance(actor, dict) else {}
    login = actor.get("login")
    account_id = actor.get("id", actor.get("databaseId"))
    if (
        actor.get("type", actor.get("__typename")) != "User"
        or not isinstance(login, str)
        or not LOGIN_RE.fullmatch(login)
        or type(account_id) is not int
        or account_id <= 0
    ):
        return False

    key = (repo.casefold(), login.casefold(), account_id)
    if key in permission_cache:
        return permission_cache[key]
    try:
        endpoint = f"repos/{repo}/collaborators/{login}/permission"
        result = json.loads(run_gh(["api", "--method", "GET", endpoint], timeout=15))
    except (RuntimeError, OSError, subprocess.TimeoutExpired, ValueError):
        result = {}
    user = result.get("user") if isinstance(result, dict) else None
    # The REST permission field maps maintain to write and triage to read.
    verified = (
        isinstance(user, dict)
        and user.get("type") == "User"
        and type(user.get("id")) is int
        and user["id"] == account_id
        and isinstance(user.get("login"), str)
        and user["login"].casefold() == login.casefold()
        and result.get("permission") in ("admin", "write")
    )
    permission_cache[key] = verified
    return verified


def review_author(repo: str, actor: dict | None, permission_cache: dict) -> dict:
    actor = actor if isinstance(actor, dict) else {}
    return {
        "author": actor.get("login"),
        "author_id": actor.get("id", actor.get("databaseId")),
        "author_type": actor.get("type", actor.get("__typename")),
        "author_has_repository_write_access": can_direct_work(repo, actor, permission_cache),
    }


def fetch_review_discussion(
    repo: str, pr_number: int, addressed_ids: set[str], permission_cache: dict,
) -> tuple[list[dict], list[dict]]:
    """Keep metadata for every reviewer, with separate diagnostic Cursor evidence."""
    discussion: list[dict] = []
    findings: list[dict] = []
    endpoints = (
        ("review_comment", f"repos/{repo}/pulls/{pr_number}/comments"),
        ("review", f"repos/{repo}/pulls/{pr_number}/reviews"),
        ("issue_comment", f"repos/{repo}/issues/{pr_number}/comments"),
    )
    for source, endpoint in endpoints:
        for item in gh_json_paginated(endpoint):
            body = (item.get("body") or "").strip()
            if not body:
                continue
            actor = item.get("user")
            actor = actor if isinstance(actor, dict) else {}
            cid = f"{source}:{item.get('id')}"
            entry = {
                "uid": cid,
                "id": item.get("id"),
                "source": source,
                "path": item.get("path"),
                "line": item.get("line") or item.get("original_line"),
                "commit_id": item.get("commit_id"),
                "url": item.get("html_url") or "",
                **review_author(repo, actor, permission_cache),
            }
            discussion.append(entry)
            login = actor.get("login")
            login = login.casefold() if isinstance(login, str) else ""
            if actor.get("type") != "Bot" or login not in CURSOR_LOGINS or STALE_MARKER in body:
                continue
            severity_match = SEVERITY_RE.search(body)
            findings.append({
                **entry,
                "severity": severity_match.group(1).upper() if severity_match else "INFO",
                "unaddressed": cid not in addressed_ids,
            })
    return discussion, findings


def fetch_unresolved_review_threads(
    repo: str, pr_number: int, head_sha: str, permission_cache: dict,
) -> tuple[list[dict], dict | None] | None:
    """Collect open threads and the PR author on the exact nightly PR head."""
    owner, name = repo.split("/", 1)
    pages = json.loads(run_gh([
        "api", "graphql", "--paginate", "--slurp",
        "-f", f"owner={owner}", "-f", f"repo={name}", "-F", f"number={pr_number}",
        "-f", f"query={REVIEW_THREADS}",
    ]))
    threads = []
    author = None
    for page in pages:
        pr = page["data"]["repository"]["pullRequest"]
        if not pr or pr["state"] != "OPEN" or pr["baseRefName"] != NIGHTLY or pr["headRefOid"] != head_sha:
            return None
        author = pr.get("author")
        for thread in pr["reviewThreads"]["nodes"]:
            if thread["isResolved"]:
                continue
            root, *_ = thread["comments"]["nodes"] or [{}]
            threads.append({
                "id": thread["id"],
                "path": thread["path"],
                "line": thread["line"] or thread["originalLine"],
                "outdated": thread["isOutdated"],
                "comment_id": root.get("databaseId"),
                "url": root.get("url"),
                **review_author(repo, root.get("author"), permission_cache),
            })
    return threads, author


def spec_implementation_pending(
    track: str, spec_match: re.Match | None, cross_repository: bool,
    author_can_direct: bool, head_sha: str, previous: dict,
) -> bool:
    return bool(
        track == "core" and spec_match and not cross_repository and author_can_direct
        and previous.get("implemented_sha") != head_sha and previous.get("pushed_sha") != head_sha
    )


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


def verification_tracks(rollup: list[dict] | None) -> dict:
    tracks = {}
    for track, name in (
        ("core", "GSD Nightly: Core"),
        ("ui", "GSD Nightly: UI"),
        ("perf_security", "Nightly Performance & Security"),
    ):
        selected = []
        for check in rollup or []:
            label = check.get("name") or check.get("context") or ""
            if label == name or label.startswith(f"{name} / "):
                selected.append(check)
        summary = summarize_checks(selected)
        counts = summary["counts"]
        if counts["fail"]:
            status = "failing"
        elif counts["pending"]:
            status = "pending"
        elif counts["pass"]:
            status = "green"
        elif counts["skip"]:
            status = "skipped"
        else:
            status = "unavailable"
        tracks[track] = {"status": status, **summary}
    return tracks


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
    parser = argparse.ArgumentParser(description="Poll open PRs targeting nightly for GSD")
    parser.add_argument("--repo", required=True, help="owner/repo")
    parser.add_argument("--track", choices=("core", "ui", "perf-security"), default="core")
    parser.add_argument("--state", type=Path, help="Override the track’s local state file")
    parser.add_argument("--hours", type=float, default=1.0, help="Window duration in hours")
    parser.add_argument("--reset", action="store_true", help="Reset state timer")
    args = parser.parse_args()
    if args.state is None:
        filenames = {
            "core": "gsd-state.json",
            "ui": "gsd-ui-state.json",
            "perf-security": "gsd-perf-security-state.json",
        }
        args.state = Path(".agent-scratch") / filenames[args.track]

    fields = (
        "number,title,headRefName,headRefOid,baseRefName,"
        "isDraft,isCrossRepository,state,url,statusCheckRollup,updatedAt"
    )
    prs = json.loads(run_gh([
        "pr", "list", "-R", args.repo, "--base", NIGHTLY, "--state", "open",
        "--limit", "1000", "--json", fields,
    ]))

    state = load_state(args.state, args.hours, args.reset)
    now = datetime.now(timezone.utc)
    end_at = datetime.fromisoformat(state["end_at"])
    remaining_seconds = max(0, int((end_at - now).total_seconds()))

    report = {
        "track": args.track,
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

    permission_cache = {}
    for pr in prs:
        num = str(pr["number"])
        title = pr["title"]
        branch = pr["headRefName"]
        head_sha = pr["headRefOid"]
        spec_match = SPEC_TITLE_RE.match(title.strip())
        rollup = pr.get("statusCheckRollup")
        checks = summarize_checks(rollup)
        passes = verification_passes(rollup)
        tracks = verification_tracks(rollup)
        prev = state["prs"].get(num, {})
        kpis = {
            track: evidence.get("kpis", [])
            for track, evidence in prev.get("nightly", {}).items()
            if track in tracks and evidence.get("head_sha") == head_sha
        }
        addressed_ids = set(prev.get("addressed_cursor_comments", []))

        if pr["baseRefName"] != NIGHTLY or pr["state"] != "OPEN":
            report["skipped_prs"].append({
                "number": pr["number"], "title": title, "branch": branch, "reason": "outside-nightly",
            })
            continue

        cursor_findings = []
        discussion = []
        unresolved = None
        author_can_direct = False
        cross_repository = pr.get("isCrossRepository") is not False
        if args.track == "core":
            review = fetch_unresolved_review_threads(args.repo, pr["number"], head_sha, permission_cache)
            if review is None:
                report["skipped_prs"].append({
                    "number": pr["number"], "title": title, "branch": branch, "reason": "changed-during-poll",
                })
                continue
            unresolved, author = review
            if spec_match and not cross_repository:
                author_can_direct = can_direct_work(args.repo, author, permission_cache)
            discussion, cursor_findings = fetch_review_discussion(
                args.repo, pr["number"], addressed_ids, permission_cache,
            )
        unaddressed_cursor_count = (
            sum(f["unaddressed"] for f in cursor_findings) if args.track == "core" else None
        )

        entry = {
            "number": pr["number"],
            "title": title,
            "feature": spec_match.group(1).strip() if spec_match and author_can_direct else None,
            "branch": branch,
            "base": pr["baseRefName"],
            "head_sha": head_sha,
            "draft": bool(pr.get("isDraft")),
            "is_cross_repository": cross_repository,
            "push_scope": "operator_authorization_required" if cross_repository else "same_repository",
            "url": pr["url"],
            "checks": checks["counts"],
            "verification_passes": passes,
            "verification_tracks": tracks,
            "nightly_kpis": kpis,
            "failed_checks": checks["failed"],
            "pending_checks": len(checks["pending"]),
            "unresolved_review_threads": unresolved,
            "review_discussion": discussion,
            "cursor_security_findings": cursor_findings,
            "unaddressed_cursor_count": unaddressed_cursor_count,
            "tracked_status": prev.get("status", "new"),
            "tracked_sha": prev.get("head_sha"),
            "needs_implementation": spec_implementation_pending(
                args.track, spec_match, cross_repository, author_can_direct, head_sha, prev,
            ),
        }

        state["prs"][num] = {
            **prev,
            "title": title,
            "branch": branch,
            "head_sha": head_sha,
            "kind": "spec_impl" if spec_match and author_can_direct else "babysit",
            "last_checks": checks["counts"],
            "verification_passes": passes,
            "verification_tracks": tracks,
            "last_unaddressed_cursor_count": unaddressed_cursor_count,
        }

        if spec_match and author_can_direct:
            report["spec_prs"].append(entry)
        else:
            report["babysit_prs"].append(entry)

    args.state.parent.mkdir(parents=True, exist_ok=True)
    args.state.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
