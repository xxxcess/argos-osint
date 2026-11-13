#!/usr/bin/env python3
"""Build native images once, smoke their exact IDs, then stage immutable digests."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import fleetworks_candidate_images as images

ROLES = frozenset({"runtime", "surrealdb", "search-index", "web"})


def run(*command, capture=False, env=None):
    result = subprocess.run(command, cwd=ROOT, env=env, check=True,
                            stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else None


def build(args):
    selected = set(args.targets.split(","))
    if not selected or not selected <= ROLES:
        raise ValueError("Unsupported image selection")
    if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
        raise ValueError("Exact source revision required")
    if run("git", "rev-parse", "HEAD", capture=True).decode().strip() != args.revision:
        raise ValueError("Checkout differs from selected source")
    if not re.fullmatch(r"ghcr\.io/[a-z0-9_.-]+", args.registry):
        raise ValueError("Repository-owner GHCR namespace required")
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,110}", args.image_tag):
        raise ValueError("Invalid immutable image tag")
    target = {"amd64": "x86_64-unknown-linux-gnu", "arm64": "aarch64-unknown-linux-gnu"}[args.arch]
    platform = images.verify_daemon(target)
    web = "web" in selected
    # Compose verifies the complete runtime dependency closure, while only
    # source-selected images are published. No image tar archive is produced.
    compose = args.compose or bool(selected - {"web"})
    closure = ({"runtime", "surrealdb", "search-index"} if compose else set()) | ({"web"} if web else set())
    tags = {role: f"{args.registry}/fleetworks-{role}:{args.image_tag}-{args.arch}"
            for role in closure}
    reused = set()
    if args.push:
        for role in sorted(selected):
            probe = subprocess.run(["docker", "buildx", "imagetools", "inspect", "--raw", tags[role]],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if probe.returncode == 0:
                # Retry uses the original immutable bytes, then repeats smoke.
                run("docker", "pull", "--platform", platform, tags[role])
                reused.add(role)
            elif not any(marker in probe.stderr.lower() for marker in (b"not found", b"manifest unknown", b"name unknown")):
                raise ValueError("Cannot establish registry tag absence safely")
    pending = closure - reused
    settings = [part for role, tag in sorted(tags.items()) if role in pending
                for part in ("--set", f"{role}.tags={tag}")]
    if pending:
        run("docker", "buildx", "bake", "-f", "deploy/docker-bake.hcl", "--load",
            "--set", f"*.platforms={platform}", *settings, *sorted(pending),
            env={**os.environ, "VERSION": args.version})
    inspected = images.inspect_images(tags, platform)
    args.directory.mkdir(parents=True, exist_ok=True)
    if compose:
        overrides = {name: tags[role] for role, name in images.image_environment(web).items()}
        expected = images.service_image_ids(overrides, inspected, platform, web=web)
        ids = args.directory / f"service-image-ids-{args.arch}.json"
        ids.write_text(json.dumps(expected, sort_keys=True) + "\n")
        run(sys.executable, "deploy/verify.py", "--name",
            f"images-{os.environ['GITHUB_RUN_ID']}-{os.environ['GITHUB_RUN_ATTEMPT']}-{args.arch}",
            "--platform", platform, *(["--web"] if web else []), "--atlas", "--source-journey",
            "--no-build", "--image-ids", str(ids), env={**images.deploy_verify.isolated_environment(os.environ), **overrides})
    else:
        # Web-only UI packaging does not compile or launch backend engines.
        run("docker", "run", "--rm", "--entrypoint", "node", tags["web"], "-e", "process.exit(0)")
    published = {}
    if args.push:
        for role in sorted(selected):
            tag = tags[role]
            if role not in reused:
                run("docker", "push", tag)
            raw = run("docker", "buildx", "imagetools", "inspect", "--raw", tag, capture=True)
            manifest = json.loads(raw)
            if manifest.get("config", {}).get("digest") != inspected[role]["id"]:
                raise ValueError("Registry image configuration differs from smoked image ID")
            repository = tag.rsplit(":", 1)[0]
            remote = json.loads(run("docker", "image", "inspect", "--format", "{{json .RepoDigests}}", tag, capture=True))
            matches = [value.split("@", 1)[1] for value in remote if value.startswith(repository + "@")]
            if len(matches) != 1 or not re.fullmatch(r"sha256:[0-9a-f]{64}", matches[0]):
                raise ValueError("Registry push did not produce one exact repository digest")
            digest = matches[0]
            # Verify addressing by digest resolves to exactly the checked bytes.
            if run("docker", "buildx", "imagetools", "inspect", "--raw", f"{repository}@{digest}", capture=True) != raw:
                raise ValueError("Registry digest verification failed")
            published[role] = {"repository": repository, "digest": digest, "image_id": inspected[role]["id"]}
    result = {"revision": args.revision, "version": args.version, "image_tag": args.image_tag,
              "arch": args.arch, "selected": sorted(selected), "images": published}
    (args.directory / f"release-images-{args.arch}.json").write_text(json.dumps(result, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--targets", required=True)
    parser.add_argument("--arch", choices=("amd64", "arm64"), required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--image-tag", required=True)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--push", action="store_true")
    parser.add_argument("--compose", action="store_true", help="also verify the complete backend closure for relay/deployment changes")
    build(parser.parse_args())


if __name__ == "__main__":
    main()
