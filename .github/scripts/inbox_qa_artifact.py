#!/usr/bin/env python3
"""Find a unique exact-sha artifact from this Actions run, including reruns."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess


def find_artifact(artifacts, family, revision, run_id, max_attempt):
    name = re.compile(re.escape(f"{family}-{revision}-{run_id}.") + r"([1-9][0-9]*)")
    matches = []
    for artifact in artifacts:
        match = name.fullmatch(artifact["name"])
        if match and not artifact["expired"] and int(match[1]) <= max_attempt:
            if type(artifact["id"]) is not int or artifact["id"] <= 0:
                raise ValueError("Invalid artifact identity")
            matches.append((int(match[1]), artifact["id"]))
    if not matches:
        raise ValueError(f"No verified {family} artifact is available in this run")
    latest = max(attempt for attempt, _ in matches)
    selected = [identity for attempt, identity in matches if attempt == latest]
    if len(selected) != 1:
        raise ValueError(f"Ambiguous {family} artifact for this run attempt")
    return selected[0], latest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family", required=True, action="append")
    options = parser.parse_args()
    revision = os.environ["FLEETWORKS_HEAD"]
    run_id = os.environ["GITHUB_RUN_ID"]
    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    if not re.fullmatch(r"(?!0{40})[a-f0-9]{40}", revision):
        raise ValueError("Artifact selection needs an exact source SHA")
    if not all(re.fullmatch(r"[A-Za-z0-9_.-]+", family) for family in options.family) or not run_id.isdecimal():
        raise ValueError("Invalid Actions run or artifact family")
    pages = json.loads(subprocess.check_output([
        "gh", "api", "--paginate", "--slurp",
        f"repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{run_id}/artifacts?per_page=100",
    ], timeout=60, text=True))
    artifacts = [artifact for page in pages for artifact in page["artifacts"]]
    selected = [find_artifact(artifacts, family, revision, run_id, attempt) for family in options.family]
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        if len(selected) == 1:
            identity, selected_attempt = selected[0]
            print(f"id={identity}", file=output)
            print(f"attempt={selected_attempt}", file=output)
        else:
            print("ids=" + ",".join(str(identity) for identity, _ in selected), file=output)


if __name__ == "__main__":
    main()
