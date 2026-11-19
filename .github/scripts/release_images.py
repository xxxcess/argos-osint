#!/usr/bin/env python3
"""Build native images once, smoke their exact IDs, then stage immutable digests."""
import argparse
from contextlib import nullcontext
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import fleetworks_candidate_images as images
from fleetworks_image_builder import bake_command

ROLES = frozenset({"runtime", "surrealdb", "search-index", "web"})


def run(*command, capture=False, env=None):
    result = subprocess.run(command, cwd=ROOT, env=env, check=True,
                            stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else None


def bake(args, platform, tags, pending):
    settings = [part for role in sorted(pending)
                for part in ("--set", f"{role}.tags={tags[role]}")]
    if "web-dev" in pending:
        settings.extend(("--set", f"web-dev.args.FLEETWORKS_UID={os.getuid()}",
                         "--set", f"web-dev.args.FLEETWORKS_GID={os.getgid()}"))
    environment = {**os.environ, "VERSION": args.version}
    docker_config = nullcontext()
    if args.builder == "depot" and args.push:
        # BuildKit registry auth can consult the client config. Keep the
        # release write credential available only to local registry commands.
        host = environment.get("DOCKER_HOST")
        if not host or environment.get("DOCKER_CONTEXT"):
            host = run("docker", "context", "inspect", "--format", "{{.Endpoints.docker.Host}}",
                       capture=True).decode().strip()
        if not host.startswith("unix:///"):
            raise ValueError("Depot release loading requires a local Unix Docker daemon")
        environment.pop("DOCKER_CONTEXT", None)
        environment.pop("DOCKER_AUTH_CONFIG", None)
        environment["DOCKER_HOST"] = host
        docker_config = tempfile.TemporaryDirectory(prefix="fleetworks-depot-docker-")
    with docker_config as credential_free_config:
        if credential_free_config is not None:
            # An explicit anonymous entry also disables Docker's default
            # credential-store discovery on runners with a helper installed.
            config = Path(credential_free_config) / "config.json"
            config.write_text(json.dumps({"auths": {"ghcr.io": {}}}), encoding="utf-8")
            environment["DOCKER_CONFIG"] = credential_free_config
        command = bake_command("deploy/docker-bake.hcl", platform, sorted(pending), settings,
                               builder=args.builder, project=args.depot_project)
        run(*command, env=environment)


def smoke_compose(args, platform, inspected, *, web, dev=False):
    overrides = {name: inspected[role]["tag"] for role, name in images.image_environment(web).items()}
    expected = images.service_image_ids(overrides, inspected, platform, web=web)
    kind = "-dev" if dev else ""
    ids = args.directory / f"service-image-ids{kind}-{args.arch}.json"
    ids.write_text(json.dumps(expected, sort_keys=True) + "\n")
    run(sys.executable, "deploy/verify.py", "--name",
        f"images{kind}-{os.environ['GITHUB_RUN_ID']}-{os.environ['GITHUB_RUN_ATTEMPT']}-{args.arch}",
        "--platform", platform, *(["--web"] if web else []), "--atlas",
        *(["--dev"] if dev else ["--source-journey"]), "--no-build", "--image-ids", str(ids),
        env={**images.deploy_verify.isolated_environment(os.environ), **overrides})


def build(args):
    selected = set(args.targets.split(","))
    if args.push and args.dev_smoke:
        raise ValueError("Development Web smoke is available for review builds only")
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
    # source-selected images are published and exported for release assets.
    compose = args.compose or args.dev_smoke or bool(selected - {"web"})
    closure = ({"runtime", "surrealdb", "search-index"} if compose else set()) | ({"web"} if web else set())
    build_roles = closure | ({"web-dev"} if args.dev_smoke else set())
    tags = {role: f"{args.registry}/fleetworks-{role}:{args.image_tag}-{args.arch}"
            for role in build_roles}
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
    pending = build_roles - reused
    started = time.monotonic()
    if pending:
        bake(args, platform, tags, pending)
    smoke_started = time.monotonic()
    inspected = images.inspect_images({role: tags[role] for role in closure}, platform)
    args.directory.mkdir(parents=True, exist_ok=True)
    if compose:
        smoke_compose(args, platform, inspected, web=web)
    else:
        # Web-only UI packaging does not compile or launch backend engines.
        run("docker", "run", "--rm", "--entrypoint", "node", tags["web"], "-e", "process.exit(0)")
    if args.dev_smoke:
        development_web = images.inspect_images({"web": tags["web-dev"]}, platform)
        smoke_compose(args, platform, {**inspected, **development_web}, web=True, dev=True)
    smoke_finished = time.monotonic()
    published = {}
    archive_info = None
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
        archive = args.directory / f"fleetworks-images-linux-{args.arch}.tar.gz"
        archive.unlink(missing_ok=True)
        images.save_images(archive, {role: tags[role] for role in sorted(selected)})
        archive_digest = images.sha256(archive)
        archive.with_name(archive.name + ".sha256").write_text(
            images.checksum_line(archive, archive_digest), encoding="utf-8"
        )
        archive_info = {"name": archive.name, "sha256": archive_digest, "bytes": archive.stat().st_size}
    result = {"revision": args.revision, "version": args.version, "image_tag": args.image_tag,
              "arch": args.arch, "selected": sorted(selected), "images": published}
    if not args.push:
        result["builder"] = args.builder
        result["timings_seconds"] = {
            "bake_and_load": round(smoke_started - started, 3),
            "smoke": round(smoke_finished - smoke_started, 3),
        }
    if archive_info is not None:
        result["archive"] = archive_info
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
    parser.add_argument("--builder", choices=("buildx", "depot"), default="buildx")
    parser.add_argument("--depot-project", help="Depot project ID for --builder depot")
    parser.add_argument("--push", action="store_true")
    parser.add_argument("--compose", action="store_true", help="also verify the complete backend closure for relay/deployment changes")
    parser.add_argument("--dev-smoke", action="store_true", help="also verify the development Web overlay in a review build")
    args = parser.parse_args()
    if args.builder == "depot" and not args.depot_project:
        parser.error("--builder depot requires --depot-project")
    if args.push and args.dev_smoke:
        parser.error("--dev-smoke cannot be combined with --push")
    build(args)


if __name__ == "__main__":
    main()
