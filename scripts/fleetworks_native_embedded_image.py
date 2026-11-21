#!/usr/bin/env python3
"""Build a Linux x86_64 development image from a verified Fleetworks native archive."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

from fleetworks_candidate import ROOT, artifact_version, checksum_line, extract_native, locked_rune_revision, sha256
from fleetworks_candidate_images import inspect_images, save_images, verify_daemon
from fleetworks_image_builder import bake_command

TARGET = "x86_64-unknown-linux-gnu"
IMAGE = "native-embedded"
DOCKERFILE = "Dockerfile.native-embedded"
BAKEFILE = "native-embedded-bake.hcl"
SMOKE_SCRIPT = ROOT / "scripts/fleetworks_local_backend_smoke.py"
LOCAL_TAG = re.compile(r"[a-z0-9]+(?:[._/-][a-z0-9]+)*:[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")

CONTAINER_SMOKE = r"""
test "$(id -u):$(id -g)" = "10001:10001"
for binary in /opt/fleetworks/bin/fleetworks-desktop /opt/fleetworks/bin/argos-api-host; do
    dependencies="$(ldd "$binary")"
    if printf '%s\n' "$dependencies" | grep 'not found' >&2; then
        printf 'Missing shared libraries for %s\n' "$binary" >&2
        exit 1
    fi
done
exec python3 /tmp/fleetworks-local-backend-smoke.py --app /opt/fleetworks --skip-rust-probe
"""


def smoke_image(image_id):
    """Exercise the bundled companion and check app linkage in the exact loaded image."""
    with tempfile.TemporaryDirectory(prefix="fleetworks-native-image-smoke-") as directory:
        cidfile = Path(directory) / "container-id"
        command = [
            "docker", "run", "--rm", "--cidfile", str(cidfile), "--network", "none", "--pull", "never",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--mount", f"type=bind,src={SMOKE_SCRIPT},dst=/tmp/fleetworks-local-backend-smoke.py,readonly",
            "--entrypoint", "/bin/sh", image_id, "-eu", "-c", CONTAINER_SMOKE,
        ]
        try:
            result = subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, timeout=360)
        except subprocess.TimeoutExpired:
            if cidfile.is_file():
                container_id = cidfile.read_text(encoding="ascii").strip()
                if re.fullmatch(r"[0-9a-f]{64}", container_id):
                    subprocess.run(["docker", "rm", "--force", container_id], check=False, timeout=30)
            raise
    evidence = json.loads(result.stdout)
    if evidence.get("status") != "pass" or evidence.get("socket_check_method") != "procfs" or evidence.get("platform") != {
        "system": "Linux", "machine": "x86_64",
    }:
        raise ValueError("Native Embedded image smoke did not verify Linux x86_64 companion IPC")
    return evidence


def export_image(output, images, platform):
    """Write the immutable image archive and checksum after smoke succeeds."""
    output.parent.mkdir(parents=True, exist_ok=True)
    sidecar = output.with_name(output.name + ".sha256")
    tags = {role: item["tag"] for role, item in images.items()}
    with tempfile.TemporaryDirectory(prefix=".fleetworks-native-image-", dir=output.parent) as directory:
        staged = Path(directory) / output.name
        staged_sidecar = Path(directory) / sidecar.name
        if inspect_images(tags, platform) != images:
            raise ValueError("The loaded development-image tag changed after smoke")
        save_images(staged, tags)
        if inspect_images(tags, platform) != images:
            raise ValueError("The loaded development-image tag changed during export")
        digest = sha256(staged)
        staged_sidecar.write_text(checksum_line(output, digest), encoding="utf-8")
        os.link(staged, output)
        try:
            os.link(staged_sidecar, sidecar)
        except OSError:
            output.unlink()
            raise
    return {"path": str(output), "sha256": digest, "checksum": str(sidecar)}


def package(args):
    version = artifact_version(args.version)
    if len(args.tag) > 255 or not LOCAL_TAG.fullmatch(args.tag):
        raise ValueError("Use a bounded local Docker image name with an explicit tag")
    if args.output is not None:
        if not args.output.name.endswith(".tar.gz"):
            raise ValueError("Native Embedded image output must end in .tar.gz")
        sidecar = args.output.with_name(args.output.name + ".sha256")
        if any(path.exists() or path.is_symlink() for path in (args.output, sidecar)):
            raise ValueError("Native Embedded image output or checksum already exists")

    platform = verify_daemon(TARGET)
    with tempfile.TemporaryDirectory(prefix="fleetworks-native-image-context-") as directory:
        context = Path(directory)
        _, manifest = extract_native(
            args.archive, context, expected_digest=args.sha256, version=version, target=TARGET,
            revision=args.revision, rune_revision=locked_rune_revision(), profile=args.profile,
            require_clean=True,
        )
        if not manifest.get("companion"):
            raise ValueError("Native Embedded image requires a verified bundled companion")
        for filename in (DOCKERFILE, BAKEFILE):
            shutil.copyfile(ROOT / "deploy" / filename, context / filename)

        settings = []
        for key, value in (
            ("tags", args.tag),
            ("args.NATIVE_REVISION", args.revision),
            ("args.NATIVE_VERSION", version),
            ("args.NATIVE_PROFILE", args.profile),
            ("args.NATIVE_ARCHIVE_SHA256", args.sha256),
        ):
            settings.extend(("--set", f"{IMAGE}.{key}={value}"))
        command = bake_command(
            BAKEFILE, platform, [IMAGE], settings, builder=args.builder, project=args.depot_project,
        )
        subprocess.run(command, cwd=context, check=True, stdout=sys.stderr, timeout=3600)

    images = inspect_images({IMAGE: args.tag}, platform)
    evidence = smoke_image(images[IMAGE]["id"])
    return {
        "revision": args.revision,
        "version": version,
        "profile": args.profile,
        "platform": platform,
        "builder": args.builder,
        "native_archive_sha256": args.sha256,
        "image": images[IMAGE],
        "archive": export_image(args.output, images, platform) if args.output is not None else None,
        "companion_ipc": evidence,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="Verified native archive with adjacent SHA-256 sidecar")
    parser.add_argument("--sha256", required=True, help="Expected native archive SHA-256 from the packaging job")
    parser.add_argument("--revision", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--profile", choices=("debug", "release"), required=True)
    parser.add_argument("--builder", choices=("buildx", "depot"), default="buildx")
    parser.add_argument("--depot-project", help="Depot project ID for --builder depot")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, help="Optional immutable image .tar.gz output with SHA-256 sidecar")
    args = parser.parse_args()
    if args.builder == "depot" and not args.depot_project:
        parser.error("--builder depot requires --depot-project")
    try:
        print(json.dumps(package(args), indent=2))
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(f"fleetworks-native-embedded-image: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
