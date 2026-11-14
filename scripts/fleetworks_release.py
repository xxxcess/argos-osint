#!/usr/bin/env python3
"""Plan immutable, commit-versioned channel artifacts without publishing them.

Stable dev/main builds share reachable release history. Nightly builds also
consume reachable nightly history. Published ancestors are excluded from the
Conventional Commit range, so promoting dev to main does not bump it again.
Source workspace versions and Git refs are never changed by this helper.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

from fleetworks_verify import ALL_IMAGES, ALL_SCOPES, BACKEND_IMAGES
from fleetworks_verify import github_plan_outputs, select_plan

ROOT = Path(__file__).resolve().parents[1]
BRANCHES = {"nightly": "nightly", "dev": "stable", "main": "stable"}
SHA = re.compile(r"[0-9a-f]{40}")
CORE = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
CORE_VERSION = re.compile(CORE)
RELEASE_TAG = re.compile(
    rf"fleetworks-(dev|main|nightly)-v{CORE}"
    r"(?:\+(dev|main)\.g([0-9a-f]{40})|-nightly\.([1-9][0-9]*)\.g([0-9a-f]{40}))"
)
COMMIT = re.compile(
    r"(feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert)"
    r"(?:\([a-z0-9][a-z0-9,._/-]*\))?(!)?: (\S.*)"
)


def git(root, *arguments):
    try:
        return subprocess.check_output(
            ["git", *arguments], cwd=root, text=True, stderr=subprocess.PIPE,
        ).strip()
    except subprocess.CalledProcessError as error:
        raise ValueError("Release planning requires fetched Git history and exact source refs") from error


def core_version(value):
    if not isinstance(value, str) or not CORE_VERSION.fullmatch(value):
        raise ValueError("Workspace release baseline must be an X.Y.Z SemVer")
    return tuple(map(int, value.split(".")))


def version_text(value):
    return ".".join(map(str, value))


def commit_impact(message, baseline, *, merge=False):
    """Match the release policy, including nonempty breaking-change footers.

    Mechanical merge messages can carry their conventional PR title in the
    first nonempty body line. Invalid ordinary commits remain errors here;
    release history uses an explicit conservative legacy fallback separately.
    """
    return _commit_details(message, baseline, merge=merge)[0]


def _commit_details(message, baseline, *, merge=False, legacy=False):
    lines = message.replace("\r\n", "\n").splitlines()
    header = COMMIT.fullmatch(lines[0]) if lines else None
    mechanical = bool(merge and lines and re.match(
        r"Merge (?:pull request #[0-9]+ from |branch |remote-tracking branch |tag )", lines[0],
    ))
    body = "\n".join(lines[1:])
    footers = re.findall(r"^BREAKING(?: CHANGE|-CHANGE):([^\n]*)", body, re.MULTILINE)
    if any(not value.strip() for value in footers):
        raise ValueError("BREAKING CHANGE requires a nonempty compatibility description")
    if not header and mechanical:
        title = next((line.strip() for line in lines[1:] if line.strip()), "")
        header = COMMIT.fullmatch(title)
    untyped = not header and not mechanical
    if untyped and not legacy:
        raise ValueError("Release commits must use Conventional Commits; mechanical merge commits are the only exception")
    if header and header.group(2) and not footers:
        raise ValueError("A breaking ! commit requires its BREAKING CHANGE footer")
    if footers:
        return ("major" if baseline[0] else "minor"), untyped
    if not header:
        # Historical feature branches may contain untyped commits. Preserve
        # their release impact conservatively instead of rewriting history or
        # silently treating unknown work as maintenance. New landing commits
        # retain their independent Conventional Commit policy gate.
        return ("patch" if untyped else "none"), untyped
    if header.group(1) == "feat":
        return "minor", False
    return ("patch" if header.group(1) in {"fix", "perf"} else "none"), False


def bump(version, impact):
    major, minor, patch = version
    if impact == "major":
        return major + 1, 0, 0
    if impact == "minor":
        return major, minor + 1, 0
    if impact == "patch":
        return major, minor, patch + 1
    return version


def artifact_identity(branch, version, head, count):
    channel = BRANCHES[branch]
    core = version_text(version)
    artifact_version = (
        f"{core}-nightly.{count}.g{head}" if channel == "nightly"
        else f"{core}+{branch}.g{head}"
    )
    return {
        "channel": channel,
        "version": artifact_version,
        "tag": f"fleetworks-{branch}-v{artifact_version}",
        "image_tag": f"{channel}-{branch}-{core}-g{head}",
        "prerelease": channel == "nightly",
    }


def published_ancestors(root, head, channel):
    """Read only exact source-bound artifact tags and canonical vX.Y.Z tags."""
    published = []
    for tag in git(root, "tag", "--merged", head).splitlines():
        canonical = CORE_VERSION.fullmatch(tag.removeprefix("v")) if tag.startswith("v") else None
        artifact = RELEASE_TAG.fullmatch(tag)
        if not canonical and not artifact:
            continue
        commit = git(root, "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
        if canonical:
            published.append((core_version(tag[1:]), commit, tag, None))
            continue
        branch, major, minor, patch, metadata_branch, stable_sha, _count, nightly_sha = artifact.groups()
        if (metadata_branch != branch if branch != "nightly" else metadata_branch is not None):
            raise ValueError("Artifact tag branch and version metadata disagree")
        if (stable_sha or nightly_sha) != commit:
            raise ValueError("Artifact tag does not bind its actual source commit")
        if channel == "stable" and branch == "nightly":
            continue
        published.append(((int(major), int(minor), int(patch)), commit, tag, branch))
    return published


def validate_scope(scope, event, base, head):
    if not isinstance(scope, dict) or type(scope.get("format")) is not int or scope["format"] != 1:
        raise ValueError("An explicit version1 scope plan is required")
    if (scope.get("head"), scope.get("event"), scope.get("base")) != (head, event, base):
        raise ValueError("Scope plan does not match the exact event, head and base")
    scopes, images = scope.get("scopes"), scope.get("image_targets")
    if (
        not isinstance(scopes, list) or not all(isinstance(value, str) for value in scopes)
        or len(set(scopes)) != len(scopes) or not set(scopes) <= ALL_SCOPES
        or not isinstance(images, list) or not all(isinstance(value, str) for value in images)
        or len(set(images)) != len(images) or not set(images) <= ALL_IMAGES
    ):
        raise ValueError("Scope plan contains unknown or duplicate scopes/image targets")
    if bool(images) != ("images" in scopes):
        raise ValueError("Image targets and the images verification scope must agree")
    if type(scope.get("full")) is not bool or scope.get("image_arches") != ["amd64", "arm64"]:
        raise ValueError("Scope plan requires an explicit full flag and both native Linux image architectures")
    if scope.get("components") not in {"auto", "native", "backend", "all"}:
        raise ValueError("Scope plan contains an unknown component selection")
    return {
        "native": "desktop" in scopes,
        "backend": bool(set(images) & BACKEND_IMAGES),
        "web": "web" in images,
        "image_targets": sorted(images),
    }


def release_coverage(root, head, published):
    """Find an earlier artifact source; canonical version tags prove no binaries.

    Exclude current-HEAD tags so retrying a partially published run retains its
    original range rather than silently turning into an empty successful plan.
    """
    candidates = [entry for entry in published if entry[3] is not None and entry[1] != head]
    if not candidates:
        return ""
    return min(candidates, key=lambda entry: (
        int(git(root, "rev-list", "--count", f"{entry[1]}..{head}")), entry[2],
    ))[1]


def plan_release(root, *, event, ref, head, base, scope):
    if event not in {"push", "pull_request", "workflow_dispatch", "schedule"}:
        raise ValueError("Unknown release event")
    if not isinstance(head, str) or not SHA.fullmatch(head) or head == "0" * 40:
        raise ValueError("Release head must be an exact nonzero Git SHA")
    if not isinstance(base, str) or (base and not SHA.fullmatch(base)):
        raise ValueError("Release base must be empty or an exact Git SHA")
    if not isinstance(ref, str) or not re.fullmatch(r"refs/(heads|tags|pull)/[A-Za-z0-9._/-]+", ref):
        raise ValueError("Release ref must be an explicit safe Git ref")
    if git(root, "rev-parse", "--verify", "HEAD") != head:
        raise ValueError("Release source does not match the checked-out head")
    selected = validate_scope(scope, event, base, head)
    scope = dict(scope)
    result = {
        "format": 1, "head": head, "base": base, "ref": ref, "event": event,
        "release_base": "", "scope": scope,
        "publish": False, "channel": "", "version": "", "tag": "", "image_tag": "",
        "prerelease": False, "untyped_commits": 0, **selected,
    }
    branch = ref.removeprefix("refs/heads/") if ref.startswith("refs/heads/") else ""
    if event != "push" or branch not in BRANCHES:
        return result
    if base and base != "0" * 40:
        if not SHA.fullmatch(base):
            raise ValueError("Push base must be an exact Git SHA")
        if git(root, "merge-base", base, head) != base:
            raise ValueError("Push base must be a fetched ancestor; force-push publication is refused")

    workspace = core_version((root / "version.txt").read_text(encoding="utf-8").strip())
    published = published_ancestors(root, head, BRANCHES[branch])
    release_base = release_coverage(root, head, published)
    pending_scopes, pending_images, _reason = select_plan("push", release_base, head, root=root)
    scopes = set(scope["scopes"]) | set(pending_scopes)
    images = set(scope["image_targets"]) | set(pending_images)
    scope.update(scopes=sorted(scopes), image_targets=sorted(images),
                 image_arches=["amd64", "arm64"], release_base=release_base)
    selected = validate_scope(scope, event, base, head)
    result.update(selected, scope=scope, release_base=release_base)
    if not any(selected[key] for key in ("native", "backend", "web")):
        return result
    count = int(git(root, "rev-list", "--count", head))
    exact = [entry for entry in published if entry[1] == head and entry[3] == branch]
    if len(exact) > 1:
        raise ValueError("Multiple artifact versions already identify this branch/source")
    if exact:
        version, _commit, existing_tag, _branch = exact[0]
        if version < workspace:
            raise ValueError("Published artifact predates the checked-out workspace baseline")
        identity = artifact_identity(branch, version, head, count)
        if identity["tag"] != existing_tag:
            raise ValueError("Published artifact tag differs from deterministic source identity")
        return {**result, **identity, "publish": True, "impact": "reused", "commits": 0}

    baseline = max([workspace, *(entry[0] for entry in published)])
    exclusions = sorted({entry[1] for entry in published})
    # A fresh repository can have no tags. Its configured bootstrap boundary
    # prevents replaying the entire pre-release history into the first artifact.
    if not exclusions:
        config_path = root / "release-please-config.json"
        if config_path.is_file():
            bootstrap = json.loads(config_path.read_text(encoding="utf-8")).get("bootstrap-sha")
            if bootstrap:
                if not isinstance(bootstrap, str) or not SHA.fullmatch(bootstrap) or git(root, "merge-base", bootstrap, head) != bootstrap:
                    raise ValueError("Release bootstrap must be a fetched ancestor of the selected source")
                exclusions.append(bootstrap)
    arguments = ["log", "--max-count=10001", "--format=%P%x00%B%x00", head]
    if exclusions:
        arguments.extend(["--not", *exclusions])
    arguments.append("--")
    fields = git(root, *arguments).split("\0")
    messages = [(fields[index + 1].strip(), len(fields[index].split()) > 1)
                for index in range(0, len(fields) - 1, 2)]
    if len(messages) > 10000:
        raise ValueError("Unreleased commit range exceeds the bounded release history window")
    details = [_commit_details(message, baseline, merge=merge, legacy=True) for message, merge in messages]
    impacts = {impact for impact, _untyped in details}
    impact = next((value for value in ("major", "minor", "patch") if value in impacts), "none")
    version = max(workspace, bump(baseline, impact))
    identity = artifact_identity(branch, version, head, count)
    if len(identity["image_tag"]) > 128:
        raise ValueError("Calculated image tag exceeds the Docker tag limit")
    return {**result, **identity, "publish": True, "impact": impact, "commits": len(messages),
            "untyped_commits": sum(untyped for _impact, untyped in details)}


def write_outputs(plan, output):
    with output.open("a", encoding="utf-8") as destination:
        for key, value in github_plan_outputs(plan["scope"]["scopes"], plan["image_targets"], full=plan["scope"]["full"]).items():
            print(f"{key}={value}", file=destination)
        for key in ("publish", "head", "base", "release_base", "version", "tag", "image_tag", "channel", "prerelease", "native", "backend", "untyped_commits"):
            value = plan[key]
            print(f"{key}={str(value).lower() if isinstance(value, bool) else value}", file=destination)
        print("release_web=" + str(plan["web"]).lower(), file=destination)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan", help="calculate exact artifact identity; never mutate refs or versions")
    plan.add_argument("--event", choices=("push", "pull_request", "workflow_dispatch", "schedule"), required=True)
    plan.add_argument("--ref", required=True)
    plan.add_argument("--head", required=True)
    plan.add_argument("--base", default="")
    plan.add_argument("--scope-plan", type=Path, required=True)
    plan.add_argument("--json-output", type=Path)
    plan.add_argument("--scope-output", type=Path, help="write the authoritative widened verification scope plan")
    plan.add_argument("--github-output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = plan_release(
            ROOT, event=args.event, ref=args.ref, head=args.head, base=args.base,
            scope=json.loads(args.scope_plan.read_text(encoding="utf-8")),
        )
        encoded = json.dumps(result, sort_keys=True, indent=2) + "\n"
        if args.json_output:
            args.json_output.write_text(encoded, encoding="utf-8")
        if args.scope_output:
            args.scope_output.write_text(json.dumps(result["scope"], sort_keys=True, indent=2) + "\n", encoding="utf-8")
        if args.github_output:
            write_outputs(result, args.github_output)
        print(encoded, end="")
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.exit(1, f"fleetworks-release: {error}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
