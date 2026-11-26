#!/usr/bin/env python3
"""Build and open Fleetworks and its Embedded companion on macOS."""

import argparse
from contextlib import contextmanager, nullcontext
import fcntl
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from zipfile import BadZipFile, ZipFile

sys.dont_write_bytecode = True
SCRIPT_CHECKOUT = Path(__file__).resolve().parents[1]


def git_path(directory, option):
    value = subprocess.check_output(
        ["git", "-C", str(directory), "rev-parse", option],
        text=True, timeout=15,
    ).strip()
    return (directory / value).resolve()


def source_checkouts():
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument("--source-dir", type=Path)
    selection, _ = parser.parse_known_args()
    try:
        common_dir = git_path(SCRIPT_CHECKOUT, "--git-common-dir")
        repository = common_dir.parent
        checkout = selection.source_dir.expanduser().resolve() if selection.source_dir else SCRIPT_CHECKOUT
        if git_path(checkout, "--show-toplevel") != checkout:
            parser.error("Select the root of a Fleetworks checkout")
        if git_path(checkout, "--git-common-dir") != common_dir:
            parser.error("--source-dir must be a worktree of this Fleetworks repository")
    except (OSError, subprocess.SubprocessError) as error:
        parser.error(f"could not select the Fleetworks checkout: {error}")
    if not all((checkout / "scripts" / helper).is_file()
               for helper in ("fleetworks_candidate.py", "fleetworks_qa_pack.py")):
        parser.error("The checkout is missing the Fleetworks packaging helpers")
    return checkout, repository


ROOT, REPOSITORY = source_checkouts()
sys.path.insert(0, str(ROOT / "scripts"))
import fleetworks_candidate as native
import fleetworks_qa_pack as qa

REPO = "xxxcess/argos-osint"
NIGHTLY = "nightly"
BUILD_SOURCE_REF = "refs/fleetworks/quick-start/source"
DEFAULT_PROFILE = REPOSITORY / ".agent-scratch" / "native-quick-start-profile"
APP_ARTIFACT_FAMILIES = {
    "debug": ("Fleetworks-macOS-Desktop-App-debug",),
    "release": ("Fleetworks-macOS-Desktop-App-release", "Fleetworks-macOS-Desktop-App"),
}
COMPANION = "Contents/MacOS/argos-api-host"
DESKTOP = "Contents/MacOS/fleetworks-desktop"
SOURCE_WORKFLOW = "Fleetworks quick start (source)"
DESKTOP_WORKFLOW = "Fleetworks quick start (hosted companion)"
MANIFEST_LIMIT = 1024**2
INBOX_PACK_FAMILY = "Fleetworks-Inbox-QA-Pack"
INBOX_SYNC_CANDIDATES_PER_SEARCH = 4
NON_APP_DIRECTORIES = (".agents/", ".planning/", "docs/")
CI_ROUTING_INPUTS = {
    ".gitignore",
    "README.md",
    "apps/fleetworks-desktop/README.md",
    "deploy/inbox-qa.md",
    ".github/scripts/fleetworks_ci.py",
    ".github/scripts/test_fleetworks_ci.py",
    ".github/workflows/fleetworks.yml",
    ".github/workflows/fleetworks-inbox-qa-smoke.yml",
    ".github/workflows/fleetworks-native-linux-route.yml",
    "scripts/fleetworks_quick_start.py",
    "scripts/test_fleetworks_quick_start.py",
}
EMBEDDED_BUTTONS = r"""
on run arguments
    set targetPid to item 1 of arguments as integer
    set deadline to (current date) + 20
    tell application "System Events"
        set targetApp to first application process whose unix id is targetPid
        set frontmost of targetApp to true
        repeat while (current date) < deadline
            if (count windows of targetApp) > 0 then
                set contentsList to entire contents of window 1 of targetApp
                repeat with controlRef in contentsList
                    set labelText to missing value
                    try
                        set controlItem to contents of controlRef
                        if (value of attribute "AXRole" of controlItem) is "AXButton" then
                            set labelText to value of attribute "AXTitle" of controlItem
                        end if
                    end try
                    if labelText is "Embedded. Run on this PC" then
                        perform action "AXPress" of controlItem
                        exit repeat
                    else if labelText is "Start Embedded Backend" then
                        perform action "AXPress" of controlItem
                        return
                    end if
                end repeat
            end if
            delay 0.35
        end repeat
    end tell
    error "Could not find the Embedded startup controls"
end run
"""
QUIT_APP = r"""
use framework "AppKit"
on run arguments
    set targetPid to item 1 of arguments as integer
    set targetApp to current application's NSRunningApplication's runningApplicationWithProcessIdentifier:targetPid
    if targetApp is missing value then return
    set targetBundle to targetApp's bundleURL()
    if targetBundle is missing value then error "The process is no longer the selected Fleetworks app"
    if (targetBundle's |path|() as text) is not item 2 of arguments then
        error "The process is no longer the selected Fleetworks app"
    end if
    if not (targetApp's terminate() as boolean) then error "Fleetworks did not accept the quit request"
end run
"""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(*args, quiet=False):
    stderr = subprocess.DEVNULL if quiet else None
    return subprocess.check_output(args, cwd=ROOT, text=True, timeout=60, stderr=stderr).strip()


def gh_json(endpoint):
    return json.loads(command("gh", "api", f"repos/{REPO}/{endpoint}"))


def pull_request_number(value):
    if not re.fullmatch(r"[1-9][0-9]{0,9}", value):
        raise argparse.ArgumentTypeError("Enter a positive PR number")
    return int(value)


def commit_sha(value):
    if not re.fullmatch(r"[0-9a-fA-F]{7,40}", value):
        raise argparse.ArgumentTypeError("Enter a Git commit SHA (7–40 hex digits)")
    return value.lower()


def open_nightly_pull_requests():
    try:
        listings = command(
            "gh", "api", "--paginate", f"repos/{REPO}/pulls?state=open&base={NIGHTLY}&per_page=100",
            "--jq", ".[] | {number,title}",
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ValueError("Could not list open Nightly PRs. Check GitHub CLI access with gh auth status") from error
    return [json.loads(line) for line in listings.splitlines()]


def prompt_source():
    while True:
        try:
            value = input("Build source [nightly] (Enter, 'pr' for the list, PR number, or commit SHA): ").strip()
        except EOFError:
            return NIGHTLY
        if not value or value.lower() == NIGHTLY:
            return NIGHTLY
        if value.lower() == "pr":
            pulls = open_nightly_pull_requests()
            if not pulls:
                print("There are no open PRs targeting nightly.")
                continue
            numbers = {pull["number"] for pull in pulls}
            for pull in pulls:
                title = "".join(char if char.isprintable() else " " for char in pull["title"])
                print(f'  #{pull["number"]}  {title}')
            while True:
                try:
                    value = input("Enter an open PR number, or press Enter for nightly: ").strip()
                except EOFError:
                    return NIGHTLY
                if not value:
                    return NIGHTLY
                try:
                    number = pull_request_number(value.removeprefix("#"))
                except argparse.ArgumentTypeError:
                    number = None
                if number in numbers:
                    return f"#{number}"
                print("Select a PR number from the list.")
        try:
            return f'#{pull_request_number(value.removeprefix("#"))}'
        except argparse.ArgumentTypeError:
            pass
        try:
            return commit_sha(value)
        except argparse.ArgumentTypeError as error:
            print(error)


def git_at(checkout, *args):
    return subprocess.check_output(["git", "-C", str(checkout), *args], text=True, timeout=180).strip()


def fetch_build_revision(shared, ref, *, remote="origin"):
    git_at(shared, "fetch", "--no-tags", "--no-write-fetch-head",
           remote, f"+{ref}:{BUILD_SOURCE_REF}")
    return git_at(shared, "rev-parse", "--verify", f"{BUILD_SOURCE_REF}^{{commit}}")


def build_revision(shared, source):
    if source == NIGHTLY:
        return fetch_build_revision(shared, f"refs/heads/{NIGHTLY}")
    if source.startswith("#"):
        number = pull_request_number(source[1:])
        pull = gh_json(f"pulls/{number}")
        require(pull["number"] == number and pull["state"] == "open"
                and pull["base"]["ref"] == NIGHTLY and pull["base"]["repo"]["full_name"].lower() == REPO.lower(),
                f"PR #{number} must be open and target {REPO}:{NIGHTLY}")
        expected = pull["head"]["sha"]
        require(re.fullmatch(r"[0-9a-f]{40}", expected), f"PR #{number} has no valid head commit")
        revision = fetch_build_revision(shared, f"refs/pull/{number}/head", remote=f"https://github.com/{REPO}.git")
        require(revision == expected, f"PR #{number} changed while fetching. Run Quick Start again")
        return revision
    sha = commit_sha(source)
    try:
        return git_at(shared, "rev-parse", "--verify", "--quiet", f"{sha}^{{commit}}")
    except subprocess.CalledProcessError:
        require(len(sha) == 40, "Commit abbreviation is unknown or ambiguous. Supply the full SHA")
        try:
            revision = fetch_build_revision(shared, sha)
        except subprocess.CalledProcessError as fetch_error:
            raise ValueError(f"Commit {sha} is unavailable from origin") from fetch_error
        require(revision == sha, "The fetched commit did not match the requested SHA")
        return revision


def checkout_build_source(shared, source):
    revision = build_revision(shared, source)
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "Build source must resolve to an exact Git commit")
    checkout = shared / ".agent-scratch" / "worktrees" / "quick-start-source"
    require(not checkout.is_symlink() and checkout.resolve() == checkout,
            "The Quick Start worktree must have a regular path inside this repository")
    if checkout.exists():
        require(git_path(checkout, "--show-toplevel") == checkout
                and git_path(checkout, "--git-common-dir") == git_path(shared, "--git-common-dir")
                and git_at(checkout, "rev-parse", "--abbrev-ref", "HEAD") == "HEAD",
                "The Quick Start worktree must be a detached worktree of this repository")
        require_clean_build_source(checkout)
        if git_at(checkout, "rev-parse", "HEAD") != revision:
            git_at(checkout, "switch", "--detach", revision)
    else:
        checkout.parent.mkdir(parents=True, exist_ok=True)
        git_at(shared, "worktree", "add", "--detach", str(checkout), revision)
    require_clean_build_source(checkout)
    print(f"Building {source} at {revision} in {checkout}", flush=True)
    return checkout


def require_clean_build_source(checkout):
    require(not git_at(checkout, "status", "--porcelain", "--untracked-files=normal"),
            "The Quick Start worktree has local changes. Use --source-dir to build those changes")


def build_inputs_match(revision, *, local_desktop=False):
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "A full source revision is required")
    paths = command("git", "diff", "--name-only", "--no-renames", revision, "--", quiet=True).splitlines()
    paths += command("git", "ls-files", "--others", "--exclude-standard").splitlines()
    if local_desktop:
        paths = [path for path in paths if not path.startswith("apps/fleetworks-desktop/")]
    return all(path.startswith(NON_APP_DIRECTORIES) or path in CI_ROUTING_INPUTS for path in paths)


def artifact_metadata(identity, target, families, *, require_success=True):
    require(identity > 0, "An Actions artifact ID must be a positive integer")
    artifact = gh_json(f"actions/artifacts/{identity}")
    require(artifact["id"] == identity and not artifact["expired"],
            "The selected Actions artifact is unavailable or expired")
    run_info = artifact["workflow_run"]
    run = gh_json(f"actions/runs/{run_info['id']}")
    require(run_info["head_repository_id"] == run_info["repository_id"]
            and run["path"] == ".github/workflows/fleetworks.yml"
            and run["id"] == run_info["id"] and run["status"] == "completed"
            and run["head_sha"] == run_info["head_sha"],
            "Select an artifact from a completed Fleetworks run in the upstream repository")
    require(not require_success or run["conclusion"] == "success",
            "Select an artifact from a successful Fleetworks run in the upstream repository")
    family_pattern = "|".join(re.escape(family) for family in families)
    suffix = f"-{target}-{run['head_sha']}-{run['id']}."
    match = re.fullmatch(f"(?:{family_pattern})" + re.escape(suffix) + r"([1-9][0-9]*)", artifact["name"])
    require(match is not None and int(match[1]) <= run["run_attempt"],
            "The artifact does not match the current Mac architecture and Fleetworks run")
    require(re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest") or ""),
            "GitHub did not provide the artifact SHA-256")
    return artifact, run, match[1]


@contextmanager
def download_artifact(artifact, destination, limit):
    require(0 < artifact["size_in_bytes"] <= limit, "The Actions artifact exceeds its size limit")
    reserve_space(destination, artifact["size_in_bytes"])
    archive = destination / "actions.zip"
    with archive.open("xb") as output:
        subprocess.run(["gh", "api", f"repos/{REPO}/actions/artifacts/{artifact['id']}/zip"],
                       cwd=ROOT, stdout=output, check=True, timeout=300)
    require(archive.stat().st_size == artifact["size_in_bytes"]
            and native.sha256(archive) == artifact["digest"].removeprefix("sha256:"),
            "The downloaded Actions artifact did not match GitHub's SHA-256")
    try:
        with ZipFile(archive) as zipped:
            yield zipped
    finally:
        archive.unlink(missing_ok=True)


def reserve_space(directory, size):
    require(shutil.disk_usage(directory).free >= size + 256 * 1024**2,
            "There is insufficient free space for this step and the 256 MiB disk reserve")


def read_json_member(zipped, filename, limit=MANIFEST_LIMIT):
    require(zipped.getinfo(filename).file_size <= limit, f"{filename} exceeds its size limit")
    return json.loads(zipped.read(filename))


def copy_member(zipped, name, destination, limit):
    entry = zipped.getinfo(name)
    require(0 < entry.file_size <= limit and not entry.is_dir(), f"Invalid archive file {name}")
    reserve_space(destination.parent, entry.file_size)
    with zipped.open(entry) as source, destination.open("xb") as output:
        shutil.copyfileobj(source, output, length=1024**2)


def check_bundle(app, manifest, target, *, profile=None, checkout_pin=True):
    require(app.is_dir() and not app.is_symlink(), "A regular macOS app bundle is required")
    rune_revision = manifest.get("rune_revision", "")
    require(manifest["format"] in native.SUPPORTED_NATIVE_FORMATS
            and manifest["target"] == target and manifest["profile"] in APP_ARTIFACT_FAMILIES
            and (profile is None or manifest["profile"] == profile)
            and re.fullmatch(r"[0-9a-f]{40}", rune_revision),
            "The app does not match the host target and selected build profile")
    if checkout_pin:
        require(rune_revision == native.locked_rune_revision(),
                "The app does not match the workspace Rune pin")
    require(manifest.get("companion", {}).get("relative_path") == COMPANION,
            "A full app bundle with the Embedded companion is required")
    embedded = app / "Contents" / "Resources" / "fleetworks-manifest.json"
    require(read_receipt(embedded) == manifest,
            "The app's embedded manifest does not match its receipt")
    hashes = manifest["files"]
    require({DESKTOP, COMPANION, "Contents/Info.plist"}.issubset(hashes), "The app file list is incomplete")
    require(manifest["companion"]["sha256"] == hashes[COMPANION],
            "The Embedded companion digest does not match its receipt")
    require(all(os.access(app / binary, os.X_OK) for binary in (DESKTOP, COMPANION)),
            "The desktop and companion must be executable")
    files = set()
    for path in app.rglob("*"):
        require(not path.is_symlink() and (path.is_dir() or path.is_file()),
                "The app bundle contains an unexpected file type")
        if path.is_file():
            files.add(path.relative_to(app).as_posix())
    require(files == set(hashes) | {embedded.relative_to(app).as_posix()},
            "The app contains files outside its manifest")
    sizes = {}
    for name, digest in hashes.items():
        relative = PurePosixPath(name)
        require(not relative.is_absolute() and ".." not in relative.parts, "Unsafe app file path")
        path = app.joinpath(*relative.parts)
        require(path.is_file() and not path.is_symlink() and path.resolve().is_relative_to(app)
                and native.sha256(path) == digest,
                f"The app file {name} does not match its manifest")
        sizes[name] = path.stat().st_size
    require("actor_catalog" in manifest["default_datasets"]["datasets"], "The app is missing its bundled actor catalog")
    native._verify_bundled_datasets(manifest, "Contents/Resources", sizes)


def read_receipt(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_uid == os.getuid()
            and path.stat().st_size <= MANIFEST_LIMIT,
            f"A bounded app receipt owned by the current user is required at {path}")
    with path.open("rb") as source:
        data = source.read(MANIFEST_LIMIT + 1)
    require(len(data) <= MANIFEST_LIMIT, "App receipt exceeds its size limit")
    value = json.loads(data)
    require(isinstance(value, dict), "Invalid app receipt")
    return value


@contextmanager
def quick_start_lock(shared):
    scratch = shared / ".agent-scratch"
    scratch.mkdir(exist_ok=True)
    lock_path = scratch / "native-quick-start.lock"
    descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "r+b") as lock:
        metadata = os.fstat(lock.fileno())
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.getuid()
                and stat.S_IMODE(metadata.st_mode) == 0o600,
                "The quick-start lock must be a private regular file")
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError("A quick-start command is already in progress for this checkout family") from error
        yield


def share_agent_cache(shared):
    cache = ROOT / "target" / "agents"
    main_cache = shared / "target" / "agents"
    if shared == ROOT:
        return
    main_cache.mkdir(parents=True, exist_ok=True)
    cache.parent.mkdir(parents=True, exist_ok=True)
    if not cache.exists() and not cache.is_symlink():
        cache.symlink_to(main_cache, target_is_directory=True)
    require(cache.samefile(main_cache), "This worktree has a separate Cargo cache; use the main checkout")


def rebuild_desktop(companion, info, target, output, shared, profile):
    require(shutil.disk_usage(output.parent).free >= 3 * 1024**3,
            "The local desktop rebuild needs 3 GiB of free disk for linking; use the hosted desktop")
    share_agent_cache(shared)
    revision, dirty = native.repository_identity()
    toolchain = native.validate_toolchain(info)
    build = native.cargo_native_command(target, profile, agent=True, implicit_host=True)
    binary = native.cargo_executable(native._run_cargo(build))
    catalog_size = (ROOT / "datasets" / "argos_actors.csv").stat().st_size
    reserve_space(output.parent, binary.stat().st_size + companion.stat().st_size + catalog_size)
    native.package_native(
        output, binary, ROOT / "apps" / "fleetworks-desktop" / "resources",
        version=native.artifact_version(), target=target, revision=revision, profile=profile,
        rune_revision=native.locked_rune_revision(), toolchain=toolchain, dirty=dirty,
        cargo_lock_sha256=native.sha256(ROOT / "Cargo.lock"), companion_binary=companion,
        workflow=DESKTOP_WORKFLOW,
    )


def build_source(output, revision, shared, profile):
    share_agent_cache(shared)
    reserve_space(output.parent, native.MAX_NATIVE_UNPACKED_BYTES)
    result = subprocess.run(
        [sys.executable, "-B", str(ROOT / "scripts" / "fleetworks_candidate.py"),
         "build-native", "--agent", "--target", "host", "--profile", profile,
         "--with-companion", "--revision", revision, "--workflow", SOURCE_WORKFLOW,
         "--run-id", f"local-{time.time_ns()}", "--run-attempt", "1", "--output", str(output)],
        cwd=ROOT, stdout=subprocess.DEVNULL,
    )
    if result.returncode:
        free_mib = shutil.disk_usage(output.parent).free // 1024**2
        raise ValueError(
            f"The {profile} desktop and companion build failed (exit {result.returncode}). "
            f"{free_mib:,} MiB remains on the build volume. See the build diagnostics above."
        )


def existing_app(output):
    app = output / native.NATIVE_APP
    receipt = output / native.NATIVE_MANIFEST
    if not output.exists():
        return app, None
    require(output.is_dir() and output.stat().st_uid == os.getuid(),
            "The app output must be a directory owned by the current user")
    if not (app.exists() or app.is_symlink() or receipt.exists() or receipt.is_symlink()):
        require(not any(output.iterdir()), "Choose an empty output directory or an existing Fleetworks bundle")
        return app, None
    return app, read_receipt(receipt)


def validate_installed_output(output, app, current, target):
    if current is not None:
        require({path.name for path in output.iterdir()} == {native.NATIVE_APP, native.NATIVE_MANIFEST},
                "The output contains other files. Select a dedicated directory for the new bundle")
        check_bundle(app, current, target, checkout_pin=False)


def ensure_replaceable(output, app, current, target):
    require(bundle_pid(app) is None, "Close this app and its Embedded companion before replacing the bundle")
    validate_installed_output(output, app, current, target)


def publish_app(staged, output, app, current, target, *, restart_profile=None):
    _, installed = existing_app(output)
    require(installed == current, "The output bundle changed while the candidate was prepared")
    validate_installed_output(output, app, current, target)
    if restart_profile is not None:
        require_no_other_app(app)
        close_app(app, restart_profile)
    require(bundle_pid(app) is None, "Close this app and its Embedded companion before replacing the bundle")
    output.mkdir(exist_ok=True)
    native._atomic_install_stage(output, staged, [native.NATIVE_APP, native.NATIVE_MANIFEST],
                                 native.NATIVE_APP, native.NATIVE_MANIFEST)
    return app


def prepare_source(target, output, app, current, shared, profile, profile_dir):
    revision, _ = native.repository_identity(root=ROOT)
    with tempfile.TemporaryDirectory(prefix=".native-quick-start-", dir=output.parent) as temp:
        staged = Path(temp) / "bundle"
        build_source(staged, revision, shared, profile)
        native.repository_identity(root=ROOT, revision=revision)
        return publish_app(staged, output, app, current, target, restart_profile=profile_dir)


def prepare_artifact(args, info, target, output, app, current, shared):
    if args.rebuild_desktop:
        ensure_replaceable(output, app, current, target)
    profile = args.profile
    artifact, run, attempt = artifact_metadata(args.app_artifact, target, APP_ARTIFACT_FAMILIES[profile])
    require(build_inputs_match(run["head_sha"], local_desktop=args.rebuild_desktop),
            "The selected artifact's build inputs differ from this checkout. Use a compatible Actions artifact")
    with tempfile.TemporaryDirectory(prefix=".native-quick-start-", dir=output.parent) as temp:
        temporary = Path(temp)
        with download_artifact(artifact, temporary, native.MAX_NATIVE_ARCHIVE_BYTES + 1024**2) as outer:
            manifest = read_json_member(outer, native.NATIVE_MANIFEST)
            require(manifest["revision"] == run["head_sha"] and manifest["run_id"] == str(run["id"])
                    and manifest["run_attempt"] == attempt and manifest["target"] == target
                    and manifest["profile"] == profile and manifest["dirty"] is False
                    and manifest["rune_revision"] == native.locked_rune_revision()
                    and manifest["dependency_provenance"]["cargo_lock_sha256"] == native.sha256(ROOT / "Cargo.lock")
                    and manifest.get("companion", {}).get("relative_path") == COMPANION,
                    "The artifact must contain a clean, complete app matching the run and checkout")
            name = native.native_archive_name(manifest["version"], target, run["head_sha"], run["id"], attempt)
            require(sorted(outer.namelist()) == sorted([name, name + ".sha256", native.NATIVE_MANIFEST]),
                    "The Actions app artifact contains unexpected files")
            if not args.rebuild_desktop and current == manifest:
                check_bundle(app, current, target, profile=profile)
                return app
            if args.rebuild_desktop:
                companion = temporary / "argos-api-host"
                with outer.open(name) as package, ZipFile(package) as zipped:
                    embedded = native.NATIVE_APP + "/Contents/Resources/fleetworks-manifest.json"
                    require(read_json_member(zipped, embedded) == manifest, "App manifest mismatch")
                    copy_member(zipped, native.NATIVE_APP + "/" + COMPANION, companion,
                                native.MAX_NATIVE_UNPACKED_BYTES)
                require(native.sha256(companion) == manifest["companion"]["sha256"] == manifest["files"][COMPANION],
                        "The companion binary does not match its manifest")
                companion.chmod(0o755)
            else:
                ensure_replaceable(output, app, current, target)
                archive = temporary / name
                copy_member(outer, name, archive, native.MAX_NATIVE_ARCHIVE_BYTES)
                copy_member(outer, name + ".sha256", archive.with_name(name + ".sha256"), 1024)
        staged = temporary / "bundle"
        if args.rebuild_desktop:
            rebuild_desktop(companion, info, target, staged, shared, profile)
        else:
            with ZipFile(archive) as inner_zip:
                reserve_space(temporary, sum(entry.file_size for entry in inner_zip.infolist()))
            checksum = archive.with_name(name + ".sha256").read_text().split(maxsplit=1)
            require(checksum and re.fullmatch(r"[0-9a-f]{64}", checksum[0]),
                    "The app archive checksum entry is invalid")
            _, inner = native.extract_native(
                archive, staged, expected_digest=checksum[0], version=manifest["version"], target=target,
                revision=run["head_sha"], rune_revision=native.locked_rune_revision(),
                profile=profile, require_clean=True,
            )
            require(inner == manifest and "actor_catalog" in manifest["default_datasets"]["datasets"],
                    "The extracted app is missing required manifest data")
            (staged / native.NATIVE_MANIFEST).write_text(json.dumps(inner) + "\n")
        return publish_app(staged, output, app, current, target)


def prepare_app(args, info, target, output, shared, profile_dir):
    app, current = existing_app(output)
    if args.reuse_bundle:
        require(current is not None and current.get("dirty") is False
                and build_inputs_match(current["revision"]),
                "The installed clean bundle does not match this checkout. Run the source quick start to update it")
        check_bundle(app, current, target, profile=args.profile)
        return app
    if args.app_artifact is not None:
        return prepare_artifact(args, info, target, output, app, current, shared)
    return prepare_source(target, output, app, current, shared, args.profile, profile_dir)


def process_table():
    return [line.split(maxsplit=2) for line in command("ps", "-ww", "-axo", "pid=,ppid=,command=").splitlines()]


def executable_pid(*paths):
    executables = [str(path) for path in paths]
    return next((int(fields[0]) for fields in process_table()
                 if len(fields) == 3 and any(fields[2] == executable or fields[2].startswith(executable + " ")
                                             for executable in executables)), None)


def app_pid(app):
    return executable_pid(app / DESKTOP)


def bundle_pid(app):
    return executable_pid(app / DESKTOP, app / COMPANION)


def embedded_running(app, pid, state):
    prefix = f"{app / COMPANION} --role local-ipc --state-dir {state} --ipc-fd "
    return any(len(fields) == 3 and fields[1] == str(pid) and fields[2].startswith(prefix)
               for fields in process_table())


def wait_for(check, message, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if result := check():
            return result
        time.sleep(0.4)
    raise ValueError(message)


def require_no_other_app(app):
    selected = app_pid(app)
    processes = [line.split(maxsplit=1) for line in command("ps", "-axo", "pid=,comm=").splitlines()
                 if line.strip()]
    require(not any(Path(executable).name == native.NATIVE_BINARY and int(pid) != selected
                    for pid, executable in processes),
            "Close the other Fleetworks app before opening this bundle")


def require_private_profile(profile):
    require(profile.is_dir() and not profile.is_symlink() and profile.stat().st_uid == os.getuid()
            and stat.S_IMODE(profile.stat().st_mode) == 0o700,
            "The isolated profile must be a private directory owned by the current user")


def app_profile_matches(pid, profile):
    invocation = f" {command('ps', 'eww', '-p', str(pid), '-o', 'command=')} "
    return all(f" {key}={profile} " in invocation for key in ("HOME", "CFFIXED_USER_HOME"))


def close_app(app, profile):
    pid = app_pid(app)
    if pid is not None:
        state = profile / ".fleetworks" / "local-backend" / "embedded"
        if executable_pid(app / COMPANION) is not None:
            require(embedded_running(app, pid, state),
                    "The running Embedded companion uses a different profile. Stop it before replacing this bundle")
        else:
            require(app_profile_matches(pid, profile),
                    "The running app uses a different profile. Close it before replacing this bundle")
        print(f"Quitting {app} and waiting for Embedded shutdown", flush=True)
        subprocess.run(["osascript", "-", str(pid), str(app)], input=QUIT_APP, text=True,
                       check=True, timeout=30)
    wait_for(lambda: bundle_pid(app) is None,
             "Fleetworks or its Embedded companion did not quit. The installed bundle is still available")


def start_companion(app, pid, state):
    try:
        subprocess.run(["osascript", "-", str(pid)], input=EMBEDDED_BUTTONS, text=True, check=True, timeout=30)
    except subprocess.SubprocessError as error:
        raise ValueError("Could not start Embedded through macOS Accessibility. "
                         "In Fleetworks, choose Embedded and click Start Embedded Backend, then rerun Quick Start.") from error
    wait_for(lambda: embedded_running(app, pid, state), "The Embedded companion did not start for this profile")


def open_app(app, profile, *, start_embedded):
    pid = app_pid(app)
    state = profile / ".fleetworks" / "local-backend" / "embedded"
    if pid is not None:
        if executable_pid(app / COMPANION) is not None:
            require(embedded_running(app, pid, state), "This app already uses a different Embedded profile")
        elif start_embedded:
            require(app_profile_matches(pid, profile), "Close this app before opening with the requested profile")
            start_companion(app, pid, state)
        print(f"Using running app {app}")
        return pid, state
    require_no_other_app(app)
    require(bundle_pid(app) is None, "The previous Embedded companion is still running")
    subprocess.run(["open", "--env", f"CFFIXED_USER_HOME={profile}", "--env", f"HOME={profile}",
                    str(app), "--args", "--local-backend-bin", str(app / COMPANION)],
                   check=True, timeout=30)
    pid = wait_for(lambda: app_pid(app), "Fleetworks did not appear after launching")
    if start_embedded and not embedded_running(app, pid, state):
        start_companion(app, pid, state)
    print(f"Opened {app}")
    return pid, state


def inbox_smoke_receipt(run, target, pack_attempt, temporary):
    family = "Fleetworks-Inbox-QA-Embedded"
    run_id = run["id"]
    revision = run["head_sha"]
    for attempt in range(int(pack_attempt), run["run_attempt"] + 1):
        name = f"{family}-{target}-{revision}-{run_id}.{attempt}"
        result = gh_json(f"actions/runs/{run_id}/artifacts?name={name}")
        matches = [item for item in result["artifacts"] if item["name"] == name and not item["expired"]]
        if not matches:
            continue
        require(len(matches) == 1, "Expected one exact Embedded Inbox smoke artifact for this attempt")
        smoke, smoke_run, smoke_attempt = artifact_metadata(
            matches[0]["id"], target, (family,), require_success=False,
        )
        require(smoke_run["id"] == run_id and smoke_run["head_sha"] == revision and int(smoke_attempt) == attempt,
                "The Embedded Inbox smoke artifact belongs to a different run or attempt")
        with download_artifact(smoke, temporary, MANIFEST_LIMIT) as outer:
            report_name = f"embedded-{target}.json"
            receipt_name = f"pack-digests-{target}.json"
            names = outer.namelist()
            if not {report_name, receipt_name}.issubset(names):
                continue
            require(names.count(report_name) == names.count(receipt_name) == 1,
                    "The Embedded Inbox smoke artifact contains duplicate evidence")
            receipt = read_json_member(outer, receipt_name, 64 * 1024)
            if str(receipt["run_attempt"]) != pack_attempt:
                continue
            report = read_json_member(outer, report_name, 64 * 1024)
            if report.get("generated_dialogue") is not True:
                continue
            evidence_file = temporary / report_name
            evidence_file.write_text(json.dumps(report), encoding="utf-8")
            try:
                subprocess.run([sys.executable, "-B", str(ROOT / ".github/scripts/inbox_qa_smoke_receipt.py"),
                                "--evidence", str(evidence_file), "--target", target, "--head", revision],
                               check=True, timeout=30)
            except subprocess.CalledProcessError:
                continue
            return receipt
    raise ValueError(f"No verified Embedded Inbox smoke receipt is available for the {target} pack in run {run_id}")


def discover_inbox_artifacts(target, revision):
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "A full app source revision is required")
    seen_runs = set()
    seen_artifacts = set()
    searches = ((f"&head_sha={revision}", 6), ("", 16))
    for exact, limit in searches:
        count = 0
        result = gh_json(f"actions/workflows/fleetworks.yml/runs?status=completed{exact}&per_page={limit}")
        runs = sorted(result["workflow_runs"], key=lambda run: (run["created_at"], run["id"]), reverse=True)
        for run in runs[:limit]:
            if count >= INBOX_SYNC_CANDIDATES_PER_SEARCH:
                break
            run_id, sha = run["id"], run["head_sha"]
            if (run_id in seen_runs or run["status"] != "completed"
                    or run["path"] != ".github/workflows/fleetworks.yml"
                    or (run.get("repository") or {}).get("full_name") != REPO
                    or (run.get("head_repository") or {}).get("full_name") != REPO
                    or exact and sha != revision):
                continue
            seen_runs.add(run_id)
            try:
                if not build_inputs_match(sha, local_desktop=True):
                    continue
            except subprocess.CalledProcessError:
                # The historical SHA might not be present in this checkout.
                continue
            prefix = re.escape(f"{INBOX_PACK_FAMILY}-{target}-{sha}-{run_id}.")
            listed = gh_json(f"actions/runs/{run_id}/artifacts?per_page=100")["artifacts"]
            candidates = []
            for artifact in listed[:100]:
                match = re.fullmatch(prefix + r"([1-9][0-9]*)", artifact["name"])
                if (not artifact["expired"] and artifact["id"] not in seen_artifacts
                        and match and int(match[1]) <= run["run_attempt"]):
                    candidates.append((int(match[1]), artifact["id"]))
            for _, identity in sorted(candidates, reverse=True):
                if identity in seen_artifacts:
                    continue
                seen_artifacts.add(identity)
                count += 1
                yield identity
                if count >= INBOX_SYNC_CANDIDATES_PER_SEARCH:
                    break


def inbox_pointer(state):
    pointer = state / "qa-pack.json"
    try:
        info = pointer.lstat()
    except FileNotFoundError:
        return None
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == 0o600 and 0 < info.st_size <= 4096,
            "The existing Inbox pack pointer is not private and bounded")
    config = json.loads(pointer.read_text(), object_pairs_hook=qa.unique_pairs)
    require(isinstance(config, dict) and set(config) == {"root", "manifest_sha256"}
            and isinstance(config["root"], str) and Path(config["root"]).is_absolute()
            and qa.is_digest(config["manifest_sha256"]),
            "The existing Inbox pack pointer is invalid")
    return config


def activate_inbox_pointer(state, pack, digest):
    with tempfile.NamedTemporaryFile("w", dir=state, prefix=".qa-pack-", delete=False) as output:
        staged = Path(output.name)
        try:
            json.dump({"root": str(pack), "manifest_sha256": digest}, output, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
            os.replace(staged, state / "qa-pack.json")
        finally:
            staged.unlink(missing_ok=True)


def install_inbox(identity, target, profile, state, *, automatic=False, replace_existing=True):
    require(state.is_dir() and state.resolve() == state and state.is_relative_to(profile)
            and state.stat().st_uid == os.getuid() and stat.S_IMODE(state.stat().st_mode) == 0o700,
            "Start Embedded in the isolated profile before installing Inbox support")
    config = inbox_pointer(state)
    artifact, run, attempt = artifact_metadata(identity, target, (INBOX_PACK_FAMILY,), require_success=False)
    require(build_inputs_match(run["head_sha"], local_desktop=True), "The Inbox worker inputs differ from this checkout")
    installed = profile / f"qa-pack-{artifact['id']}"
    with tempfile.TemporaryDirectory(prefix=".inbox-quick-start-", dir=profile) as temp:
        temporary = Path(temp)
        smoke = inbox_smoke_receipt(run, target, attempt, temporary)
        digest = smoke["manifest_sha256"]
        require(smoke["target"] == target and smoke["source_sha"] == run["head_sha"]
                and str(smoke["run_id"]) == str(run["id"]) and str(smoke["run_attempt"]) == attempt
                and type(smoke["archive_bytes"]) is int and 0 < smoke["archive_bytes"] <= qa.MAX_COMPRESSED
                and qa.is_digest(smoke["archive_sha256"]) and qa.is_digest(digest),
                "The Embedded Inbox smoke receipt does not identify the verified pack")
        if config is not None and config["manifest_sha256"] == digest:
            active = Path(config["root"])
            if active == installed or (automatic and active.is_relative_to(profile) and active.resolve() == active):
                try:
                    qa.verify_directory(active, target, digest)
                except (OSError, ValueError, subprocess.SubprocessError):
                    if active == installed:
                        raise
                else:
                    return False
        require(replace_existing or config is None,
                "Quit Fleetworks before switching the Inbox pack for this profile")
        if installed.exists() or installed.is_symlink():
            qa.verify_directory(installed, target, digest)
            activate_inbox_pointer(state, installed, digest)
            return True
        progress = getattr(qa, "pack_progress", None)
        stage = progress(state, "downloading_pack") if progress else nullcontext()
        with stage, download_artifact(artifact, temporary, qa.MAX_COMPRESSED + 1024**2) as outer:
            name = f"{INBOX_PACK_FAMILY}-{target}.tar.gz"
            require(sorted(outer.namelist()) == sorted([name, "digests.json"]),
                    "The Inbox pack artifact contains unexpected files")
            evidence = read_json_member(outer, "digests.json", 64 * 1024)
            require(evidence == smoke and outer.getinfo(name).file_size == smoke["archive_bytes"],
                    "The Inbox pack archive and receipt do not match the verified Embedded smoke")
            archive = temporary / name
            copy_member(outer, name, archive, qa.MAX_COMPRESSED)
        reserve_space(profile, archive.stat().st_size + qa.MAX_UNPACKED)
        qa.install(argparse.Namespace(archive=archive, output=installed, state_dir=state, target=target,
                                     archive_sha256=smoke["archive_sha256"], manifest_sha256=digest))
        return True


def sync_inbox(target, revision, profile, state):
    try:
        for identity in discover_inbox_artifacts(target, revision):
            try:
                changed = install_inbox(identity, target, profile, state, automatic=True)
            except (BadZipFile, KeyError, TypeError, ValueError, subprocess.CalledProcessError):
                continue
            print(f"Offline Inbox pack {'synced' if changed else 'already current'} (Actions artifact {identity}).")
            return True
    except (BadZipFile, KeyError, OSError, TypeError, ValueError, subprocess.SubprocessError):
        pass
    print("No compatible, smoke-verified Inbox pack could be synced. Keeping this profile's current pack.")
    return False


def main():
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--source-dir", type=Path, help="Use a local worktree and include its uncommitted changes")
    source.add_argument("--nightly", action="store_true", help="Build the latest origin/nightly without prompting")
    source.add_argument("--pr", type=pull_request_number, metavar="NUMBER",
                        help="Build the head of this open PR targeting nightly without prompting")
    source.add_argument("--commit", type=commit_sha, metavar="SHA", help="Build this Git commit without prompting")
    parser.add_argument("--profile", choices=tuple(APP_ARTIFACT_FAMILIES), default="debug",
                        help="Use the debug or optimized release build profile (default: debug)")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--app-artifact", type=int, help="Open this verified macOS app Actions artifact for the selected profile")
    mode.add_argument("--reuse-bundle", action="store_true",
                      help="Open a matching installed clean bundle without building or downloading an app")
    parser.add_argument("--rebuild-desktop", action="store_true",
                        help="Build only the desktop in the shared Cargo cache and reuse the verified companion")
    parser.add_argument("--start-embedded", action="store_true",
                        help="Select and start Embedded through the app's macOS accessibility controls")
    parser.add_argument("--inbox-artifact", type=int, help="Install a verified offline Inbox model pack")
    parser.add_argument("--no-inbox-sync", action="store_true", help="Skip automatic Inbox pack synchronization")
    parser.add_argument("--output", type=Path,
                        help="Managed bundle directory (default: target/native/<platform>/<profile>)")
    parser.add_argument("--profile-dir", type=Path, default=DEFAULT_PROFILE)
    args = parser.parse_args()
    if args.rebuild_desktop and args.app_artifact is None:
        parser.error("--rebuild-desktop requires --app-artifact")
    source_build = args.app_artifact is None and not args.reuse_bundle
    if not source_build and (args.nightly or args.pr is not None or args.commit):
        parser.error("--nightly, --pr, and --commit require a source build")
    try:
        info = native.detect()
        target = info.native_target
        require(target in native.DARWIN_NATIVE_TARGETS, "This quick start supports Intel and Apple Silicon macOS")
        shared = REPOSITORY
        requested_output = args.output or shared / "target" / "native" / target / args.profile
        require(not requested_output.expanduser().is_symlink(), "The app output cannot be a symlink")
        output = native.native_output(requested_output, target, args.profile)
        require(not args.profile_dir.expanduser().is_symlink(), "The isolated profile cannot be a symlink")
        profile = args.profile_dir.expanduser().resolve()
        home = Path.home().resolve()
        require(profile != home and not profile.is_relative_to(home / ".fleetworks")
                and not profile.is_relative_to(output)
                and not output.is_relative_to(profile),
                "Select a dedicated profile directory outside the app output")
        selection = args.commit
        if source_build and args.source_dir is None:
            if args.nightly:
                selection = NIGHTLY
            elif args.pr is not None:
                selection = f"#{args.pr}"
            elif selection is None:
                selection = prompt_source()
        print(f"Fleetworks {target} ({args.profile}): {output / native.NATIVE_APP}", flush=True)
        with quick_start_lock(shared):
            require_no_other_app(output / native.NATIVE_APP)
            if profile.exists():
                require_private_profile(profile)
            if selection is not None:
                ROOT = checkout_build_source(shared, selection)
            output.parent.mkdir(parents=True, exist_ok=True)
            app = prepare_app(args, info, target, output, shared, profile)
            profile.mkdir(parents=True, mode=0o700, exist_ok=True)
            require_private_profile(profile)
            state = profile / ".fleetworks" / "local-backend" / "embedded"
            stopped = bundle_pid(app) is None
            automatic = stopped and not args.no_inbox_sync and args.inbox_artifact is None
            prelaunch = stopped and state.is_dir()
            if prelaunch:
                if args.inbox_artifact is not None:
                    install_inbox(args.inbox_artifact, target, profile, state)
                elif automatic:
                    revision = read_receipt(output / native.NATIVE_MANIFEST)["revision"]
                    sync_inbox(target, revision, profile, state)
            pid, state = open_app(app, profile, start_embedded=args.start_embedded or bool(args.inbox_artifact))
            running = embedded_running(app, pid, state)
            if args.inbox_artifact is not None:
                require(running, "The profile's Embedded companion must be running to add Inbox support")
                if not prelaunch:
                    install_inbox(args.inbox_artifact, target, profile, state, replace_existing=False)
                print("Offline Inbox pack verified. Enable Backend inference in Settings → Connection.")
            elif automatic and not prelaunch and args.start_embedded and running:
                revision = read_receipt(output / native.NATIVE_MANIFEST)["revision"]
                sync_inbox(target, revision, profile, state)
            if running:
                print(f"Embedded companion launched with state in {state}")
            else:
                print('Choose Embedded, then "Start Embedded Backend".')
    except KeyboardInterrupt:
        parser.exit(130, "\nQuick Start cancelled.\n")
    except (BadZipFile, KeyError, OSError, TypeError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"fleetworks-quick-start: {error}\n")


if __name__ == "__main__":
    main()
