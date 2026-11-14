#!/usr/bin/env python3
"""Export native Docker release candidates and smoke only the loaded image IDs."""

import argparse
import gzip
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from fleetworks_candidate import ROOT, checksum_line, sha256
from fleetworks_platform import detect

sys.path.insert(0, str(ROOT / "deploy"))
import native_review
import verify as deploy_verify

ARCHITECTURES = {"x86_64-unknown-linux-gnu": "amd64", "aarch64-unknown-linux-gnu": "arm64"}
IMAGE_ENV = {
    "runtime": "FLEETWORKS_RUNTIME_IMAGE",
    "surrealdb": "FLEETWORKS_SURREAL_IMAGE",
    "search-index": "FLEETWORKS_SEARCH_IMAGE",
}
MAX_ARCHIVE_BYTES = 1024 ** 3
MAX_IMAGE_BYTES = 3 * MAX_ARCHIVE_BYTES
IMAGE_ID = re.compile(r"sha256:[0-9a-f]{64}")


def docker(*args, env=None, output=False):
    result = subprocess.run(["docker", *map(str, args)], cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE if output else None, check=True)
    return result.stdout if output else None


def verify_daemon(target):
    host = detect()
    platform = f"linux/{ARCHITECTURES[target]}"
    if host.system != "Linux" or host.docker_platform != platform or host.rosetta:
        raise ValueError("Image candidates require the matching physical Linux host")
    if docker("version", "--format", "{{.Server.Os}}/{{.Server.Arch}}", output=True).strip() != platform:
        raise ValueError(f"Docker daemon is not native for {platform}")
    return platform


def image_environment(web=False):
    return {**IMAGE_ENV, **({"web": "FLEETWORKS_WEB_IMAGE"} if web else {})}


def image_tags(revision, platform, *, web=False):
    arch = platform.split("/", 1)[1]
    return {role: f"fleetworks-candidate-{role}:{revision}-{arch}" for role in image_environment(web)}


def compose_images(overrides, platform="linux/amd64", *, web=False):
    with tempfile.TemporaryDirectory(prefix="fleetworks-image-config-") as temporary:
        receipt = native_review.provision(Path(temporary), "image-config", 18091, platform,
                                          "0" * 40, web_port=18092 if web else None, images=overrides)
        workspace = Path(receipt["workspace"])
        environment = {**deploy_verify.isolated_environment(os.environ), **receipt["environment"]}
        model = json.loads(docker(
            *deploy_verify.compose_command(workspace, web=web, atlas=True)[1:],
            "config", "--format", "json", env=environment, output=True,
        ))
    return {service: item["image"] for service, item in model["services"].items()}


def inspect_images(tags, platform):
    result = {}
    for role, tag in tags.items():
        images = json.loads(docker("image", "inspect", tag, output=True))
        if len(images) != 1:
            raise ValueError(f"Expected one Docker image for {tag}")
        [image] = images
        image_id = image["Id"]
        if not IMAGE_ID.fullmatch(image_id) or f'{image["Os"]}/{image["Architecture"]}' != platform:
            raise ValueError(f"{tag} does not have a valid {platform} image identity")
        result[role] = {"tag": tag, "id": image_id, "bytes": image["Size"]}
    if len({image["id"] for image in result.values()}) != len(tags):
        raise ValueError("Candidate images must have distinct identities")
    if not all(isinstance(image["bytes"], int) and image["bytes"] > 0 for image in result.values()):
        raise ValueError("Candidate image sizes must be positive")
    if sum(image["bytes"] for image in result.values()) > MAX_IMAGE_BYTES:
        raise ValueError("Candidate images exceed the uncompressed size limit")
    return result


def save_images(archive, tags):
    with archive.open("xb") as output:
        process = None
        try:
            process = subprocess.Popen(["docker", "image", "save", *tags.values()], cwd=ROOT, stdout=subprocess.PIPE)
            with process.stdout, gzip.GzipFile(filename="", fileobj=output, mode="wb", compresslevel=1, mtime=0) as compressed:
                raw_bytes = 0
                for chunk in iter(lambda: process.stdout.read(1024 * 1024), b""):
                    raw_bytes += len(chunk)
                    if raw_bytes > MAX_IMAGE_BYTES:
                        raise ValueError("Docker archive exceeds the uncompressed size limit")
                    compressed.write(chunk)
                    if archive.stat().st_size > MAX_ARCHIVE_BYTES:
                        raise ValueError("Docker archive exceeds the compressed size limit")
            if process.wait():
                raise ValueError("Docker failed to export the candidate images")
        except BaseException:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
            archive.unlink(missing_ok=True)
            raise
    if archive.stat().st_size > MAX_ARCHIVE_BYTES:
        archive.unlink()
        raise ValueError("Docker archive exceeds the compressed size limit")


def package(directory, revision, target, *, web=False):
    platform = verify_daemon(target)
    tags = image_tags(revision, platform, web=web)
    for role in image_environment(web):
        docker("build", "--platform", platform, "--file", ROOT / "deploy/Dockerfile", "--target", role, "--tag", tags[role], ROOT)
    images = inspect_images(tags, platform)
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / "images.tar.gz"
    save_images(archive, tags)
    archive_digest = sha256(archive)
    archive.with_suffix(".gz.sha256").write_text(checksum_line(archive, archive_digest), encoding="utf-8")
    manifest = {
        "format": 1, "revision": revision, "target": target, "platform": platform,
        "archive": {"sha256": archive_digest, "bytes": archive.stat().st_size},
        "images": images,
    }
    path = directory / "images.json"
    path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return path


def read_manifest(directory, expected_digest, revision, target, platform):
    path = directory / "images.json"
    if sha256(path) != expected_digest:
        raise ValueError("Image manifest does not match the packaging job's SHA-256")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if (manifest["format"], manifest["revision"], manifest["target"], manifest["platform"]) != (1, revision, target, platform):
        raise ValueError("Image manifest does not match the requested commit and platform")
    archive = directory / "images.tar.gz"
    size = archive.stat().st_size
    if not 0 < size <= MAX_ARCHIVE_BYTES or size != manifest["archive"]["bytes"]:
        raise ValueError("Image archive size differs from the manifest or exceeds the limit")
    actual_digest = sha256(archive)
    if actual_digest != manifest["archive"]["sha256"] or archive.with_suffix(".gz.sha256").read_text(encoding="utf-8") != checksum_line(archive, actual_digest):
        raise ValueError("Image archive does not match the expected SHA-256")
    return manifest, archive


def service_image_ids(overrides, images, platform="linux/amd64", *, web=False):
    by_tag = {item["tag"]: item["id"] for item in images.values()}
    services = compose_images(overrides, platform, web=web)
    if set(services.values()) != by_tag.keys():
        raise ValueError("Compose services do not use exactly the archived image tags")
    if services.get("native-worker") != images["runtime"]["tag"]:
        raise ValueError("Registered native worker must use the archived runtime image")
    return {service: by_tag[tag] for service, tag in services.items()}


def verify(directory, manifest_digest, revision, target, *, web=False):
    platform = verify_daemon(target)
    manifest, archive = read_manifest(directory, manifest_digest, revision, target, platform)
    tags = image_tags(revision, platform, web=web)
    expected = manifest["images"]
    if expected.keys() != tags.keys() or any(expected[role]["tag"] != tag for role, tag in tags.items()):
        raise ValueError("Manifest contains unexpected candidate image tags")
    docker("image", "load", "--input", archive)
    loaded = inspect_images(tags, platform)
    if any(loaded[role]["id"] != expected[role]["id"] for role in tags):
        raise ValueError("Loaded image IDs differ from the packaging job's image IDs")
    overrides = {name: tags[role] for role, name in image_environment(web).items()}
    expected_ids = service_image_ids(overrides, loaded, platform, web=web)
    with tempfile.TemporaryDirectory(prefix="fleetworks-image-smoke-") as temporary:
        id_file = Path(temporary) / "service-image-ids.json"
        id_file.write_text(json.dumps(expected_ids), encoding="utf-8")
        name = "candidate-" + revision[:12] + "-" + os.urandom(4).hex()
        subprocess.run([
            sys.executable, str(ROOT / "deploy/verify.py"), "--name", name, "--platform", platform,
            *(["--web"] if web else []), "--atlas", "--source-journey", "--no-build", "--image-ids", str(id_file),
        ], cwd=ROOT, env={**deploy_verify.isolated_environment(os.environ), **overrides}, check=True)
    print(f"Verified downloaded Docker image IDs for {revision} on {platform}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pack = commands.add_parser("pack")
    check = commands.add_parser("verify")
    for command in (pack, check):
        command.add_argument("--directory", type=Path, required=True)
        command.add_argument("--revision", required=True)
        command.add_argument("--target", choices=ARCHITECTURES, required=True)
        command.add_argument("--web", action="store_true", help="also package/verify optional Intel Web")
    pack.add_argument("--github-output", type=Path, required=True)
    check.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
            raise ValueError("An exact Git revision is required")
        if args.command == "pack":
            manifest = package(args.directory.resolve(), args.revision, args.target, web=args.web)
            with args.github_output.open("a", encoding="utf-8") as output:
                print(f"manifest-sha256={sha256(manifest)}", file=output)
        else:
            verify(args.directory.resolve(), args.manifest_sha256, args.revision, args.target, web=args.web)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"fleetworks-candidate-images: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
