#!/usr/bin/env python3
"""Select Fleetworks PR lanes without changing the source-bound release plan."""

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


# These files are verified by the scope job itself. Exercise changed orchestration
# with a native candidate, and route changes to reusable backend checks by owner.
POLICY_SCOPES = {
    ".github/workflows/fleetworks.yml": {"desktop", "local_backend"},
    ".github/workflows/embedded.yml": {"server", "storage", "local_backend"},
    ".github/scripts/fleetworks_ci.py": {"desktop", "local_backend"},
    ".github/scripts/test_fleetworks_ci.py": {"desktop", "local_backend"},
    "scripts/fleetworks_verify.py": {"desktop", "local_backend"},
    "scripts/test_fleetworks_verify.py": {"desktop", "local_backend"},
    "deploy/compose.yml": {"server", "compose", "web", "web_dev"},
    "crates/argos-osint-api/tests/classifier_activation.rs": {"desktop", "server", "storage"},
}
ARCHITECTURES = {
    "amd64": ("ubuntu-24.04", "x86_64", "ELF 64-bit.*x86-64"),
    "arm64": ("ubuntu-24.04-arm", "aarch64", "ELF 64-bit.*(aarch64|ARM aarch64)"),
}
SCOPES = {
    "bert", "compose", "desktop", "images", "local_backend",
    "queue", "server", "storage", "web", "web_dev",
}


def path_selection(planner, path):
    if not path or path.startswith("docs/design/"):
        return set(), set()
    if path in POLICY_SCOPES:
        return POLICY_SCOPES[path], set()
    if path.startswith("assets/brand/"):
        scopes = {"desktop", "local_backend"}
        if not path.startswith(("assets/brand/orbit/", "assets/brand/reference/")):
            scopes.add("web")
        return scopes, set()
    return planner.path_scopes(path), planner.images_for(path)


def compare(planner, base, head):
    """Use classifiers from the trusted base for the actual three-dot PR diff."""
    scopes, images = set(), set()
    try:
        paths = planner.git_paths("diff", "--name-only", "--no-renames", "-z",
                                  f"{base}...{head}", "--")
        for path in paths:
            changed_scopes, changed_images = path_selection(planner, path)
            scopes.update(changed_scopes)
            images.update(changed_images)
    except (OSError, subprocess.CalledProcessError):
        scopes, images = set(planner.ALL_SCOPES), set(planner.ALL_IMAGES)
    return {
        "base": base,
        "components": "auto",
        "event": "pull_request",
        "format": 1,
        "full": False,
        "head": head,
        "image_arches": list(ARCHITECTURES),
        "image_targets": sorted(images),
        "scopes": sorted(scopes),
    }


def backend_matrix(arches):
    entries = []
    for arch in arches:
        runner, machine, executable = ARCHITECTURES[arch]
        entries.append({
            "arch": arch,
            "runner": runner,
            "machine": machine,
            "target": f"{machine}-unknown-linux-gnu",
            "executable": executable,
            "name": f"Linux {arch.upper()}",
        })
    return entries


def image_matrix(stage, targets, compose):
    entries = []
    for arch, (runner, _, _) in ARCHITECTURES.items():
        selected = targets
        smoke_compose = compose
        if stage == "first":
            selected = targets - {"web"} if arch == "amd64" else set()
        elif stage == "second" and arch == "amd64":
            selected = targets & {"web"}
            smoke_compose = False
        if selected:
            entries.append({
                "arch": arch,
                "runner": runner,
                "targets": ",".join(sorted(selected)),
                "compose": smoke_compose,
            })
    return entries


def verification(stage, plan):
    """Apply the pass to checks while leaving release publication inputs intact."""
    scope = plan["scope"]
    scopes = set(scope["scopes"])
    targets = set(scope["image_targets"])
    if stage == "first":
        scopes.difference_update({"web", "web_dev"})
        targets.discard("web")
        # The Web build planner may mark Compose for browser/relay tooling alone.
        if not (scopes & {"server", "storage", "queue"} or targets):
            scopes.discard("compose")
    arches = {
        "first": ["amd64"],
        "second": ["arm64"],
        "standard": list(ARCHITECTURES),
    }[stage]
    images = image_matrix(stage, targets, "compose" in scopes)
    if not images:
        scopes.discard("images")
    native = bool(scopes & {"desktop", "local_backend"})
    return {
        **{name: name in scopes for name in sorted(SCOPES)},
        "desktop_linux": native and stage != "second",
        "desktop_macos": native and stage != "first",
        "backend_matrix": backend_matrix(arches),
        "backend_arch": arches[0] if len(arches) == 1 else "both",
        "image_matrix": images,
        "image_targets": ",".join(sorted(targets)),
        "full": stage == "standard" and scope["full"],
    }


def exact_revision(value):
    return isinstance(value, str) and bool(re.fullmatch(r"(?!0{40}$)[a-f0-9]{40}", value))


def github_json(*arguments):
    return json.loads(subprocess.check_output(["gh", "api", *arguments], text=True))


def promote(repository, branch, number, head, revision, standard_base, components):
    """Accept only an exact open PR with a green first pass for its current base."""
    if (
        not re.fullmatch(r"[1-9][0-9]{0,8}", number)
        or not exact_revision(head)
        or head != revision
        or standard_base
        or components != "auto"
    ):
        raise ValueError("Second pass requires an open PR number and exact branch head; omit base and components")
    pr = github_json(f"repos/{repository}/pulls/{number}")
    source, destination = pr["head"], pr["base"]
    if (
        pr["state"] != "open"
        or (source.get("repo") or {}).get("full_name") != repository
        or source["sha"] != head
        or f"refs/heads/{source['ref']}" != branch
        or destination["ref"] not in {"dev", "nightly", "main"}
        or not exact_revision(destination["sha"])
    ):
        raise ValueError("Dispatch the open same-repository PR branch at its current exact head")
    base = destination["sha"]
    runs = github_json(
        "--method", "GET", f"repos/{repository}/actions/workflows/fleetworks.yml/runs",
        "-f", "event=pull_request", "-f", f"head_sha={head}", "-f", "per_page=100",
    )["workflow_runs"]
    latest = next((
        run for run in runs
        if any(
            record["number"] == int(number) and record["base"]["sha"] == base
            for record in run["pull_requests"]
        )
    ), None)
    if latest is None or latest["conclusion"] != "success":
        raise ValueError("The latest first pass must succeed for this PR, head, and base before second pass")
    return base, latest["id"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    compare_parser = commands.add_parser("compare")
    compare_parser.add_argument("--base", required=True)
    compare_parser.add_argument("--head", required=True)
    compare_parser.add_argument("--output", required=True, type=Path)
    promote_parser = commands.add_parser("promote")
    promote_parser.add_argument("--output", required=True, type=Path)
    select_parser = commands.add_parser("select")
    select_parser.add_argument("--stage", choices=("first", "second", "standard"), required=True)
    select_parser.add_argument("--plan", required=True, type=Path)
    select_parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "compare":
        import fleetworks_verify

        if not all(fleetworks_verify.valid_sha(value) for value in (args.base, args.head)):
            parser.error("Comparison requires exact, nonzero base and head SHAs")
        fleetworks_verify.checkout_head(args.head)
        plan = compare(fleetworks_verify, args.base, args.head)
        args.output.write_text(json.dumps(plan), encoding="utf-8")
    elif args.command == "promote":
        try:
            base, first_run = promote(
                os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_REF"], os.environ["PR_NUMBER"],
                os.environ["EXPECTED_HEAD"], os.environ["FLEETWORKS_HEAD"], os.environ["STANDARD_BASE"],
                os.environ["COMPONENTS"],
            )
        except (ValueError, subprocess.CalledProcessError) as error:
            raise SystemExit(str(error)) from error
        with args.output.open("a", encoding="utf-8") as output:
            print(f"base={base}", file=output)
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a", encoding="utf-8") as summary:
            print(f"Second pass follows successful first-pass run {first_run}.", file=summary)
    else:
        selected = verification(args.stage, json.loads(args.plan.read_text(encoding="utf-8")))
        with args.output.open("a", encoding="utf-8") as output:
            for key, value in selected.items():
                encoded = value if isinstance(value, str) else json.dumps(value, separators=(",", ":"))
                print(f"{key}={encoded}", file=output)


if __name__ == "__main__":
    main()
