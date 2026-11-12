#!/usr/bin/env python3
"""Build/package native app candidates and verify retained CLI payloads.

Native helpers require Python 3.9 or newer and never provision a backend,
initialize user workspaces, reset configuration or publish an artifact.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
import platform
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.parse
import zipfile
from pathlib import Path, PurePosixPath

from fleetworks_platform import NATIVE_TARGETS, detect, resolve_native_target

ROOT = Path(__file__).resolve().parents[1]
BINARIES = ("venture", *(f"fleetworks-{app}" for app in ("atlas", "brain", "jobs", "tools", "models", "profile")))
TARGETS = {
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64", r"ELF 64-bit .*x86-64"),
    "aarch64-unknown-linux-gnu": ("Linux", "aarch64", r"ELF 64-bit .*(aarch64|ARM aarch64)"),
    "x86_64-apple-darwin": ("Darwin", "x86_64", r"Mach-O 64-bit executable x86_64"),
    "aarch64-apple-darwin": ("Darwin", "arm64", r"Mach-O 64-bit executable arm64"),
}
FILE_MODES = {"LICENSE": 0o644, **{f"bin/{name}": 0o755 for name in BINARIES}}
NATIVE_BINARY = "fleetworks-desktop"
NATIVE_APP = "Fleetworks.app"
NATIVE_MANIFEST = "Fleetworks.app.manifest.json"
NATIVE_FORMAT = "fleetworks-native-1"
NATIVE_CPU_TYPES = {"x86_64-apple-darwin": 0x01000007, "aarch64-apple-darwin": 0x0100000C}
VERSION_PATTERN = r"([0-9]+)\.([0-9]+)\.([0-9]+)(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?"


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def archive_name(version, target, revision):
    return f"fleetworks-cli-{version}-{target}-{revision[:12]}.tar.gz"


def identity(version, target, revision):
    return {"format": 1, "version": version, "target": target, "revision": revision}


def tar_member(name, size, mode):
    member = tarfile.TarInfo(name)
    member.size = size
    member.mode = mode
    return member


def checksum_line(archive, digest):
    return f"{digest}  {archive.name}\n"


def package(destination, binaries, license_path, version, target, revision):
    sources = {"LICENSE": license_path, **{f"bin/{name}": binaries / name for name in BINARIES}}
    for name, path in sources.items():
        if not path.is_file() or path.is_symlink() or (FILE_MODES[name] == 0o755 and not os.access(path, os.X_OK)):
            raise ValueError(f"Missing or invalid bundle input {name}")
    manifest = {**identity(version, target, revision), "files": {name: sha256(path) for name, path in sources.items()}}
    manifest_bytes = (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode("utf-8")
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / archive_name(version, target, revision)
    with archive.open("xb") as output, gzip.GzipFile(filename="", fileobj=output, mode="wb", mtime=0) as stream:
        with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as bundle:
            bundle.addfile(tar_member("manifest.json", len(manifest_bytes), 0o644), io.BytesIO(manifest_bytes))
            for name, path in sources.items():
                with path.open("rb") as source:
                    bundle.addfile(tar_member(name, path.stat().st_size, FILE_MODES[name]), source)
    digest = sha256(archive)
    archive.with_suffix(".gz.sha256").write_text(checksum_line(archive, digest), encoding="utf-8")
    return archive, digest


def extract(archive, destination, expected_digest, version, target, revision):
    if sha256(archive) != expected_digest:
        raise ValueError("Bundle archive does not match the packaging job's SHA-256")
    if archive.with_suffix(".gz.sha256").read_text(encoding="utf-8") != checksum_line(archive, expected_digest):
        raise ValueError("Bundle checksum sidecar does not match the packaging job")
    modes = {"manifest.json": 0o644, **FILE_MODES}
    with tarfile.open(archive, mode="r:gz") as bundle:
        members = bundle.getmembers()
        if len(members) != len(modes) or {member.name for member in members} != modes.keys():
            raise ValueError("Bundle has missing, unexpected or duplicate entries")
        if any(not member.isfile() or member.mode != modes[member.name] for member in members):
            raise ValueError("Bundle entry type or permissions are invalid")
        with bundle.extractfile("manifest.json") as manifest_file:
            manifest = json.load(manifest_file)
        if not isinstance(manifest, dict):
            raise ValueError("Bundle manifest must be an object")
        hashes = manifest.pop("files", None)
        if manifest != identity(version, target, revision) or not isinstance(hashes, dict) or hashes.keys() != FILE_MODES.keys():
            raise ValueError("Bundle manifest does not match the expected revision, version and target")
        (destination / "bin").mkdir(parents=True)
        for name, mode in FILE_MODES.items():
            path = destination / name
            with bundle.extractfile(name) as source, path.open("xb") as output:
                shutil.copyfileobj(source, output)
            if sha256(path) != hashes[name]:
                raise ValueError(f"Bundle checksum mismatch for {name}")
            path.chmod(mode)


def native_pattern(target):
    system, machine, pattern = TARGETS[target]
    if (platform.system(), platform.machine()) != (system, machine):
        raise ValueError(f"Verification runner is not native for {target}")
    return pattern


def smoke(directory, target):
    pattern = native_pattern(target)
    package_bin = directory / "bin"
    home = directory / "home"
    home.mkdir()
    environment = {"HOME": str(home), "PATH": os.environ.get("PATH", os.defpath), "LC_ALL": "C"}
    for name in BINARIES:
        binary = package_bin / name
        description = subprocess.check_output(["file", "-b", str(binary)], text=True, timeout=10)
        if not re.search(pattern, description, re.IGNORECASE):
            raise ValueError(f"{name} has the wrong native format: {description.strip()}")
        subprocess.run([str(binary), "--help"], stdout=subprocess.DEVNULL, env=environment, timeout=20, check=True)
    workspace = directory / "review workspace"
    command = [str(package_bin / "venture"), "--workspace", str(workspace)]
    subprocess.run([*command, "init"], stdout=subprocess.DEVNULL, env=environment, timeout=20, check=True)
    output = subprocess.check_output([*command, "client", "profile", "workspace", "--json"], env=environment, timeout=20)
    if json.loads(output) != {"workspace": str(workspace)}:
        raise ValueError("Venture did not launch its packaged Profile sibling with the requested workspace")
    view = subprocess.check_output([*command, "client", "profile"], input="workspace\nquit\n", text=True, env=environment, timeout=20)
    if "Fleetworks Profile" not in view or str(workspace) not in view:
        raise ValueError("Venture did not open the packaged interactive Profile view")


def _run(command, *, runner=None, root=ROOT, environment=None, timeout=60):
    result = (runner or subprocess.run)(command, cwd=root, env=environment,
                                       capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        diagnostic = (result.stderr or result.stdout or f"exit {result.returncode}").strip()
        raise ValueError(f"{command[0]} failed: {diagnostic[-4000:]}")
    return result.stdout.strip()


def native_archive_name(version, target, revision, run_id, attempt):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,79}", str(run_id)):
        raise ValueError("Candidate run identity must be a bounded filename-safe value")
    if not re.fullmatch(r"[1-9][0-9]{0,5}", str(attempt)):
        raise ValueError("Candidate run attempt must be a positive bounded integer")
    return f"Fleetworks-{version}-{revision[:12]}-{run_id}.{attempt}-{target}.zip"


def apple_versions(version, bundle_version=None):
    """Map workspace SemVer to Apple's numeric version fields.

    The full workspace version remains in the manifest. The default bundle
    first component is SemVer major + 1, so pre-1.0 workspace versions retain
    a positive Apple build version. CI may supply its reviewed numeric value.
    """
    match = re.fullmatch(VERSION_PATTERN, version)
    if not match:
        raise ValueError("Invalid release version")
    major, minor, patch = map(int, match.groups())
    short = f"{major}.{minor}.{patch}"
    if bundle_version is None:
        if major >= 9999 or minor > 99 or patch > 99:
            raise ValueError("Workspace version needs an explicit Apple-compatible --bundle-version")
        bundle_version = f"{major + 1}.{minor}.{patch}"
    if not re.fullmatch(r"[1-9][0-9]{0,3}(?:\.(?:0|[1-9][0-9]?)){0,2}", bundle_version):
        raise ValueError("Invalid Apple numeric bundle version")
    return short, bundle_version


def cargo_native_command(target, profile, *, agent=False, root=ROOT, implicit_host=False):
    if target not in NATIVE_TARGETS or profile not in ("debug", "release"):
        raise ValueError("Unsupported native target or profile")
    command = ([sys.executable, str(root / "scripts/agent_cargo.py")] if agent else ["cargo"])
    command += ["build", "--locked", "-p", NATIVE_BINARY, "--bin", NATIVE_BINARY,
                "--message-format=json-render-diagnostics"]
    if not implicit_host:
        command += ["--target", target]
    if profile == "release":
        command.append("--release")
    return command


def cargo_executable(output, *, name=NATIVE_BINARY, kind="bin"):
    """Resolve the scoped executable from successful Cargo JSON output."""
    found = set()
    finished = False
    for line in output.splitlines():
        if not line.strip():
            continue
        try:
            message = json.loads(line)
        except (TypeError, json.JSONDecodeError) as error:
            raise ValueError("Cargo did not return valid JSON artifact output") from error
        if message.get("reason") == "build-finished":
            finished = message.get("success") is True
        target = message.get("target", {})
        package_id = message.get("package_id", "")
        package_matches = bool(re.search(r"(?:#|^)fleetworks-desktop(?:@|\s)", package_id))
        if not package_matches and "#" in package_id:
            location, package_version = package_id.rsplit("#", 1)
            package_matches = bool(re.fullmatch(VERSION_PATTERN, package_version)) and \
                Path(urllib.parse.unquote(urllib.parse.urlparse(location).path)).name == NATIVE_BINARY
        if message.get("reason") == "compiler-artifact" and target.get("name") == name \
                and kind in target.get("kind", []) and package_matches and message.get("executable"):
            found.add(message["executable"])
    if not finished or len(found) != 1:
        raise ValueError(f"Successful Cargo output must identify exactly one {name} executable")
    executable = Path(found.pop())
    if not executable.is_absolute():
        raise ValueError("Cargo executable path must be absolute")
    return executable


def locked_rune_revision(root=ROOT):
    revisions = {}
    for block in (root / "Cargo.lock").read_text(encoding="utf-8").split("[[package]]"):
        name = re.search(r'^name = "(rune|rune-kit)"$', block, re.MULTILINE)
        source = re.search(r'^source = "git\+[^"\n]+#([0-9a-f]{40})"$', block, re.MULTILINE)
        if name and source:
            revisions[name.group(1)] = source.group(1)
    if set(revisions) != {"rune", "rune-kit"} or len(set(revisions.values())) != 1:
        raise ValueError("Cargo.lock must pin rune and rune-kit to the same exact Git revision")
    return revisions["rune"]


def repository_identity(root=ROOT, *, revision=None, runner=None):
    actual = _run(["git", "rev-parse", "HEAD"], runner=runner, root=root)
    if not re.fullmatch(r"[0-9a-f]{40}", actual):
        raise ValueError("Repository must report an exact Git revision")
    if revision is not None and revision != actual:
        raise ValueError("Requested revision does not match the checkout HEAD")
    dirty = bool(_run(["git", "status", "--porcelain", "--untracked-files=normal"], runner=runner, root=root))
    return actual, dirty


def validate_toolchain(info, root=ROOT):
    configuration = (root / "rust-toolchain.toml").read_text(encoding="utf-8")
    pin = re.search(r'^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', configuration, re.MULTILINE)
    if not pin:
        raise ValueError("Native helper requires an explicit repository Rust version pin")
    if info.rust_release != pin.group(1):
        raise ValueError(f"Selected Rust {info.rust_release or 'unavailable'} differs from repository pin {pin.group(1)}; remove conflicting RUSTUP_TOOLCHAIN/directory overrides explicitly")
    return pin.group(1)


def native_output(directory, target, profile, *, root=ROOT):
    output = Path(directory) if directory else root / "dist/native" / target / profile
    output = output.expanduser().resolve()
    forbidden = (Path("/Applications"), Path.home() / "Applications", Path.home() / ".fleetworks", Path("/System"), Path("/Library"))
    if output == Path("/") or any(output == path or path in output.parents for path in forbidden):
        raise ValueError("Native output must be a review directory outside installed apps, system paths and the real workspace container")
    return output


def macho_architecture(binary):
    if binary.is_symlink() or not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("Native binary must be a regular executable file")
    with binary.open("rb") as source:
        header = source.read(32)
    if len(header) != 32 or header[:4] not in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf"):
        raise ValueError("Native binary is not a thin 64-bit Mach-O executable")
    endian = "<" if header[:4] == b"\xcf\xfa\xed\xfe" else ">"
    fields = struct.unpack(endian + "8I", header)
    if fields[3] != 2:
        raise ValueError("Mach-O input is not an executable")
    return fields[1]


def inspect_native_binary(binary, target, *, runner=None):
    if macho_architecture(binary) != NATIVE_CPU_TYPES[target]:
        raise ValueError(f"Native binary has the wrong architecture for {target}")
    listing = _run(["otool", "-L", str(binary)], runner=runner)
    lines = listing.splitlines()
    if not lines or not lines[0].rstrip().endswith(":"):
        raise ValueError("otool did not report native library dependencies")
    libraries = []
    for line in lines[1:]:
        library = line.strip().split(" (", 1)[0]
        if not library:
            continue
        # This app is statically linked apart from Apple-provided libraries.
        # A newly introduced dylib needs explicit bundling, not a host path.
        if not library.startswith(("/System/Library/", "/usr/lib/")):
            raise ValueError(f"Native bundle has an unbundled/non-system library: {library}")
        libraries.append(library)
    return libraries


def _write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def _resource_files(resources):
    if resources.is_symlink() or not resources.is_dir():
        raise ValueError("Native resource directory is missing or a symlink")
    files = []
    for path in sorted(resources.rglob("*")):
        if path.is_symlink():
            raise ValueError("Native resources cannot include symlinks")
        if path.is_file() and path.name != "Info.plist":
            if path.suffix.lower() not in (".icns", ".png", ".jpg", ".jpeg", ".svg", ".metallib"):
                raise ValueError(f"Unexpected native resource type: {path.name}")
            files.append(path)
    return files


def _zip_app(app, archive):
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        for path in [app, *sorted(app.rglob("*"))]:
            relative = path.relative_to(app.parent).as_posix()
            directory = path.is_dir()
            entry = zipfile.ZipInfo(relative + ("/" if directory else ""), date_time=(1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            mode = (stat.S_IFDIR | 0o755) if directory else (stat.S_IFREG | (0o755 if os.access(path, os.X_OK) else 0o644))
            entry.external_attr = mode << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            if directory:
                output.writestr(entry, b"")
            else:
                with path.open("rb") as source, output.open(entry, "w") as destination:
                    shutil.copyfileobj(source, destination, length=1024 * 1024)


def package_native(destination, binary, resources, *, version, target, revision, profile,
                   rune_revision, toolchain, dirty, minimum_os=None, bundle_version=None,
                   run_id=None, attempt="1", workflow="local", archive=False,
                   cargo_lock_sha256="", runner=None):
    """Stage a checked complete app and replace only this helper's outputs.

    Packaging/architecture/library checks are not a GUI, signing, notarization
    or minimum-OS pass. Existing unrelated apps and immutable ZIP identities
    are never overwritten.
    """
    if target not in NATIVE_TARGETS or profile not in ("debug", "release"):
        raise ValueError("Unsupported native target or profile")
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or not re.fullmatch(r"[0-9a-f]{40}", rune_revision):
        raise ValueError("Native packaging requires exact source and Rune revisions")
    short_version, build_version = apple_versions(version, bundle_version)
    resources = Path(resources)
    if resources.is_symlink():
        raise ValueError("Native resource directory cannot be a symlink")
    resources = resources.resolve()
    resource_files = _resource_files(resources)
    plist_path = resources / "Info.plist"
    if plist_path.is_symlink() or not plist_path.is_file():
        raise ValueError("Native resources require a regular Info.plist")
    with plist_path.open("rb") as source:
        plist = plistlib.load(source)
    if not isinstance(plist, dict) or not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", str(plist.get("CFBundleIdentifier", ""))):
        raise ValueError("Native Info.plist requires a valid bundle identifier")
    resource_minimum = plist.get("LSMinimumSystemVersion")
    minimum_os = minimum_os or resource_minimum
    if not isinstance(minimum_os, str) or not re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", minimum_os):
        raise ValueError("Native minimum macOS must be explicit in Info.plist or --minimum-macos")
    if isinstance(resource_minimum, str) and re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", resource_minimum):
        selected = tuple(map(int, minimum_os.split("."))) + (0,) * (3 - len(minimum_os.split(".")))
        required = tuple(map(int, resource_minimum.split("."))) + (0,) * (3 - len(resource_minimum.split(".")))
        if selected < required:
            raise ValueError("--minimum-macos cannot be lower than the app resource minimum")
    icon = plist.get("CFBundleIconFile")
    if icon:
        icon_name = icon if str(icon).endswith(".icns") else str(icon) + ".icns"
        if Path(icon_name).name != icon_name or not (resources / icon_name).is_file():
            raise ValueError("Declared native app icon is missing")
    binary = Path(binary)
    if binary.is_symlink():
        raise ValueError("Native binary cannot be a symlink")
    binary = binary.resolve()
    libraries = inspect_native_binary(binary, target, runner=runner)
    destination = native_output(destination, target, profile)
    run_id = str(run_id or f"local-{time.time_ns()}")
    archive_file = native_archive_name(version, target, revision, run_id, attempt)
    if archive and (destination / archive_file).exists():
        raise ValueError("Native candidate archive identity already exists; it is immutable")
    manifest = {
        "format": NATIVE_FORMAT, "version": version, "revision": revision,
        "dirty": bool(dirty), "target": target, "architecture": NATIVE_TARGETS[target],
        "profile": profile, "rune_revision": rune_revision, "toolchain": toolchain,
        "minimum_macos": minimum_os, "bundle_identifier": plist["CFBundleIdentifier"],
        "bundle_short_version": short_version, "bundle_version": build_version,
        "workflow": workflow, "run_id": run_id, "run_attempt": str(attempt),
        "backend_compatibility": {"api_version": 1, "required_connect_contract": 1},
        "dependency_provenance": {"cargo_lock_sha256": cargo_lock_sha256},
        "signing": {"status": "unsigned-review", "notarization": "not_requested", "verified": False},
        "verification": {"architecture": "pass", "libraries": "pass", "bundle": "pass",
                         "gui_launch": "not_run", "minimum_os": "not_tested", "product_tests": "not_recorded"},
        "libraries": libraries,
    }
    old_app = destination / NATIVE_APP
    old_manifest = destination / NATIVE_MANIFEST
    if old_app.exists() or old_app.is_symlink() or old_manifest.exists() or old_manifest.is_symlink():
        if old_app.is_symlink() or old_manifest.is_symlink() or not old_app.is_dir() or not old_manifest.is_file():
            raise ValueError("Existing native output is not an owned app/manifest pair")
        previous = json.loads(old_manifest.read_text(encoding="utf-8"))
        if not isinstance(previous, dict) or previous.get("format") != NATIVE_FORMAT or previous.get("bundle_identifier") != manifest["bundle_identifier"]:
            raise ValueError("Refusing to replace an unrelated native app")
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".fleetworks-stage-", dir=destination) as temporary:
        stage = Path(temporary)
        app = stage / NATIVE_APP
        contents = app / "Contents"
        executable = contents / "MacOS" / NATIVE_BINARY
        executable.parent.mkdir(parents=True)
        bundled_resources = contents / "Resources"
        bundled_resources.mkdir()
        shutil.copyfile(binary, executable)
        executable.chmod(0o755)
        for path in resource_files:
            target_path = bundled_resources / path.relative_to(resources)
            target_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target_path)
            target_path.chmod(0o644)
        plist.update(CFBundleExecutable=NATIVE_BINARY, CFBundleName="Fleetworks",
                     CFBundleDisplayName="Fleetworks", CFBundlePackageType="APPL",
                     CFBundleShortVersionString=short_version, CFBundleVersion=build_version,
                     LSMinimumSystemVersion=minimum_os)
        with (contents / "Info.plist").open("wb") as output:
            plistlib.dump(plist, output, sort_keys=True)
        manifest["files"] = {path.relative_to(app).as_posix(): sha256(path)
                             for path in sorted(app.rglob("*")) if path.is_file()}
        _write_json(bundled_resources / "fleetworks-manifest.json", manifest)
        _write_json(stage / NATIVE_MANIFEST, manifest)
        outputs = [NATIVE_APP, NATIVE_MANIFEST]
        digest = None
        if archive:
            _zip_app(app, stage / archive_file)
            digest = sha256(stage / archive_file)
            (stage / (archive_file + ".sha256")).write_text(checksum_line(stage / archive_file, digest), encoding="utf-8")
            outputs.extend([archive_file, archive_file + ".sha256"])
        backup = stage / "previous"
        backup.mkdir()
        replaced, installed = [], []
        try:
            for name in outputs:
                existing = destination / name
                if existing.exists() or existing.is_symlink():
                    if name not in (NATIVE_APP, NATIVE_MANIFEST):
                        raise ValueError("Native candidate archive identity already exists; it is immutable")
                    os.replace(existing, backup / name)
                    replaced.append(name)
                os.replace(stage / name, existing)
                installed.append(name)
        except BaseException:
            for name in reversed(installed):
                path = destination / name
                if path.is_dir() and not path.is_symlink():
                    shutil.rmtree(path)
                elif path.exists() or path.is_symlink():
                    path.unlink()
            for name in reversed(replaced):
                os.replace(backup / name, destination / name)
            raise
    return {"app": str(destination / NATIVE_APP), "manifest": str(destination / NATIVE_MANIFEST),
            "archive": str(destination / archive_file) if archive else None, "sha256": digest,
            "identity": manifest}


def open_native(app, info, target, *, runner=None):
    if info.system != "Darwin" or target != info.native_target:
        raise ValueError("--open requires an artifact matching this physical Mac architecture")
    commands = _run(["ps", "-axo", "comm="], runner=runner)
    if any(Path(line.strip()).name == NATIVE_BINARY for line in commands.splitlines()):
        return "Fleetworks is already running; close it normally, then open the new printed app path. No process was stopped."
    _run(["open", str(app)], runner=runner)
    return "Requested Launch Services opening; actual GUI readiness and authenticated operations remain unverified."


def extract_native(archive, destination, *, expected_digest, version, target, revision,
                   rune_revision, profile=None, require_clean=False, runner=None):
    """Validate a downloaded immutable ZIP before bounded private extraction.

    The caller obtains the expected digest/identity from its trusted packaging
    job or review record, not from the archive being verified. Extraction does
    not install or launch the app and cannot prove signing or GUI behavior.
    """
    archive, destination = Path(archive), Path(destination)
    if not re.fullmatch(r"[0-9a-f]{64}", expected_digest):
        raise ValueError("A packaging-job SHA-256 is required for native verification")
    if target not in NATIVE_TARGETS or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("An exact native target and source revision are required")
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size > 512 * 1024 * 1024:
        raise ValueError("Native archive must be a bounded regular file")
    if sha256(archive) != expected_digest:
        raise ValueError("Native archive does not match the packaging job's SHA-256")
    if archive.with_suffix(archive.suffix + ".sha256").read_text(encoding="utf-8") != checksum_line(archive, expected_digest):
        raise ValueError("Native archive checksum sidecar does not match the packaging job")
    if destination.is_symlink() or (destination.exists() and (not destination.is_dir() or any(destination.iterdir()))):
        raise ValueError("Native extraction requires an empty private directory")
    embedded_name = f"{NATIVE_APP}/Contents/Resources/fleetworks-manifest.json"
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        names = [entry.filename for entry in entries]
        if not entries or len(entries) > 10000 or len(set(names)) != len(names):
            raise ValueError("Native archive has missing, duplicate or excessive entries")
        if sum(entry.file_size for entry in entries) > 1024 * 1024 * 1024:
            raise ValueError("Native archive exceeds its uncompressed size limit")
        for entry in entries:
            path = PurePosixPath(entry.filename)
            if len(entry.filename) > 1024 or any(ord(char) < 32 for char in entry.filename) \
                    or path.is_absolute() or ".." in path.parts or "\\" in entry.filename \
                    or not path.parts or path.parts[0] != NATIVE_APP \
                    or entry.filename.rstrip("/") != path.as_posix():
                raise ValueError("Native archive contains an unsafe or unexpected path")
            mode = entry.external_attr >> 16
            expected = (stat.S_IFDIR | 0o755) if entry.is_dir() else \
                (stat.S_IFREG | (0o755 if entry.filename == f"{NATIVE_APP}/Contents/MacOS/{NATIVE_BINARY}" else 0o644))
            if mode != expected or entry.flag_bits & 1:
                raise ValueError("Native archive entry type, permissions or encryption is invalid")
        if embedded_name not in names or zipped.getinfo(embedded_name).file_size > 1024 * 1024:
            raise ValueError("Native archive has no bounded embedded manifest")
        manifest = json.loads(zipped.read(embedded_name))
        if not isinstance(manifest, dict) or manifest.get("format") != NATIVE_FORMAT \
                or manifest.get("version") != version or manifest.get("revision") != revision \
                or manifest.get("target") != target or manifest.get("rune_revision") != rune_revision \
                or manifest.get("architecture") != NATIVE_TARGETS[target] \
                or (profile is not None and manifest.get("profile") != profile):
            raise ValueError("Native manifest does not match expected version, revision, target and Rune pin")
        if require_clean and manifest.get("dirty") is not False:
            raise ValueError("Verified CI/release artifact must have a clean source identity")
        if native_archive_name(version, target, revision, manifest.get("run_id"), manifest.get("run_attempt")) != archive.name:
            raise ValueError("Native archive filename does not match its immutable identity")
        hashes = manifest.get("files")
        if not isinstance(hashes, dict) or not hashes:
            raise ValueError("Native manifest has no file hashes")
        file_names = {f"{NATIVE_APP}/{name}" for name in hashes} | {embedded_name}
        if file_names != {entry.filename for entry in entries if not entry.is_dir()}:
            raise ValueError("Native archive files do not match the manifest")
        directories = {str(parent) + "/" for name in file_names
                       for parent in PurePosixPath(name).parents if str(parent) != "."}
        if any(entry.filename not in directories for entry in entries if entry.is_dir()):
            raise ValueError("Native archive contains unexpected empty directories")
        for name, expected in hashes.items():
            if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
                raise ValueError("Native manifest contains an invalid file digest")
            with zipped.open(f"{NATIVE_APP}/{name}") as source:
                digest = hashlib.sha256()
                for block in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(block)
            if digest.hexdigest() != expected:
                raise ValueError(f"Native bundle file digest mismatch: {name}")
        destination.mkdir(parents=True, exist_ok=True)
        for entry in entries:
            target_path = destination.joinpath(*PurePosixPath(entry.filename).parts)
            if entry.is_dir():
                target_path.mkdir(parents=True, exist_ok=True)
                target_path.chmod(0o755)
            else:
                target_path.parent.mkdir(parents=True, exist_ok=True)
                with zipped.open(entry) as source, target_path.open("xb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                target_path.chmod((entry.external_attr >> 16) & 0o777)
    app = destination / NATIVE_APP
    with (app / "Contents/Info.plist").open("rb") as source:
        plist = plistlib.load(source)
    short, build = apple_versions(version, manifest.get("bundle_version"))
    expected_plist = {"CFBundleExecutable": NATIVE_BINARY, "CFBundlePackageType": "APPL",
                      "CFBundleIdentifier": manifest.get("bundle_identifier"),
                      "CFBundleShortVersionString": short, "CFBundleVersion": build,
                      "LSMinimumSystemVersion": manifest.get("minimum_macos")}
    if any(plist.get(key) != value for key, value in expected_plist.items()):
        raise ValueError("Native Info.plist does not match its manifest")
    libraries = inspect_native_binary(app / "Contents/MacOS" / NATIVE_BINARY, target, runner=runner)
    if libraries != manifest.get("libraries"):
        raise ValueError("Native library dependencies do not match the manifest")
    return app, manifest


def native_main(args):
    if sys.version_info < (3, 9):
        raise ValueError("Native helper requires Python 3.9 or newer")
    info = detect()
    target = resolve_native_target(info, args.target)
    if args.open and target != info.native_target:
        raise ValueError("--open cannot launch an artifact for the other physical Mac architecture")
    toolchain = validate_toolchain(info)
    destination = native_output(args.output, target, args.profile)
    command = cargo_native_command(target, args.profile, agent=args.agent, implicit_host=args.target == "host") if args.command == "build-native" else None
    if args.dry_run:
        print(json.dumps({"target": target, "platform": info.as_dict(), "profile": args.profile,
                          "build_command": command, "app": str(destination / NATIVE_APP),
                          "archive_requested": args.archive, "open_requested": args.open,
                          "prerequisites": "SDK/package inputs checked during build/package, not this dry run"}, indent=2))
        return 0
    version = (ROOT / "version.txt").read_text(encoding="utf-8").strip()
    revision, dirty = repository_identity(revision=args.revision)
    if (args.require_clean or os.environ.get("CI") == "true") and dirty:
        raise ValueError("CI/release native candidates require a clean exact-revision checkout")
    rune_revision = locked_rune_revision()
    resources = args.resources or ROOT / "apps/fleetworks-desktop/resources"
    if command:
        _run(["xcode-select", "-p"])
        _run(["xcrun", "--sdk", "macosx", "--show-sdk-path"])
        environment = dict(os.environ)
        environment.pop("ARGOS_EMBED", None)
        environment["RUSTUP_AUTO_INSTALL"] = "0"
        binary = cargo_executable(_run(command, environment=environment, timeout=3600))
    else:
        binary = args.binary
    result = package_native(destination, binary, resources, version=version, target=target,
                            revision=revision, profile=args.profile, rune_revision=rune_revision,
                            toolchain=toolchain, dirty=dirty, minimum_os=args.minimum_macos,
                            bundle_version=args.bundle_version, run_id=args.run_id,
                            attempt=args.run_attempt, workflow=args.workflow, archive=args.archive,
                            cargo_lock_sha256=sha256(ROOT / "Cargo.lock"))
    print(result["app"])
    print(json.dumps({key: value for key, value in result.items() if key != "identity"}, indent=2))
    if args.open:
        print(open_native(Path(result["app"]), info, target))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pack = commands.add_parser("pack")
    verify = commands.add_parser("verify")
    for command in (pack, verify):
        command.add_argument("--directory", type=Path, required=True)
        command.add_argument("--revision", required=True)
        command.add_argument("--target", choices=TARGETS, required=True)
    pack.add_argument("--binaries", type=Path, required=True)
    pack.add_argument("--github-output", type=Path)
    verify.add_argument("--sha256", required=True)
    build_native = commands.add_parser("build-native", help="Build and package the API-only Mac app; never starts a backend")
    pack_native = commands.add_parser("package-native", help="Package a built Mac executable using the same checked app packager")
    for command in (build_native, pack_native):
        command.add_argument("--target", choices=("host", *NATIVE_TARGETS), default="host")
        command.add_argument("--profile", choices=("debug", "release"), default="debug")
        command.add_argument("--output", type=Path)
        command.add_argument("--resources", type=Path)
        command.add_argument("--revision")
        command.add_argument("--minimum-macos")
        command.add_argument("--bundle-version")
        command.add_argument("--run-id", default=os.environ.get("GITHUB_RUN_ID"))
        command.add_argument("--run-attempt", default=os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
        command.add_argument("--workflow", default=os.environ.get("GITHUB_WORKFLOW", "local"))
        command.add_argument("--archive", action="store_true")
        command.add_argument("--open", action="store_true")
        command.add_argument("--dry-run", action="store_true")
        command.add_argument("--require-clean", action="store_true")
        command.add_argument("--agent", action="store_true", help="Use scripts/agent_cargo.py and target/agents for a build")
    pack_native.add_argument("--binary", type=Path, required=True)
    verify_native = commands.add_parser("verify-native", help="Verify an exact downloaded app ZIP; does not install or launch by default")
    verify_native.add_argument("--archive", type=Path, required=True)
    verify_native.add_argument("--sha256", required=True)
    verify_native.add_argument("--revision", required=True)
    verify_native.add_argument("--target", choices=NATIVE_TARGETS, required=True)
    verify_native.add_argument("--profile", choices=("debug", "release"))
    verify_native.add_argument("--rune-revision")
    verify_native.add_argument("--directory", type=Path, help="Retain verified extraction in this new/empty review directory")
    verify_native.add_argument("--require-clean", action="store_true")
    verify_native.add_argument("--open", action="store_true")
    args = parser.parse_args()
    try:
        if args.command in ("build-native", "package-native"):
            return native_main(args)
        if args.command == "verify-native":
            version = (ROOT / "version.txt").read_text(encoding="utf-8").strip()
            if args.open and not args.directory:
                raise ValueError("--open needs --directory so the exact verified app remains available for GUI review")
            options = dict(expected_digest=args.sha256, version=version, target=args.target,
                           revision=args.revision, rune_revision=args.rune_revision or locked_rune_revision(),
                           profile=args.profile, require_clean=args.require_clean)
            if args.directory:
                destination = native_output(args.directory, args.target, args.profile or "release")
                app, manifest = extract_native(args.archive, destination, **options)
            else:
                with tempfile.TemporaryDirectory(prefix="fleetworks-native-verify-") as temporary:
                    app, manifest = extract_native(args.archive, Path(temporary), **options)
            if args.open:
                print(open_native(app, detect(), args.target))
            print(json.dumps({"archive": str(args.archive.resolve()), "sha256": args.sha256,
                              "revision": manifest["revision"], "target": manifest["target"],
                              "bundle_checks": "pass", "gui_launch": "not_run",
                              "signing": manifest["signing"]}, indent=2))
            return 0
        version = (ROOT / "version.txt").read_text(encoding="utf-8").strip()
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", version):
            raise ValueError("Invalid release version")
        if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
            raise ValueError("An exact Git revision is required")
        native_pattern(args.target)
        if args.command == "pack":
            archive, digest = package(args.directory, args.binaries, ROOT / "LICENSE", version, args.target, args.revision)
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as output:
                    print(f"sha256={digest}", file=output)
            print(checksum_line(archive, digest), end="")
        else:
            if not re.fullmatch(r"[0-9a-f]{64}", args.sha256):
                raise ValueError("The packaging job must supply a SHA-256")
            archive = args.directory / archive_name(version, args.target, args.revision)
            with tempfile.TemporaryDirectory(prefix="fleetworks-candidate-") as temporary:
                directory = Path(temporary).resolve()
                extract(archive, directory, args.sha256, version, args.target, args.revision)
                smoke(directory, args.target)
            print(f"Verified and ran {archive.name} ({args.sha256})")
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError, zipfile.BadZipFile, plistlib.InvalidFileException) as error:
        print(f"fleetworks-candidate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
