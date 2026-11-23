#!/usr/bin/env python3
"""Open a complete macOS Fleetworks bundle with an isolated Embedded profile."""

import argparse
from contextlib import contextmanager
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

import fleetworks_candidate as native
import fleetworks_qa_pack as qa


ROOT = native.ROOT
REPO = "xxxcess/argos-osint"
DEFAULT_OUTPUT = ROOT / ".agent-scratch" / "native-quick-start"
COMPANION = "Contents/MacOS/argos-api-host"
DESKTOP = "Contents/MacOS/fleetworks-desktop"
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
                repeat with controlRef in entire contents of window 1 of targetApp
                    if (role of controlRef as text) is "AXButton" then
                        set labelText to name of controlRef as text
                        if labelText is "Embedded. Run on this PC" then
                            click controlRef
                            exit repeat
                        else if labelText is "Start Embedded Backend" then
                            click controlRef
                            return
                        end if
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


def read_json_member(zipped, filename, limit=1024**2):
    require(zipped.getinfo(filename).file_size <= limit, f"{filename} exceeds its size limit")
    return json.loads(zipped.read(filename))


def copy_member(zipped, name, destination, limit):
    entry = zipped.getinfo(name)
    require(0 < entry.file_size <= limit and not entry.is_dir(), f"Invalid archive file {name}")
    reserve_space(destination.parent, entry.file_size)
    with zipped.open(entry) as source, destination.open("xb") as output:
        shutil.copyfileobj(source, output, length=1024**2)


def check_bundle(app, manifest, target):
    require(app.is_dir() and not app.is_symlink(), "A regular macOS app bundle is required")
    require(manifest["target"] == target and manifest["profile"] == "debug"
            and manifest.get("rune_revision") == native.locked_rune_revision(),
            "The app does not match the host debug profile or workspace Rune pin")
    require(manifest.get("companion", {}).get("relative_path") == COMPANION,
            "A full app bundle with the Embedded companion is required")
    embedded = app / "Contents" / "Resources" / "fleetworks-manifest.json"
    require(not embedded.is_symlink() and json.loads(embedded.read_text()) == manifest,
            "The app's embedded manifest does not match its receipt")
    hashes = manifest["files"]
    require({DESKTOP, COMPANION, "Contents/Info.plist"}.issubset(hashes), "The app file list is incomplete")
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


def share_agent_cache():
    shared = Path(command("git", "rev-parse", "--path-format=absolute", "--git-common-dir")).parent
    cache = ROOT / "target" / "agents"
    main_cache = shared / "target" / "agents"
    if shared == ROOT:
        return
    main_cache.mkdir(parents=True, exist_ok=True)
    cache.parent.mkdir(parents=True, exist_ok=True)
    if not cache.exists() and not cache.is_symlink():
        cache.symlink_to(main_cache, target_is_directory=True)
    require(cache.samefile(main_cache), "This worktree has a separate Cargo cache; use the main checkout")


def rebuild_desktop(app, companion, info, target, output):
    require(shutil.disk_usage(output.parent).free >= 3 * 1024**3,
            "The local desktop rebuild needs 3 GiB of free disk for linking; use the hosted desktop")
    share_agent_cache()
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
    )
    return app


def prepare_app(args, info, target, output):
    app = output / native.NATIVE_APP
    receipt = output / native.NATIVE_MANIFEST
    if not args.rebuild_desktop and receipt.is_file() and not receipt.is_symlink():
        manifest = json.loads(receipt.read_text())
        if not manifest["dirty"] and build_inputs_match(manifest["revision"]):
            check_bundle(app, manifest, target)
            return app
    require(args.app_artifact is not None, "Supply --app-artifact from a successful Mac debug packaging job")
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
                archive = temporary / name
                copy_member(outer, name, archive, native.MAX_NATIVE_ARCHIVE_BYTES)
                copy_member(outer, name + ".sha256", archive.with_name(name + ".sha256"), 1024)
        if args.rebuild_desktop:
            return rebuild_desktop(app, companion, info, target, output)
        require(not output.exists(), "Use an empty app output directory for a different packaged revision")
        staged = temporary / "bundle"
        with ZipFile(archive) as inner_zip:
            reserve_space(temporary, sum(entry.file_size for entry in inner_zip.infolist()))
        digest = archive.with_name(name + ".sha256").read_text().split()[0]
        _, inner = native.extract_native(
            archive, staged, expected_digest=digest, version=manifest["version"], target=target,
            revision=run["head_sha"], rune_revision=native.locked_rune_revision(),
            profile="debug", require_clean=True,
        )
        require(inner == manifest and "actor_catalog" in manifest["default_datasets"]["datasets"],
                "The extracted app is missing required manifest data")
        (staged / native.NATIVE_MANIFEST).write_text(json.dumps(inner) + "\n")
        staged.rename(output)
    return app


def process_table():
    return [line.split(maxsplit=2) for line in command("ps", "-ww", "-axo", "pid=,ppid=,command=").splitlines()]


def app_pid(app):
    executable = str(app / DESKTOP)
    return next((int(fields[0]) for fields in process_table()
                 if len(fields) == 3 and (fields[2] == executable or fields[2].startswith(executable + " "))), None)


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


def open_app(app, profile, *, start_embedded):
    pid = app_pid(app)
    if pid is None:
        require("fleetworks-desktop" not in [Path(line).name for line in command("ps", "-axo", "comm=").splitlines()],
                "Close the other Fleetworks app before opening this bundle")
        subprocess.run(["open", "--env", f"CFFIXED_USER_HOME={profile}", "--env", f"HOME={profile}",
                        str(app), "--args", "--local-backend-bin", str(app / COMPANION)],
                       check=True, timeout=30)
        pid = wait_for(lambda: app_pid(app), "Fleetworks did not appear after launching")
    state = profile / ".fleetworks" / "local-backend" / "embedded"
    if start_embedded and not embedded_running(app, pid, state):
        subprocess.run(["osascript", "-", str(pid)], input=EMBEDDED_BUTTONS, text=True, check=True, timeout=30)
        wait_for(lambda: embedded_running(app, pid, state), "The Embedded companion did not start for this profile")
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-artifact", type=int, help="Successful macOS debug app Actions artifact ID")
    parser.add_argument("--rebuild-desktop", action="store_true",
                        help="Build only the desktop in the shared debug cache and reuse the verified companion")
    parser.add_argument("--start-embedded", action="store_true",
                        help="Select and start Embedded through the app's macOS accessibility controls")
    parser.add_argument("--inbox-artifact", type=int, help="Install a verified offline Inbox model pack")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--profile-dir", type=Path, default=DEFAULT_OUTPUT.with_name("native-quick-start-profile"))
    args = parser.parse_args()
    try:
        info = native.detect()
        target = info.native_target
        require(target in native.DARWIN_NATIVE_TARGETS, "This quick start supports Intel and Apple Silicon macOS")
        require(not args.output.is_symlink(), "The app output cannot be a symlink")
        output = args.output.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        if args.rebuild_desktop:
            require(app_pid(output / native.NATIVE_APP) is None, "Close this app before rebuilding its bundle")
        app = prepare_app(args, info, target, output)
        require(not args.profile_dir.is_symlink(), "The isolated profile cannot be a symlink")
        profile = args.profile_dir.resolve()
        require(profile != Path.home().resolve() and not profile.is_relative_to(output)
                and not output.is_relative_to(profile),
                "Select a dedicated profile directory outside the app output")
        profile.mkdir(parents=True, mode=0o700, exist_ok=True)
        require(profile.stat().st_uid == os.getuid() and stat.S_IMODE(profile.stat().st_mode) == 0o700,
                "The isolated profile must be a private directory owned by the current user")
        pid, state = open_app(app, profile, start_embedded=args.start_embedded or bool(args.inbox_artifact))
        if args.inbox_artifact is not None:
            require(embedded_running(app, pid, state), "The profile's Embedded companion must be running to add Inbox support")
            install_inbox(args.inbox_artifact, target, profile, state)
            print("Offline Inbox pack installed. Enable Backend inference in Gateway settings when desired.")
        print(f"Opened {app}")
        if embedded_running(app, pid, state):
            print(f"Embedded is running in {state}")
        else:
            print('Choose Embedded, then "Start Embedded Backend".')
    except (BadZipFile, KeyError, OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"fleetworks-quick-start: {error}\n")


if __name__ == "__main__":
    main()
