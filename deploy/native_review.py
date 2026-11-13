#!/usr/bin/env python3
"""Provision, build, and run local Fleetworks backend reviews and supported native builds."""

import argparse
import importlib.util
import json
import shlex
import os
import re
import secrets
import stat
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REVIEW_NAME = "native-local"
DEFAULT_API_PORT = 18091
TOKEN_NAMES = (
    "native-token", "operator-token", "web-relay-token", "gateway-health-token", "brain-token",
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


def read_private(path, limit=65536):
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
                         | getattr(os, "O_NONBLOCK", 0))
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or info.st_uid != os.getuid() or info.st_mode & 0o077
                or info.st_size > limit):
            raise ValueError("Expected a bounded private owned regular file")
        data = source.read(limit + 1)
        if len(data) > limit:
            raise ValueError("Private file exceeds its bound")
        return data.decode("utf-8")


@contextmanager
def preparation_lock(workspace, filename=".prepare.lock", *, wait=True):
    import fcntl

    for directory in (workspace, workspace / "secrets"):
        info = directory.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
                or info.st_mode & 0o077):
            raise ValueError("Expected a private owned review directory")
    descriptor = os.open(workspace / filename,
                         os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
    with os.fdopen(descriptor, "r+") as lock:
        info = os.fstat(lock.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or info.st_uid != os.getuid() or info.st_mode & 0o077):
            raise ValueError("Unsafe preparation lock")
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | (0 if wait else fcntl.LOCK_NB))
        except BlockingIOError:
            raise ValueError("Another update is already running for this review") from None
        yield


def current_environment(root, workspace):
    """Read generated Compose settings as data, including the supported root alias."""
    env_file = workspace / "compose.env"
    if env_file.is_symlink():
        target = root / "compose.env"
        if target.is_symlink() or env_file.resolve(strict=True) != target:
            raise ValueError("Unsupported Compose environment alias")
        contents = read_private(target)
    else:
        contents = read_private(env_file)
    allowed = {
        "COMPOSE_PROJECT_NAME", "FLEETWORKS_HOST_WORKSPACE", "PLATFORM",
        "FLEETWORKS_API_PORT", "FLEETWORKS_ACCESS_SCOPE_ID", "FLEETWORKS_ACCESS_EPOCH",
        "FLEETWORKS_RUNTIME_IMAGE", "FLEETWORKS_SURREAL_IMAGE", "FLEETWORKS_SEARCH_IMAGE",
        "FLEETWORKS_UID", "FLEETWORKS_GID", "FLEETWORKS_SEARCH_SOURCE_EPOCH",
    }
    settings = {}
    for line in contents.splitlines():
        key, separator, value = line.partition("=")
        key = key.strip()
        if not separator or key not in allowed:
            continue
        parts = shlex.split(value, comments=True)
        if key in settings or len(parts) != 1:
            raise ValueError("Ambiguous Compose setting")
        settings[key] = parts[0]
    if set(settings) != allowed:
        raise ValueError("Missing review Compose settings")
    return env_file, settings


def prepare(root, name):
    """Add only missing Operator authority, then resolve reusable command inputs."""
    if os.name != "posix":
        raise ValueError("Preparation requires POSIX private-file protections")
    root = root.resolve(strict=True)
    workspace = review_path(root, name)
    with preparation_lock(workspace):
        receipt = json.loads(read_private(workspace / "review.json"))
        env_file, environment = current_environment(root, workspace)
        if (receipt["version"] != 1 or Path(environment["FLEETWORKS_HOST_WORKSPACE"]) != workspace
                or receipt["workspace"] != str(workspace)
                or environment["COMPOSE_PROJECT_NAME"] != receipt["environment"]["COMPOSE_PROJECT_NAME"]):
            raise ValueError("Review identity differs from its recorded deployment")
        if environment["PLATFORM"] not in {"linux/amd64", "linux/arm64"}:
            raise ValueError("Unsupported review platform")
        port = int(environment["FLEETWORKS_API_PORT"])
        if not 1024 <= port <= 65535:
            raise ValueError("Invalid review API port")
        for key in ("FLEETWORKS_RUNTIME_IMAGE", "FLEETWORKS_SURREAL_IMAGE", "FLEETWORKS_SEARCH_IMAGE"):
            if not re.fullmatch(r"[a-z0-9][a-z0-9._/-]*:[A-Za-z0-9._-]+", environment[key]):
                raise ValueError("Review images must have explicit tags")
        version = (root / "version.txt").read_text().strip()
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.+-]+)?", version):
            raise ValueError("Invalid workspace version")
        manifest = workspace / "secrets/gateway-credentials.json"
        data = json.loads(read_private(manifest))
        if (data["version"] != 1 or data["scope_id"] != environment["FLEETWORKS_ACCESS_SCOPE_ID"]
                or data["access_epoch"] != environment["FLEETWORKS_ACCESS_EPOCH"]):
            raise ValueError("Gateway authority differs from Compose settings")
        entries = data["credentials"]
        if (not isinstance(entries, list) or any(not isinstance(entry, dict) for entry in entries)
                or len({entry["id"] for entry in entries}) != len(entries)):
            raise ValueError("Invalid credential catalog")
        operators = [entry for entry in entries if entry["purpose"] == "operator"]
        token = workspace / "secrets/operator-token"
        container_token = "/run/secrets/gateway_operator_token"
        if operators:
            if (len(operators) != 1 or operators[0]["token_file"] != container_token
                    or not {"read_backend", "register_workstation", "manage_configuration"}.issubset(operators[0]["permissions"])):
                raise ValueError("Existing Operator configuration needs explicit operator review")
        elif any(entry["id"] == "operator-review" for entry in entries):
            raise ValueError("Operator identifier already belongs to another purpose")
        # Never relabel another purpose's file, or rotate an existing authority.
        if any(entry["purpose"] != "operator" and entry["token_file"] == container_token for entry in entries):
            raise ValueError("Operator token file is shared with another purpose")
        existing = [read_private(workspace / "secrets" / filename, 4096).strip()
                    for filename in ("native-token", "web-relay-token", "gateway-health-token")]
        if not operators and not token.exists() and not token.is_symlink():
            private_file(token, secrets.token_urlsafe(48) + "\n")
        operator_secret = read_private(token, 4096).strip()
        if (not 32 <= len(operator_secret) <= 4096
                or any(secrets.compare_digest(operator_secret, value) for value in existing)):
            raise ValueError("Operator authority must use its own private credential")
        if not operators:
            entries.append({"id": "operator-review", "purpose": "operator",
                            "permissions": list(NATIVE_PERMISSIONS), "token_file": container_token})
            temporary = manifest.with_name(".gateway-" + secrets.token_hex(8) + ".tmp")
            try:
                private_file(temporary, json.dumps(data, indent=2) + "\n")
                os.replace(temporary, manifest)
            finally:
                temporary.unlink(missing_ok=True)
        return {"workspace": workspace, "env_file": env_file, "environment": environment,
                "version": version, "operator": token, "origin": f"http://127.0.0.1:{port}"}


def review_commands(root, prepared):
    environment = prepared["environment"]
    # Compose must read this review's file even after another shell exported a
    # different review's settings. Names, not authority values, reach output.
    compose = ["env"]
    for key in sorted(set(environment) | {"FLEETWORKS_UID", "FLEETWORKS_GID", "FLEETWORKS_SEARCH_SOURCE_EPOCH"}):
        compose.extend(["-u", key])
    compose += ["docker", "compose", "--env-file", str(prepared["env_file"]),
               "-f", str(root / "deploy/compose.native-review.yml"),
               "--project-name", environment["COMPOSE_PROJECT_NAME"]]
    build = ["env", "PLATFORM=" + environment["PLATFORM"], "VERSION=" + prepared["version"],
             "docker", "buildx", "bake", "-f", str(root / "deploy/docker-bake.hcl"),
             "--progress=plain", "--set", "runtime.args.FLEETWORKS_CORE_OPT_LEVEL=1"]
    for target, key in (("runtime", "FLEETWORKS_RUNTIME_IMAGE"),
                        ("surrealdb", "FLEETWORKS_SURREAL_IMAGE"),
                        ("search-index", "FLEETWORKS_SEARCH_IMAGE")):
        build.extend(["--set", target + ".tags=" + environment[key]])
    build.extend(["--load", "backend"])
    return build, compose


def print_approval(root, prepared):
    print(f"Setup Gateway → {prepared['origin']} → Pair again. Approve the displayed values:")
    print(shlex.join(["python3", str(root / "scripts/fleetworks_pairing.py"),
                     "--origin", prepared["origin"], "--operator-token-file", str(prepared["operator"]),
                     "approve", "--pairing-id", "<displayed-id>", "--approval-code", "<displayed-code>"]))


def print_prepared(root, prepared):
    build, compose = review_commands(root, prepared)
    print(f"Prepared existing review: {prepared['workspace']}")
    print("Run from the repository root; no exports or sourcing Compose settings needed.")
    print("1. Build backend images (local review optimization, visible progress):")
    print(shlex.join(build))
    print("2. After the build succeeds, restart this review; bootstrap precedes dependents:")
    print(shlex.join(compose + ["stop"]) + " && " + shlex.join(compose + [
        "up", "--no-build", "--pull", "never", "--force-recreate", "--detach", "--wait"]))
    print(shlex.join(compose + ["ps"]))
    print("3. Pair the native app:")
    print_approval(root, prepared)
    print("Credentials retained. No images built or services started.")


class ReviewActionError(Exception):
    def __init__(self, stage, code):
        self.stage = stage
        self.code = code if code > 0 else 128 - code


class ReviewConfigurationError(ValueError):
    """A controlled, non-secret operator action message."""


def detect_docker_platform():
    """Detect the matching Docker engine platform for the current workstation."""
    scripts_dir = Path(__file__).resolve().parents[1] / "scripts"
    spec = importlib.util.spec_from_file_location("fleetworks_platform", scripts_dir / "fleetworks_platform.py")
    if spec is None or spec.loader is None:
        raise ReviewConfigurationError("Unable to load host platform detector")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    info = module.detect()
    if not info.docker_platform:
        raise ReviewConfigurationError(
            f"Unsupported host platform for Docker review: {info.system}/{info.physical_architecture}"
        )
    return info.docker_platform


def run_review(root, name, *, backend_only=False, api_port=None, web_port=None, platform=None, root_env=False):
    """Auto-initialize if needed, build before stopping, and launch the native app on macOS."""
    root = root.resolve(strict=True)
    workspace = review_path(root, name)
    if not workspace.exists():
        resolved_platform = platform or detect_docker_platform()
        resolved_port = DEFAULT_API_PORT if api_port is None else api_port
        resolved_root_env = root_env or (
            not (root / "compose.env").exists() and not (root / "compose.env").is_symlink()
        )
        revision = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD"], cwd=root, text=True).strip()
        receipt = provision(
            root, name, resolved_port, resolved_platform, revision,
            web_port=web_port, root_env=resolved_root_env,
        )
        print(f"Prepared isolated backend review: {receipt['workspace']}", flush=True)
        if resolved_root_env:
            print(f"Editable Compose settings: {root / 'compose.env'}", flush=True)
    prepared = prepare(root, name)
    environment = prepared["environment"]
    if api_port is not None and str(api_port) != environment.get("FLEETWORKS_API_PORT"):
        raise ReviewConfigurationError("Review already exists with a different API port; edit compose.env or choose another --name")
    if web_port is not None and str(web_port) != environment.get("FLEETWORKS_WEB_PORT"):
        raise ReviewConfigurationError("Review already exists with a different Web port; edit compose.env or choose another --name")
    if platform is not None and platform != environment.get("PLATFORM"):
        raise ReviewConfigurationError("Review already exists with a different platform; edit compose.env or choose another --name")
    open_native = sys.platform == "darwin" and not backend_only
    stages = 6 if open_native else 5
    stage = "Docker preflight"
    with preparation_lock(prepared["workspace"], ".up.lock", wait=False):
        try:
            print(f"[1/{stages}] Checking local Docker for {name}…", flush=True)
            if os.environ.get("DOCKER_HOST") and not os.environ.get("DOCKER_CONTEXT"):
                raise ReviewConfigurationError("Select a named local Docker context instead of a DOCKER_HOST override")
            context = subprocess.check_output(["docker", "context", "show"], cwd=root, text=True).strip()
            if not re.fullmatch(r"[A-Za-z0-9_.-]+", context):
                raise ReviewConfigurationError("Invalid Docker context name")
            endpoint = json.loads(subprocess.check_output([
                "docker", "context", "inspect", context, "--format", "{{json .Endpoints.docker.Host}}",
            ], cwd=root, text=True))
            if not isinstance(endpoint, str) or not endpoint.startswith("unix://"):
                raise ReviewConfigurationError("This local review requires a local Docker context")
            docker = ["docker", "--context", context]
            platform = subprocess.check_output(
                docker + ["info", "--format", "{{.OSType}}/{{.Architecture}}"], cwd=root, text=True).strip()
            platform = {"linux/x86_64": "linux/amd64", "linux/aarch64": "linux/arm64"}.get(platform, platform)
            if platform != prepared["environment"]["PLATFORM"]:
                raise ReviewConfigurationError("Docker architecture differs from the review; choose a matching native daemon")

            build, compose = review_commands(root, prepared)
            def pin_context(command):
                position = command.index("docker") + 1
                return command[:position] + ["--context", context] + command[position:]
            build, compose = pin_context(build), pin_context(compose)
            position = build.index("bake") + 1
            # A context's built-in builder uses that daemon, not a separately
            # selected remote or emulating Buildx instance.
            build[position:position] = ["--builder", context]
            subprocess.run(compose + ["config", "--quiet"], cwd=root, check=True)
            stage = "image build"
            print(f"[2/{stages}] Building backend images; Cargo/BuildKit output follows…", flush=True)
            subprocess.run(build, cwd=root, check=True)
            _, current = current_environment(root, prepared["workspace"])
            if current != prepared["environment"]:
                raise ReviewConfigurationError("Review settings changed during the build; rerun before restarting")
            stage = "stop"
            print(f"[3/{stages}] Stopping the selected review after its successful build…", flush=True)
            subprocess.run(compose + ["stop"], cwd=root, check=True)
            stage = "startup/bootstrap"
            print(f"[4/{stages}] Recreating services and waiting for bootstrap/health…", flush=True)
            subprocess.run(compose + ["up", "--no-build", "--pull", "never", "--force-recreate",
                                      "--detach", "--wait"], cwd=root, check=True)
            stage = "status"
            print(f"[5/{stages}] Review status:", flush=True)
            subprocess.run(compose + ["ps"], cwd=root, check=True)
            if open_native:
                stage = "native build/open"
                print("[6/6] Backend ready. Building and opening the native app; Cargo output follows…", flush=True)
                subprocess.run([
                    sys.executable, str(root / "scripts/fleetworks_candidate.py"),
                    "build-native", "--target", "host", "--profile", "debug", "--open",
                ], cwd=root, check=True)
            elif not backend_only:
                print("Backend ready. Native build/open requires macOS; skipped.", flush=True)
            print_approval(root, prepared)
        except subprocess.CalledProcessError as error:
            raise ReviewActionError(stage, error.returncode) from None


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
            {"id": "operator-review", "purpose": "operator", "permissions": list(NATIVE_PERMISSIONS), "token_file": "/run/secrets/gateway_operator_token"},
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
    initialize.add_argument("--name", default=DEFAULT_REVIEW_NAME, help=f"review name (default: {DEFAULT_REVIEW_NAME})")
    initialize.add_argument("--api-port", type=int, default=DEFAULT_API_PORT, help=f"loopback API port (default: {DEFAULT_API_PORT})")
    initialize.add_argument("--web-port", type=int, help="optional distinct loopback Intel Web port")
    initialize.add_argument("--root-env", action="store_true", help="keep editable settings in repo-root compose.env; refuse an existing file")
    initialize.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), help="target Docker platform (auto-detected when omitted)")
    value = subcommands.add_parser("value", help="read one non-secret review setting")
    value.add_argument("--name", default=DEFAULT_REVIEW_NAME, help=f"review name (default: {DEFAULT_REVIEW_NAME})")
    value.add_argument("--field", choices=("workspace", "image_tag", "project"), required=True)
    existing = subcommands.add_parser("prepare", help="prepare existing credentials and print complete build/start/approval commands")
    existing.add_argument("--name", default=DEFAULT_REVIEW_NAME, help=f"review name (default: {DEFAULT_REVIEW_NAME})")
    update = subcommands.add_parser("up", help="auto-init if missing, build/redeploy backend containers, and build/open the native app on macOS")
    update.add_argument("--name", default=DEFAULT_REVIEW_NAME, help=f"review name (default: {DEFAULT_REVIEW_NAME})")
    update.add_argument("--api-port", type=int, help=f"loopback API port when initializing a new review (default: {DEFAULT_API_PORT})")
    update.add_argument("--web-port", type=int, help="optional distinct loopback Intel Web port when initializing")
    update.add_argument("--root-env", action="store_true", help="require linking editable settings to repo-root compose.env on init; refuse an existing file")
    update.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), help="target Docker platform when initializing (auto-detected when omitted)")
    update.add_argument("--backend-only", action="store_true", help="skip building/opening the native app")
    args = parser.parse_args()
    try:
        if args.command == "init":
            platform = args.platform or detect_docker_platform()
            revision = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD"], cwd=ROOT, text=True).strip()
            receipt = provision(ROOT, args.name, args.api_port, platform, revision,
                                web_port=args.web_port, root_env=args.root_env)
            print(f"Prepared isolated backend review: {receipt['workspace']}")
            if args.root_env:
                print(f"Editable Compose settings: {ROOT / 'compose.env'}")
            origin = f"http://127.0.0.1:{args.api_port}"
            operator = Path(receipt["workspace"]) / "secrets" / "operator-token"
            print(f"After starting the backend, enter Gateway URL {origin} in Setup Gateway.")
            print("Approve the pairing ID and code displayed by the app from a separate operator terminal:")
            print(shlex.join([
                "python3", "scripts/fleetworks_pairing.py", "--origin", origin,
                "--operator-token-file", str(operator), "approve",
                "--pairing-id", "<pairing-id>", "--approval-code", "<approval-code>",
            ]))
            print("Keep the operator credential outside the native client; native-token remains for explicit operator/headless provisioning.")
            print("No images built or services started. Keep compose.env and all credentials for restart.")
        elif args.command == "prepare":
            print_prepared(ROOT, prepare(ROOT, args.name))
        elif args.command == "up":
            run_review(
                ROOT,
                args.name,
                backend_only=args.backend_only,
                api_port=args.api_port,
                web_port=args.web_port,
                platform=args.platform,
                root_env=args.root_env,
            )
        else:
            receipt = json.loads((review_path(ROOT, args.name) / "review.json").read_text(encoding="utf-8"))
            print(receipt["environment"]["COMPOSE_PROJECT_NAME"] if args.field == "project" else receipt[args.field])
    except ReviewActionError as error:
        backend = " Backend services remain running." if error.stage == "native build/open" else ""
        parser.exit(error.code, f"Review {error.stage} failed (exit {error.code}). No later stage ran; data and credentials were retained.{backend}\n")
    except ReviewConfigurationError as error:
        parser.exit(2, f"Update stopped before service restart: {error}.\n")
    except KeyboardInterrupt:
        parser.exit(130, "Review operation interrupted. Data and credentials retained; rerun up to resume.\n")
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Review provisioning failed: {type(error).__name__}. Existing files were retained.\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
