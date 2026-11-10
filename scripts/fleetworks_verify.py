#!/usr/bin/env python3
"""Plan, run local checks, or dispatch hosted Fleetworks verification."""

import argparse
import json
import os
import platform
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

PRIMARY_SCOPES = frozenset({"native", "server", "storage", "compose", "broker"})
ALL_SCOPES = PRIMARY_SCOPES | {"bert", "web", "web_dev"}
BERT_ONLY_PATHS = {".github/workflows/atlas-bert.yml"}
BERT_SHARED_PATHS = {
    "apps/venture/src/classifier.rs",
    "services/api-host/src/classifier_service.rs",
    "services/api-host/src/classifier_train.rs",
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
SHARED_PRIMARY_PATHS = {"Cargo.lock", "Cargo.toml", "rust-toolchain.toml", "scripts/agent_cargo.py"}
SHARED_PRIMARY_PREFIXES = (".cargo/", "crates/fleetworks-contracts/", "crates/fleetworks-workspace/")
WEB_BUILD_PATHS = {
    ".github/workflows/intel-web.yml", ".node-version", ".nvmrc", "package.json", "pnpm-lock.yaml",
    "pnpm-workspace.yaml", "tsconfig.json", "playwright.config.ts", "scripts/check_web_build.mjs",
    "scripts/serve_web.mjs", "scripts/test_web_stack.py",
}
SERVER_PREFIXES = (
    "services/api-host/",
    "crates/argos-api-host/",
    "crates/argos-osint-api/",
    "crates/argos-osint-core/",
    "crates/fleetworks-broker/",
    "services/",
)
SERVER_STORAGE_PATHS = {
    "services/api-host/Cargo.toml",
    "services/api-host/src/main.rs",
    "services/api-host/src/fleetworks.rs",
    "services/api-host/src/actor_seed.rs",
    "services/api-host/src/atlas_worker.rs",
}
NATIVE_PACKAGES = (
    "-p", "fleetworks-workspace", "-p", "fleetworks-contracts", "-p", "fleetworks-cli", "-p", "fleetworks-venture",
)
EVENTS = ("push", "pull_request", "workflow_dispatch", "schedule")


def path_scopes(path):
    if not path or path.endswith(".md") or path == "scripts/fleetworks_doc_links.py":
        return set()
    if path.startswith(("docs/", ".planning/")) and path.endswith((".svg", ".html")):
        return set()
    if path.startswith("tools/atlas-bert/") or path in BERT_ONLY_PATHS:
        return {"bert"}
    if path in SHARED_PRIMARY_PATHS or path.startswith(SHARED_PRIMARY_PREFIXES):
        return PRIMARY_SCOPES
    if path in {"scripts/test_surreal.py", "scripts/test_surreal_release.py", "scripts/surrealdb-release.json"}:
        return {"server", "storage", "broker"}
    if path == "scripts/fixtures/atlas_bert":
        return {"bert", "server", "broker", "web", "web_dev"}
    if path == "scripts/fleetworks_network_smoke.py":
        return {"server"}
    if path in WEB_BUILD_PATHS or path.startswith("patches/"):
        return {"web", "web_dev", "server", "compose"}
    if path.startswith(("tests/web/", "vitest")) or path == "eslint.config.mjs":
        return {"web"}

    scopes = {"bert"} if path in BERT_SHARED_PATHS or path.startswith(BERT_SHARED_PREFIXES) else set()
    if path.startswith("apps/venture/"):
        return scopes | {"native", "server", "compose"}
    if path.startswith("apps/fleetworks-cli/"):
        return {"native"}
    if path.startswith("apps/web-desktop/app/api/"):
        return {"web", "web_dev", "server", "compose"}
    if path.startswith(("apps/web-desktop/app/", "packages/")) and not path.endswith(("/package.json", "/tsconfig.json")):
        return {"web", "server", "compose"}
    if path.startswith(("apps/web-desktop/", "packages/")):
        return {"web", "web_dev", "server", "compose"}
    if path.startswith("deploy/"):
        return scopes | {"server", "compose"}
    if path in SERVER_STORAGE_PATHS or path.startswith((
        "crates/argos-osint-core/src/storage/", "crates/argos-osint-core/tests/storage_",
        "crates/argos-osint-core/src/intel_recon/brief_",
    )):
        return scopes | {"server", "storage", "broker"}
    if path.startswith(SERVER_PREFIXES) or path == "scripts/fleetworks_atlas_smoke.py":
        return scopes | {"server", "broker"}

    # Unknown files and planner or workflow edits may affect every lane.
    return ALL_SCOPES


def git_paths(*args):
    raw = subprocess.check_output(["git", *args], stderr=subprocess.DEVNULL)
    return map(os.fsdecode, raw.split(b"\0"))


def valid_sha(value):
    return re.fullmatch(r"[0-9a-f]{40}", value) and value != "0" * 40


def select_scopes(event, base, head, *, worktree=False):
    if event in {"workflow_dispatch", "schedule"} or not valid_sha(base):
        return ALL_SCOPES, "No comparison requested or available"
    separator = "..." if event == "pull_request" else ".."
    diff = ("diff", "--name-only", "--no-renames", "-z")
    scopes = set()
    try:
        for path in git_paths(*diff, f"{base}{separator}{head}", "--"):
            scopes.update(path_scopes(path))
        if worktree:
            for path in git_paths(*diff, "HEAD", "--"):
                scopes.update(path_scopes(path))
            for path in git_paths("ls-files", "--others", "--exclude-standard", "-z"):
                scopes.update(path_scopes(path))
    except (OSError, subprocess.CalledProcessError):
        return ALL_SCOPES, "Git comparison unavailable"
    return scopes, None


def checkout_head(expected):
    try:
        actual = subprocess.check_output(
            ["git", "rev-parse", "--verify", "HEAD"], text=True, stderr=subprocess.DEVNULL
        ).strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError("Unable to read the checked-out Git commit") from error
    if expected is not None and expected != actual:
        raise ValueError("Checkout does not match the requested head")
    return actual


def run(command, *, env=None):
    print(f"$ {shlex.join(command)}", flush=True)
    try:
        return subprocess.run(command, env=env, check=False).returncode
    except OSError as error:
        print(error, file=sys.stderr)
        return 1


def run_local(scopes, comparison=None):
    checks = [
        ["git", "diff", "--check"],
        ["git", "diff", "--cached", "--check"],
        [sys.executable, "scripts/fleetworks_doc_links.py"],
        [sys.executable, "-B", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_fleetworks_*.py"],
        ["node", "scripts/release_check.mjs", "versions"],
        ["node", "--test", *(str(path) for path in sorted(Path(".opencode/tests").glob("*.test.mjs")))],
    ]
    if comparison:
        checks.insert(0, ["git", "diff", "--check", comparison])
    if "native" in scopes:
        checks.extend([
            ["cargo", "fmt", *NATIVE_PACKAGES, "--", "--check"],
            [sys.executable, "scripts/agent_cargo.py", "test", "--locked", *NATIVE_PACKAGES],
        ])
    if "web" in scopes:
        checks.extend(["pnpm", command] for command in ("typecheck", "lint", "test"))
    if scopes:
        print(f"Hosted gates still required: {', '.join(sorted(scopes))}", flush=True)
    for command in checks:
        status = run(command)
        if status:
            return status
    return 0


def timed(phase, command):
    start = time.monotonic()
    status = run(command)
    elapsed = round(time.monotonic() - start, 3)
    print(f"{phase}: {elapsed:.3f}s (exit {status})", flush=True)
    if directory := os.environ.get("RUNNER_TEMP"):
        record = {
            "phase": phase, "seconds": elapsed, "exit": status, "system": platform.system(),
            "machine": platform.machine(), "revision": os.environ.get("FLEETWORKS_HEAD"),
        }
        with (Path(directory) / "fleetworks-timings.jsonl").open("a", encoding="utf-8") as output:
            print(json.dumps(record), file=output)
    return status


def dispatch(args, head):
    command = ["gh", "workflow", "run", "fleetworks.yml", f"--ref={args.ref}", "-f", f"mode={args.mode}", "-f", f"head={head}"]
    if args.mode == "pr":
        if not valid_sha(args.base):
            raise ValueError("A scoped dispatch needs an exact --base commit SHA")
        command.extend(["-f", f"base={args.base}"])
    return run(command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name, help_text, default_event in (
        ("plan", "print selected scopes without running their checks", "push"),
        ("local", "run local checks for selected scopes and working changes", "push"),
        ("pr", "select the hosted checks for a push or PR comparison", "pull_request"),
        ("full", "select every hosted check", "workflow_dispatch"),
    ):
        mode = commands.add_parser(name, help=help_text)
        mode.add_argument("--event", choices=EVENTS, default=default_event)
        mode.add_argument("--base", default="", metavar="SHA", help="comparison commit, or omit to select all")
        mode.add_argument("--head", metavar="SHA", help="expected checkout commit, defaulting to HEAD")
        if name != "local":
            mode.add_argument("--github-output", type=Path, help="append Boolean scope outputs for GitHub Actions")
    remote = commands.add_parser("dispatch", help="explicitly start a GitHub Actions workflow run")
    remote.add_argument("mode", choices=("pr", "full"))
    remote.add_argument("--ref", required=True, help="remote branch or tag at the checked-out commit")
    remote.add_argument("--head", metavar="SHA", help="expected checkout commit, defaulting to HEAD")
    remote.add_argument("--base", default="", metavar="SHA", help="required exact comparison commit for pr")
    timer = commands.add_parser("timed", help="execute a command and record a CI phase duration")
    timer.add_argument("--phase", required=True)
    timer.add_argument("argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    try:
        if args.command == "timed":
            command = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
            if not command:
                raise ValueError("A timed phase needs a command")
            return timed(args.phase, command)
        actual = checkout_head(args.head)
        if args.command == "dispatch":
            return dispatch(args, actual)
        event = "workflow_dispatch" if args.command == "full" else args.event
        scopes, reason = select_scopes(event, args.base, actual, worktree=args.command == "local")
        if reason:
            print(f"{reason}. Selecting all scopes.")
        if output_path := getattr(args, "github_output", None):
            with output_path.open("a", encoding="utf-8") as output:
                for scope in sorted(ALL_SCOPES):
                    print(f"{scope}={str(scope in scopes).lower()}", file=output)
                print(f"full={str(args.command == 'full').lower()}", file=output)
        print(f"{actual}: {', '.join(sorted(scopes)) or 'no runtime scopes selected'}", flush=True)
        if args.command == "local":
            separator = "..." if event == "pull_request" else ".."
            return run_local(scopes, f"{args.base}{separator}{actual}" if reason is None else None)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
