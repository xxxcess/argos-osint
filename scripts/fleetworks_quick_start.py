#!/usr/bin/env python3
"""Build and open Fleetworks and its Embedded companion on macOS."""

import argparse
from contextlib import contextmanager
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


def source_checkout():
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument("--source-dir", type=Path)
    selection, _ = parser.parse_known_args()
    if selection.source_dir is None:
        return SCRIPT_CHECKOUT

    checkout = selection.source_dir.expanduser().resolve()

    def git_path(directory, option):
        path = subprocess.check_output(
            ["git", "-C", str(directory), "rev-parse", "--path-format=absolute", option],
            text=True, timeout=15,
        ).strip()
        return Path(path).resolve()

    try:
        if git_path(checkout, "--show-toplevel") != checkout:
            parser.error("--source-dir must be the root of a Fleetworks worktree")
        if git_path(checkout, "--git-common-dir") != git_path(SCRIPT_CHECKOUT, "--git-common-dir"):
            parser.error("--source-dir must be a worktree of this Fleetworks repository")
    except (OSError, subprocess.SubprocessError) as error:
        parser.error(f"could not select --source-dir: {error}")
    if not all((checkout / "scripts" / helper).is_file()
               for helper in ("fleetworks_candidate.py", "fleetworks_qa_pack.py")):
        parser.error("--source-dir is missing the Fleetworks packaging helpers")
    return checkout


ROOT = source_checkout()
sys.path.insert(0, str(ROOT / "scripts"))
import fleetworks_candidate as native
import fleetworks_qa_pack as qa

REPO = "xxxcess/argos-osint"
DEFAULT_OUTPUT = ROOT / ".agent-scratch" / "native-quick-start"
COMPANION = "Contents/MacOS/argos-api-host"
DESKTOP = "Contents/MacOS/fleetworks-desktop"
SOURCE_WORKFLOW = "Fleetworks quick start (source)"
DESKTOP_WORKFLOW = "Fleetworks quick start (hosted companion)"
MANIFEST_LIMIT = 1024**2
NON_APP_DIRECTORIES = (".agents/", ".planning/", "docs/")
CI_ROUTING_INPUTS = {
    "README.md",
    "apps/fleetworks-desktop/README.md",
    ".github/scripts/fleetworks_ci.py",
    ".github/scripts/test_fleetworks_ci.py",
    ".github/workflows/fleetworks.yml",
    ".github/workflows/fleetworks-inbox-qa-smoke.yml",
    ".github/workflows/fleetworks-native-linux-route.yml",
    "scripts/fleetworks_quick_start.py",
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


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, timeout=60).strip()


def gh_json(endpoint):
    return json.loads(command("gh", "api", f"repos/{REPO}/{endpoint}"))


def build_inputs_match(revision, *, local_desktop=False):
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "A full source revision is required")
    paths = command("git", "diff", "--name-only", "--no-renames", revision, "--").splitlines()
    paths += command("git", "ls-files", "--others", "--exclude-standard").splitlines()
    if local_desktop:
        paths = [path for path in paths if not path.startswith("apps/fleetworks-desktop/")]
    return all(path.startswith(NON_APP_DIRECTORIES) or path in CI_ROUTING_INPUTS for path in paths)


def artifact_metadata(identity, target, family):
    require(identity > 0, "An Actions artifact ID must be a positive integer")
    artifact = gh_json(f"actions/artifacts/{identity}")
    require(not artifact["expired"], "The selected Actions artifact expired")
    run_info = artifact["workflow_run"]
    run = gh_json(f"actions/runs/{run_info['id']}")
    require(run_info["head_repository_id"] == run_info["repository_id"]
            and run["path"] == ".github/workflows/fleetworks.yml"
            and run["status"] == "completed" and run["conclusion"] == "success"
            and run["head_sha"] == run_info["head_sha"],
            "Select an artifact from a successful Fleetworks run in the upstream repository")
    prefix = f"{family}-{target}-{run['head_sha']}-{run['id']}."
    match = re.fullmatch(re.escape(prefix) + r"([1-9][0-9]*)", artifact["name"])
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


def check_bundle(app, manifest, target, *, checkout_pin=True):
    require(app.is_dir() and not app.is_symlink(), "A regular macOS app bundle is required")
    rune_revision = manifest.get("rune_revision", "")
    require(manifest["format"] in native.SUPPORTED_NATIVE_FORMATS
            and manifest["target"] == target and manifest["profile"] == "debug"
            and re.fullmatch(r"[0-9a-f]{40}", rune_revision),
            "The app does not match the host debug profile")
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


def repo_family_root():
    return Path(command("git", "rev-parse", "--path-format=absolute", "--git-common-dir")).parent


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


def rebuild_desktop(companion, info, target, output, shared):
    require(shutil.disk_usage(output.parent).free >= 3 * 1024**3,
            "The local desktop rebuild needs 3 GiB of free disk for linking; use the hosted desktop")
    share_agent_cache(shared)
    revision, dirty = native.repository_identity()
    toolchain = native.validate_toolchain(info)
    build = native.cargo_native_command(target, "debug", agent=True, implicit_host=True)
    binary = native.cargo_executable(native._run_cargo(build))
    catalog_size = (ROOT / "datasets" / "argos_actors.csv").stat().st_size
    reserve_space(output.parent, binary.stat().st_size + companion.stat().st_size + catalog_size)
    native.package_native(
        output, binary, ROOT / "apps" / "fleetworks-desktop" / "resources",
        version=native.artifact_version(), target=target, revision=revision, profile="debug",
        rune_revision=native.locked_rune_revision(), toolchain=toolchain, dirty=dirty,
        cargo_lock_sha256=native.sha256(ROOT / "Cargo.lock"), companion_binary=companion,
        workflow=DESKTOP_WORKFLOW,
    )


def build_source(output, revision, shared):
    share_agent_cache(shared)
    reserve_space(output.parent, native.MAX_NATIVE_UNPACKED_BYTES)
    subprocess.run(
        [sys.executable, "-B", str(ROOT / "scripts" / "fleetworks_candidate.py"),
         "build-native", "--agent", "--target", "host", "--profile", "debug",
         "--with-companion", "--revision", revision, "--workflow", SOURCE_WORKFLOW,
         "--run-id", f"local-{time.time_ns()}", "--run-attempt", "1", "--output", str(output)],
        cwd=ROOT, stdout=subprocess.DEVNULL, check=True,
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


def ensure_replaceable(output, app, current, target):
    require(bundle_pid(app) is None, "Close this app and its Embedded companion before replacing the bundle")
    if current is not None:
        require({path.name for path in output.iterdir()} == {native.NATIVE_APP, native.NATIVE_MANIFEST},
                "The output contains other files. Select a dedicated directory for the new bundle")
        check_bundle(app, current, target, checkout_pin=False)


def publish_app(staged, output, app, current, target):
    _, installed = existing_app(output)
    require(installed == current, "The output bundle changed while the candidate was prepared")
    ensure_replaceable(output, app, current, target)
    output.mkdir(exist_ok=True)
    native._atomic_install_stage(output, staged, [native.NATIVE_APP, native.NATIVE_MANIFEST],
                                 native.NATIVE_APP, native.NATIVE_MANIFEST)
    return app


def prepare_source(target, output, app, current, shared):
    revision, dirty = native.repository_identity()
    if not dirty and current is not None and current.get("workflow") == SOURCE_WORKFLOW \
            and current.get("dirty") is False and current.get("revision") == revision \
            and current.get("version") == native.artifact_version() \
            and current.get("dependency_provenance", {}).get("cargo_lock_sha256") == native.sha256(ROOT / "Cargo.lock"):
        check_bundle(app, current, target)
        return app
    ensure_replaceable(output, app, current, target)
    with tempfile.TemporaryDirectory(prefix=".native-quick-start-", dir=output.parent) as temp:
        staged = Path(temp) / "bundle"
        build_source(staged, revision, shared)
        return publish_app(staged, output, app, current, target)


def prepare_artifact(args, info, target, output, app, current, shared):
    if args.rebuild_desktop:
        ensure_replaceable(output, app, current, target)
    artifact, run, attempt = artifact_metadata(args.app_artifact, target, "Fleetworks-macOS-Desktop-App-debug")
    require(build_inputs_match(run["head_sha"], local_desktop=args.rebuild_desktop),
            "The selected artifact's build inputs differ from this checkout. Use a compatible Actions artifact")
    with tempfile.TemporaryDirectory(prefix=".native-quick-start-", dir=output.parent) as temp:
        temporary = Path(temp)
        with download_artifact(artifact, temporary, native.MAX_NATIVE_ARCHIVE_BYTES + 1024**2) as outer:
            manifest = read_json_member(outer, native.NATIVE_MANIFEST)
            require(manifest["revision"] == run["head_sha"] and manifest["run_id"] == str(run["id"])
                    and manifest["run_attempt"] == attempt and manifest["target"] == target
                    and manifest["profile"] == "debug" and manifest["dirty"] is False
                    and manifest["rune_revision"] == native.locked_rune_revision()
                    and manifest["dependency_provenance"]["cargo_lock_sha256"] == native.sha256(ROOT / "Cargo.lock")
                    and manifest.get("companion", {}).get("relative_path") == COMPANION,
                    "The artifact must contain a clean, complete app matching the run and checkout")
            name = native.native_archive_name(manifest["version"], target, run["head_sha"], run["id"], attempt)
            require(sorted(outer.namelist()) == sorted([name, name + ".sha256", native.NATIVE_MANIFEST]),
                    "The Actions app artifact contains unexpected files")
            if not args.rebuild_desktop and current == manifest:
                check_bundle(app, current, target)
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
            rebuild_desktop(companion, info, target, staged, shared)
        else:
            with ZipFile(archive) as inner_zip:
                reserve_space(temporary, sum(entry.file_size for entry in inner_zip.infolist()))
            checksum = archive.with_name(name + ".sha256").read_text().split(maxsplit=1)
            require(checksum and re.fullmatch(r"[0-9a-f]{64}", checksum[0]),
                    "The app archive checksum entry is invalid")
            _, inner = native.extract_native(
                archive, staged, expected_digest=checksum[0], version=manifest["version"], target=target,
                revision=run["head_sha"], rune_revision=native.locked_rune_revision(),
                profile="debug", require_clean=True,
            )
            require(inner == manifest and "actor_catalog" in manifest["default_datasets"]["datasets"],
                    "The extracted app is missing required manifest data")
            (staged / native.NATIVE_MANIFEST).write_text(json.dumps(inner) + "\n")
        return publish_app(staged, output, app, current, target)


def prepare_app(args, info, target, output, shared):
    app, current = existing_app(output)
    if args.reuse_bundle:
        require(current is not None and current.get("dirty") is False
                and build_inputs_match(current["revision"]),
                "The installed clean bundle does not match this checkout. Run the source quick start to update it")
        check_bundle(app, current, target)
        return app
    if args.app_artifact is not None:
        return prepare_artifact(args, info, target, output, app, current, shared)
    return prepare_source(target, output, app, current, shared)


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
    if app_pid(app) is None:
        require("fleetworks-desktop" not in [Path(line).name for line in command("ps", "-axo", "comm=").splitlines()],
                "Close the other Fleetworks app before opening this bundle")


def require_private_profile(profile):
    require(profile.is_dir() and not profile.is_symlink() and profile.stat().st_uid == os.getuid()
            and stat.S_IMODE(profile.stat().st_mode) == 0o700,
            "The isolated profile must be a private directory owned by the current user")


def app_profile_matches(pid, profile):
    invocation = f" {command('ps', 'eww', '-p', str(pid), '-o', 'command=')} "
    return all(f" {key}={profile} " in invocation for key in ("HOME", "CFFIXED_USER_HOME"))


def start_companion(app, pid, state):
    subprocess.run(["osascript", "-", str(pid)], input=EMBEDDED_BUTTONS, text=True, check=True, timeout=30)
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


def install_inbox(identity, target, profile, state):
    require(state.is_dir() and state.resolve().is_relative_to(profile)
            and not state.is_symlink() and stat.S_IMODE(state.stat().st_mode) == 0o700,
            "Start Embedded in the isolated profile before installing Inbox support")
    pointer = state / "qa-pack.json"
    if pointer.exists():
        require(not pointer.is_symlink() and pointer.stat().st_uid == os.getuid()
                and stat.S_IMODE(pointer.stat().st_mode) == 0o600 and pointer.stat().st_size <= 4096,
                "The existing Inbox pack pointer is not private and bounded")
        config = json.loads(pointer.read_text())
        pack = Path(config["root"])
        require(pack == profile / f"qa-pack-{identity}" and not pack.is_symlink(),
                "This profile already points to a different Inbox pack; use a separate profile")
        qa.verify_directory(pack, target, config["manifest_sha256"])
        return
    artifact, run, attempt = artifact_metadata(identity, target, "Fleetworks-Inbox-QA-Pack")
    require(build_inputs_match(run["head_sha"], local_desktop=True), "The Inbox worker inputs differ from this checkout")
    installed = profile / f"qa-pack-{artifact['id']}"
    with tempfile.TemporaryDirectory(prefix=".inbox-quick-start-", dir=profile) as temp:
        temporary = Path(temp)
        with download_artifact(artifact, temporary, qa.MAX_COMPRESSED + 1024**2) as outer:
            evidence = read_json_member(outer, "digests.json")
            require(evidence["source_sha"] == run["head_sha"] and str(evidence["run_id"]) == str(run["id"])
                    and str(evidence["run_attempt"]) == attempt and evidence["target"] == target,
                    "The Inbox worker receipt does not match the verified artifact")
            name = f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
            archive = temporary / name
            copy_member(outer, name, archive, qa.MAX_COMPRESSED)
        reserve_space(profile, archive.stat().st_size + qa.MAX_UNPACKED)
        qa.install(argparse.Namespace(archive=archive, output=installed, state_dir=state, target=target,
                                     archive_sha256=evidence["archive_sha256"],
                                     manifest_sha256=evidence["manifest_sha256"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-dir", type=Path,
                        help="Build from this local worktree of the same repository")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--app-artifact", type=int, help="Open this verified macOS debug app Actions artifact")
    mode.add_argument("--reuse-bundle", action="store_true",
                      help="Open a matching installed clean bundle without building or downloading an app")
    parser.add_argument("--rebuild-desktop", action="store_true",
                        help="Build only the desktop in the shared debug cache and reuse the verified companion")
    parser.add_argument("--start-embedded", action="store_true",
                        help="Select and start Embedded through the app's macOS accessibility controls")
    parser.add_argument("--inbox-artifact", type=int, help="Install a verified offline Inbox model pack")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--profile-dir", type=Path, default=DEFAULT_OUTPUT.with_name("native-quick-start-profile"))
    args = parser.parse_args()
    if args.rebuild_desktop and args.app_artifact is None:
        parser.error("--rebuild-desktop requires --app-artifact")
    try:
        info = native.detect()
        target = info.native_target
        require(target in native.DARWIN_NATIVE_TARGETS, "This quick start supports Intel and Apple Silicon macOS")
        require(not args.output.expanduser().is_symlink(), "The app output cannot be a symlink")
        output = native.native_output(args.output, target, "debug")
        require(not args.profile_dir.expanduser().is_symlink(), "The isolated profile cannot be a symlink")
        profile = args.profile_dir.expanduser().resolve()
        home = Path.home().resolve()
        require(profile != home and not profile.is_relative_to(home / ".fleetworks")
                and not profile.is_relative_to(output)
                and not output.is_relative_to(profile),
                "Select a dedicated profile directory outside the app output")
        shared = repo_family_root()
        source_build = args.app_artifact is None and not args.reuse_bundle
        with quick_start_lock(shared):
            if not source_build:
                require_no_other_app(output / native.NATIVE_APP)
            if profile.exists():
                require_private_profile(profile)
            output.parent.mkdir(parents=True, exist_ok=True)
            app = prepare_app(args, info, target, output, shared)
            profile.mkdir(parents=True, mode=0o700, exist_ok=True)
            require_private_profile(profile)
            pid, state = open_app(app, profile, start_embedded=args.start_embedded or bool(args.inbox_artifact))
            if args.inbox_artifact is not None:
                require(embedded_running(app, pid, state), "The profile's Embedded companion must be running to add Inbox support")
                install_inbox(args.inbox_artifact, target, profile, state)
                print("Offline Inbox pack installed. Enable Backend inference in Gateway settings when desired.")
            if embedded_running(app, pid, state):
                print(f"Embedded is running in {state}")
            else:
                print('Choose Embedded, then "Start Embedded Backend".')
    except (BadZipFile, KeyError, OSError, TypeError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"fleetworks-quick-start: {error}\n")


if __name__ == "__main__":
    main()
