#!/usr/bin/env python3
"""Plan Fleetworks verification for the checked-out Git commit."""

import argparse
import os
import re
import subprocess
from pathlib import Path

PRIMARY_SCOPES = frozenset({"native", "server", "compose", "broker"})
ALL_SCOPES = PRIMARY_SCOPES | {"bert"}
BERT_ONLY_PATHS = {".github/workflows/atlas-bert.yml", "scripts/fixtures/atlas_bert"}
BERT_SHARED_PATHS = {
    "apps/venture/src/classifier.rs",
    "crates/argos-api-host/src/classifier_service.rs",
    "crates/argos-api-host/src/classifier_train.rs",
    "crates/argos-osint-core/tests/adversarial_classifier_runtime.rs",
    "deploy/compose.atlas.yml",
}
BERT_SHARED_PREFIXES = (
    "crates/argos-osint-core/src/classifier/",
    "crates/argos-osint-core/src/atlas_classify",
    "crates/argos-osint-core/src/storage/classifier_",
    "crates/argos-osint-core/tests/classifier_",
    "crates/argos-osint-core/tests/fixtures/classifier/",
    "crates/argos-osint-core/tests/storage_classifier_",
)
SHARED_PRIMARY_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "rust-toolchain.toml",
    "scripts/agent_cargo.py",
    "scripts/fleetworks_network_smoke.py",
    "scripts/test_surreal.py",
    "tsconfig.json",
}
SHARED_PRIMARY_PREFIXES = (
    ".cargo/",
    "crates/fleetworks-contracts/",
    "crates/fleetworks-workspace/",
    "packages/",
    "patches/",
)
SERVER_PREFIXES = (
    "crates/argos-api-host/",
    "crates/argos-osint-api/",
    "crates/argos-osint-core/",
    "crates/fleetworks-broker/",
)


def path_scopes(path):
    if not path or path in {"README.md", "CHANGELOG.md"}:
        return set()
    if path.startswith(("docs/", ".planning/")) and path.endswith((".md", ".svg", ".html")):
        return set()
    if path.startswith("tools/atlas-bert/") or path in BERT_ONLY_PATHS:
        return {"bert"}
    if path in SHARED_PRIMARY_PATHS or path.startswith(SHARED_PRIMARY_PREFIXES):
        return PRIMARY_SCOPES

    scopes = {"bert"} if path in BERT_SHARED_PATHS or path.startswith(BERT_SHARED_PREFIXES) else set()
    if path.startswith("apps/venture/"):
        return scopes | {"native", "server", "compose"}
    if path.startswith("apps/fleetworks-cli/"):
        return {"native"}
    if path.startswith(("apps/web-desktop/", "deploy/")):
        return scopes | {"server", "compose"}
    if path.startswith(SERVER_PREFIXES) or path == "scripts/fleetworks_atlas_smoke.py":
        return scopes | {"server", "broker"}

    # Unknown files and planner or workflow edits may affect every lane.
    return ALL_SCOPES


def select_scopes(event, base, head):
    if event == "workflow_dispatch" or not re.fullmatch(r"[0-9a-f]{40}", base) or base == "0" * 40:
        return ALL_SCOPES, "No comparison requested or available"
    separator = "..." if event == "pull_request" else ".."
    diff = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", f"{base}{separator}{head}", "--"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if diff.returncode:
        return ALL_SCOPES, "Git comparison unavailable"
    scopes = set()
    for path in map(os.fsdecode, diff.stdout.split(b"\0")):
        scopes.update(path_scopes(path))
    return scopes, None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan", help="print selected scopes without running their checks")
    plan.add_argument("--event", choices=("push", "pull_request", "workflow_dispatch"), default="push")
    plan.add_argument("--base", default="", metavar="SHA", help="comparison commit, or omit to select all")
    plan.add_argument("--head", metavar="SHA", help="expected checkout commit, defaulting to HEAD")
    plan.add_argument("--github-output", type=Path, help="append Boolean scope outputs for GitHub Actions")
    args = parser.parse_args()

    try:
        actual = subprocess.check_output(
            ["git", "rev-parse", "--verify", "HEAD"], text=True, stderr=subprocess.DEVNULL
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        parser.exit(1, "Unable to read the checked-out Git commit\n")
    if args.head is not None and args.head != actual:
        parser.exit(1, "Checkout does not match the requested head\n")
    scopes, reason = select_scopes(args.event, args.base, actual)
    if reason:
        print(f"{reason}. Selecting all scopes.")
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            for scope in sorted(ALL_SCOPES):
                print(f"{scope}={str(scope in scopes).lower()}", file=output)
    print(f"{actual}: {', '.join(sorted(scopes)) or 'no runtime scopes selected'}")


if __name__ == "__main__":
    main()
