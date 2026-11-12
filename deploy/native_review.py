#!/usr/bin/env python3
"""Provision a fresh isolated backend review directory; never start Docker."""

import argparse
import json
import os
import re
import secrets
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOKEN_NAMES = (
    "native-token", "web-relay-token", "gateway-health-token", "brain-token",
    "tools-token", "control-token", "jobs-token", "execution-token",
    "internal-token", "storage-password", "storage-root-password", "search-read-token", "search-write-token",
)
NATIVE_PERMISSIONS = (
    "read_backend", "register_workstation", "rebind_workstation", "manage_configuration",
    "maintain_backend", "submit_interactive", "read_workstation_activity", "retry_job", "cancel_job",
)


def review_path(root, name):
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,31}", name):
        raise ValueError("Review name must be 1–32 lowercase letters, digits or hyphens")
    root = root.resolve(strict=True)
    path = root / ".agent-scratch" / "native-review" / name
    for item in (root / ".agent-scratch", path.parent, path):
        if item.is_symlink():
            raise ValueError("Review directories must not be symlinks")
    return path


def private_file(path, value):
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())


def expose_root_environment(root, name):
    """Keep one editable root config with the review path pointing to it."""
    root = root.resolve(strict=True)
    workspace = review_path(root, name)
    source = workspace / "compose.env"
    target = root / "compose.env"
    if target.exists() or target.is_symlink():
        raise FileExistsError("Repo-root compose.env already exists")
    if source.is_symlink() or not source.is_file():
        raise ValueError("Expected the review's generated regular env file")
    contents = source.read_text(encoding="utf-8")
    private_file(target, contents)
    try:
        source.unlink()
        source.symlink_to(os.path.relpath(target, workspace))
    except OSError:
        if not source.exists() and not source.is_symlink():
            private_file(source, contents)
        target.unlink()
        raise
    return target


def provision(root, name, api_port, platform, revision, *, web_port=None, images=None, root_env=False):
    if not 1024 <= api_port <= 65535:
        raise ValueError("Choose an unused unprivileged API port")
    if web_port is not None and (not 1024 <= web_port <= 65535 or web_port == api_port):
        raise ValueError("Choose a distinct unused unprivileged Web port")
    if platform not in {"linux/amd64", "linux/arm64"}:
        raise ValueError("Review platform must be linux/amd64 or linux/arm64")
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Review revision must be a full Git SHA")
    if images:
        allowed = {"FLEETWORKS_RUNTIME_IMAGE", "FLEETWORKS_SURREAL_IMAGE", "FLEETWORKS_SEARCH_IMAGE", "FLEETWORKS_WEB_IMAGE"}
        if set(images) - allowed or any(not re.fullmatch(r"[a-z0-9][a-z0-9._/-]*:[A-Za-z0-9._-]+", value) for value in images.values()):
            raise ValueError("Review image overrides must be explicit tagged images")
    workspace = review_path(root, name)
    if root_env and ((root / "compose.env").exists() or (root / "compose.env").is_symlink()):
        raise FileExistsError("Repo-root compose.env already exists")
    if any(character in str(workspace) for character in "'\\\r\n"):
        raise ValueError("Review repository path cannot contain quotes, backslashes or line breaks")
    workspace.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    workspace.mkdir(mode=0o700)  # Existing data/credentials are never overwritten.
    for child in ("config", "secrets", "deployment", "storage", "storage/surrealdb", "storage/jobs", "storage/search-index", "staged-datasets", "models", "models/atlas-bert", "logs"):
        (workspace / child).mkdir(mode=0o700)
    for token in TOKEN_NAMES:
        private_file(workspace / "secrets" / token, secrets.token_urlsafe(48) + "\n")
    scope = "review-" + secrets.token_hex(16)
    epoch = secrets.token_hex(24)
    source_epoch = secrets.token_hex(24)
    credentials = {
        "version": 1,
        "scope_id": scope,
        "access_epoch": epoch,
        "credentials": [
            {"id": "native-review", "purpose": "native", "permissions": list(NATIVE_PERMISSIONS), "token_file": "/run/secrets/gateway_native_token"},
            {"id": "web-relay-review", "purpose": "web_relay", "permissions": ["read_backend", "resolve_web_session", "submit_interactive", "read_workstation_activity", "retry_job", "cancel_job"], "token_file": "/run/secrets/gateway_web_token"},
            {"id": "internal-health-review", "purpose": "internal_service", "permissions": ["read_backend"], "token_file": "/run/secrets/gateway_health_token"},
        ],
    }
    private_file(workspace / "secrets/gateway-credentials.json", json.dumps(credentials, indent=2) + "\n")
    image_tag = f"native-review-{name}-{revision[:12]}"
    environment = {
        "COMPOSE_PROJECT_NAME": "fleetworks-native-review-" + name,
        "FLEETWORKS_HOST_WORKSPACE": str(workspace),
        "FLEETWORKS_UID": str(os.getuid()), "FLEETWORKS_GID": str(os.getgid()),
        "FLEETWORKS_API_PORT": str(api_port), "PLATFORM": platform,
        "FLEETWORKS_ACCESS_SCOPE_ID": scope, "FLEETWORKS_ACCESS_EPOCH": epoch,
        "FLEETWORKS_SEARCH_SOURCE_EPOCH": source_epoch,
        "FLEETWORKS_RUNTIME_IMAGE": "fleetworks-runtime:" + image_tag,
        "FLEETWORKS_SURREAL_IMAGE": "fleetworks-surrealdb:" + image_tag,
        "FLEETWORKS_SEARCH_IMAGE": "fleetworks-search-index:" + image_tag,
    }
    if web_port is not None:
        environment.update(FLEETWORKS_WEB_PORT=str(web_port),
                           FLEETWORKS_WEB_ORIGIN=f"http://127.0.0.1:{web_port}",
                           FLEETWORKS_WEB_IMAGE="fleetworks-web:" + image_tag)
    if images:
        environment.update(images)
    # Compose reads this directly; documentation never sources/evals file text.
    # Single quotes preserve literal dollar signs in the path; Compose must not
    # expand a repository directory as an environment-variable expression.
    private_file(workspace / "compose.env", "".join(f"{key}='{value}'\n" for key, value in environment.items()))
    if root_env:
        expose_root_environment(root, name)
    receipt = {"version": 1, "revision": revision, "image_tag": image_tag, "workspace": str(workspace), "environment": environment}
    private_file(workspace / "review.json", json.dumps(receipt, indent=2) + "\n")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    initialize = subcommands.add_parser("init", help="create new private files under .agent-scratch/native-review")
    initialize.add_argument("--name", required=True)
    initialize.add_argument("--api-port", type=int, required=True)
    initialize.add_argument("--web-port", type=int, help="optional distinct loopback Intel Web port")
    initialize.add_argument("--root-env", action="store_true", help="keep editable settings in repo-root compose.env; refuse an existing file")
    initialize.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    value = subcommands.add_parser("value", help="read one non-secret review setting")
    value.add_argument("--name", required=True)
    value.add_argument("--field", choices=("workspace", "image_tag", "project"), required=True)
    args = parser.parse_args()
    try:
        if args.command == "init":
            revision = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD"], cwd=ROOT, text=True).strip()
            receipt = provision(ROOT, args.name, args.api_port, args.platform, revision,
                                web_port=args.web_port, root_env=args.root_env)
            print(f"Prepared isolated backend review: {receipt['workspace']}")
            if args.root_env:
                print(f"Editable Compose settings: {ROOT / 'compose.env'}")
            print("No images built or services started. Keep compose.env and all credentials for restart.")
        else:
            receipt = json.loads((review_path(ROOT, args.name) / "review.json").read_text(encoding="utf-8"))
            print(receipt["environment"]["COMPOSE_PROJECT_NAME"] if args.field == "project" else receipt[args.field])
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Review provisioning failed: {type(error).__name__}. Existing files were retained.\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
