#!/usr/bin/env python3
"""Build, package and verify native Mac and Linux x86_64 app candidates.

Native helpers require Python 3.9 or newer and never provision a backend,
initialize user workspaces, reset configuration or publish an artifact.
"""

import argparse
import codecs
import gzip
import hashlib
import io
import json
import os
import plistlib
import re
import shutil
import shlex
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.parse
import zipfile
from pathlib import Path, PurePosixPath

from fleetworks_platform import DARWIN_NATIVE_TARGETS, LINUX_NATIVE_TARGETS, NATIVE_TARGETS, detect, resolve_native_target

ROOT = Path(__file__).resolve().parents[1]
NATIVE_BINARY = "fleetworks-desktop"
COMPANION_PACKAGE = "argos-api-host"
COMPANION_BINARY = "argos-api-host"
NATIVE_APP = "Fleetworks.app"
NATIVE_MANIFEST = "Fleetworks.app.manifest.json"
LINUX_APP = "Fleetworks-linux-x86_64"
LINUX_MANIFEST = "Fleetworks-linux-x86_64.manifest.json"
NATIVE_FORMAT = "fleetworks-native-1"
NATIVE_CPU_TYPES = {"x86_64-apple-darwin": 0x01000007, "aarch64-apple-darwin": 0x0100000C}
LINUX_ELF_MACHINES = {"x86_64-unknown-linux-gnu": 62}
DISALLOWED_LINUX_LIBRARIES = ("libssl", "libcrypto", "librocksdb", "libsurreal", "libtorch", "libonnxruntime")
FORBIDDEN_CLIENT_ENGINE_CRATES = ("surrealdb", "rocksdb", "rusqlite", "tantivy", "lancedb", "argos-osint-core")
CLIENT_ENGINE_FREE_PACKAGES = ("fleetworks-desktop", "fleetworks-contracts", "fleetworks-workstation")
ALLOWED_RUSQLITE_PARENTS = ("fleetworks-jobs",)
MAX_NATIVE_UNPACKED_BYTES = 1024 * 1024 * 1024
MAX_NATIVE_ARCHIVE_BYTES = 512 * 1024 * 1024
DIAGNOSTIC_STARTUP_PROBE_SECONDS = 2.0
VERSION_PATTERN = r"([0-9]+)\.([0-9]+)\.([0-9]+)(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?"
ARTIFACT_VERSION_PATTERN = (
    r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def checksum_line(archive, digest):
    return f"{digest}  {archive.name}\n"


def _run(command, *, runner=None, root=ROOT, environment=None, timeout=60):
    result = (runner or subprocess.run)(command, cwd=root, env=environment,
                                       capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        diagnostic = (result.stderr or result.stdout or f"exit {result.returncode}").strip()
        raise ValueError(f"{command[0]} failed: {diagnostic[-4000:]}")
    return result.stdout.strip()


def _progress(message):
    print(f"fleetworks-candidate: {message}", file=sys.stderr, flush=True)


def _run_cargo(command, *, root=ROOT, environment=None, timeout=3600):
    """Tee Cargo progress immediately while retaining its exact JSON artifact stream.

    Cargo writes human progress/rendered compiler diagnostics to stderr. Drain
    that pipe independently so neither a quiet stdout nor a large diagnostic
    blocks progress. Keep only the diagnostic tail; stdout retains the existing
    successful-artifact validation contract.
    """
    diagnostic = [""]
    display_errors = []
    stream = sys.stderr
    with tempfile.TemporaryFile() as output:
        with subprocess.Popen(command, cwd=root, env=environment,
                              stdout=output, stderr=subprocess.PIPE) as process:
            def forward_stderr():
                decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
                while True:
                    chunk = process.stderr.read1(4096)
                    rendered = decoder.decode(chunk, final=not chunk)
                    diagnostic[0] = (diagnostic[0] + rendered)[-4000:]
                    if rendered:
                        try:
                            stream.write(rendered)
                            stream.flush()
                        except (OSError, ValueError) as error:
                            if not display_errors:
                                display_errors.append(error)
                    if not chunk:
                        break

            progress = threading.Thread(target=forward_stderr, daemon=True)
            progress.start()
            try:
                process.wait(timeout=timeout)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                process.kill()
                process.wait()
                raise
            finally:
                progress.join()
                process.stderr.close()
            output.seek(0)
            stdout = output.read().decode("utf-8", errors="replace").strip()
            if process.returncode:
                detail = (diagnostic[0] or stdout or f"exit {process.returncode}").strip()
                raise ValueError(f"{command[0]} failed: {detail[-4000:]}")
            if display_errors:
                raise OSError("Could not display Cargo progress") from display_errors[0]
            return stdout


def native_bundle_name(target):
    return LINUX_APP if target in LINUX_NATIVE_TARGETS else NATIVE_APP


def native_manifest_name(target):
    return LINUX_MANIFEST if target in LINUX_NATIVE_TARGETS else NATIVE_MANIFEST


def native_archive_name(version, target, revision, run_id, attempt):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,79}", str(run_id)):
        raise ValueError("Candidate run identity must be a bounded filename-safe value")
    if not re.fullmatch(r"[1-9][0-9]{0,5}", str(attempt)):
        raise ValueError("Candidate run attempt must be a positive bounded integer")
    extension = ".tar.gz" if target in LINUX_NATIVE_TARGETS else ".zip"
    return f"Fleetworks-{version}-{revision[:12]}-{run_id}.{attempt}-{target}{extension}"


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


def artifact_version(override=None, *, root=ROOT):
    """Select an explicit channel artifact SemVer without rewriting the source."""
    version = override if override is not None else (root / "version.txt").read_text(encoding="utf-8").strip()
    if not isinstance(version, str) or not re.fullmatch(ARTIFACT_VERSION_PATTERN, version):
        raise ValueError("Invalid release version")
    # Artifact versions appear in ZIP paths as well as manifests. Bound the
    # explicit override before any build/package operation or filesystem write.
    if len(version) > 160:
        raise ValueError("Release version exceeds the artifact identity limit")
    return version


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


def cargo_companion_command(target, profile, *, agent=False, root=ROOT, implicit_host=False,
                            features=None, no_default_features=False):
    """Build command for the companion local-IPC backend binary."""
    if target not in NATIVE_TARGETS or profile not in ("debug", "release"):
        raise ValueError("Unsupported native target or profile")
    command = ([sys.executable, str(root / "scripts/agent_cargo.py")] if agent else ["cargo"])
    command += ["build", "--locked", "-p", COMPANION_PACKAGE, "--bin", COMPANION_BINARY,
                "--message-format=json-render-diagnostics"]
    if no_default_features:
        command.append("--no-default-features")
    if features:
        command += ["--features", features]
    if not implicit_host:
        command += ["--target", target]
    if profile == "release":
        command.append("--release")
    return command


def cargo_executable(output, *, name=NATIVE_BINARY, package=NATIVE_BINARY, kind="bin"):
    """Resolve the scoped executable from successful Cargo JSON output."""
    found = set()
    finished = False
    package_re = re.compile(rf"(?:#|^){re.escape(package)}(?:@|\s)")
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
        package_matches = bool(package_re.search(package_id))
        if not package_matches and "#" in package_id:
            location, package_version = package_id.rsplit("#", 1)
            dir_name = "api-host" if package == COMPANION_PACKAGE else package
            package_matches = bool(re.fullmatch(VERSION_PATTERN, package_version)) and \
                Path(urllib.parse.unquote(urllib.parse.urlparse(location).path)).name in (package, dir_name)
        if message.get("reason") == "compiler-artifact" and target.get("name") == name \
                and kind in target.get("kind", []) and package_matches and message.get("executable"):
            found.add(message["executable"])
    if not finished or len(found) != 1:
        raise ValueError(f"Successful Cargo output must identify exactly one {name} executable")
    executable = Path(found.pop())
    if not executable.is_absolute():
        raise ValueError("Cargo executable path must be absolute")
    return executable


def _parse_cargo_tree_crates(listing):
    """Extract distinct crate names from `cargo tree --prefix none` output."""
    crates = set()
    for line in listing.splitlines():
        token = line.strip().split(" ", 1)[0]
        if token and token != "[dev-dependencies]" and token != "[build-dependencies]":
            crates.add(token)
    return crates


def verify_engine_free(root=ROOT, *, runner=None):
    """Verify engine-free native client invariants and companion backend DB bounds."""
    client_command = ["cargo", "tree", "--locked", "--prefix", "none"]
    for package in CLIENT_ENGINE_FREE_PACKAGES:
        client_command += ["-p", package]
    client_crates = _parse_cargo_tree_crates(_run(client_command, runner=runner, root=root, timeout=120))
    forbidden_client = sorted(crate for crate in FORBIDDEN_CLIENT_ENGINE_CRATES if crate in client_crates)
    if forbidden_client:
        raise ValueError(
            f"Engine-free native crates unexpectedly depend on backend engine crates: {', '.join(forbidden_client)}"
        )

    backend_variants = (
        ("default", ["cargo", "tree", "--locked", "--prefix", "none", "-p", COMPANION_PACKAGE]),
        ("embedded_only", ["cargo", "tree", "--locked", "--prefix", "none", "-p", COMPANION_PACKAGE,
                           "--no-default-features", "--features", "embedded"]),
    )
    for variant, command in backend_variants:
        crates = _parse_cargo_tree_crates(_run(command, runner=runner, root=root, timeout=120))
        if "lancedb" in crates:
            raise ValueError(f"Companion backend ({variant}) must never depend on lancedb")

    rusqlite_tree = _run(
        ["cargo", "tree", "--locked", "--prefix", "depth", "--invert", "rusqlite",
         "-p", COMPANION_PACKAGE, "--no-default-features", "--features", "embedded,nlp,classifier"],
        runner=runner, root=root, timeout=120,
    )
    direct_rusqlite_parents = set()
    for line in rusqlite_tree.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("1"):
            crate = stripped[1:].strip().split(" ", 1)[0]
            if crate:
                direct_rusqlite_parents.add(crate)
    disallowed_parents = sorted(p for p in direct_rusqlite_parents if p not in ALLOWED_RUSQLITE_PARENTS)
    if disallowed_parents:
        raise ValueError(
            f"Companion backend must only pull rusqlite via fleetworks-jobs, found: {', '.join(disallowed_parents)}"
        )
    return {
        "client_packages": list(CLIENT_ENGINE_FREE_PACKAGES),
        "client_engine_free": True,
        "companion_package": COMPANION_PACKAGE,
        "lancedb_absent": True,
        "rusqlite_parents": sorted(direct_rusqlite_parents),
    }


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


def elf_architecture(binary):
    if binary.is_symlink() or not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("Native binary must be a regular executable file")
    with binary.open("rb") as source:
        header = source.read(20)
    if len(header) != 20 or header[:4] != b"\x7fELF" or header[4] != 2 or header[5] != 1:
        raise ValueError("Native binary is not a 64-bit little-endian ELF executable")
    e_type, e_machine = struct.unpack("<HH", header[16:20])
    if e_type not in (2, 3):
        raise ValueError("ELF input is not an executable")
    return e_machine


def binary_architecture_matches(binary, target):
    if target in DARWIN_NATIVE_TARGETS:
        return macho_architecture(binary) == NATIVE_CPU_TYPES[target]
    if target in LINUX_NATIVE_TARGETS:
        return elf_architecture(binary) == LINUX_ELF_MACHINES[target]
    raise ValueError("Unsupported native target")


def inspect_native_binary(binary, target, *, runner=None):
    if not binary_architecture_matches(binary, target):
        raise ValueError(f"Native binary has the wrong architecture for {target}")
    if target in LINUX_NATIVE_TARGETS:
        listing = _run(["readelf", "-d", str(binary)], runner=runner)
        libraries = []
        for line in listing.splitlines():
            match = re.search(r"\(NEEDED\)\s+Shared library:\s+\[([^\]]+)\]", line)
            if not match:
                continue
            library = match.group(1).strip()
            if not library or "/" in library or "\\" in library or ".." in library:
                raise ValueError(f"Native Linux binary references an unsafe library path: {library}")
            lower = library.lower()
            if any(lower.startswith(prefix) for prefix in DISALLOWED_LINUX_LIBRARIES):
                raise ValueError(f"Native Linux binary links disallowed library: {library}")
            libraries.append(library)
        if not libraries:
            raise ValueError("readelf did not report Linux shared library dependencies")
        return libraries
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


def linux_runtime_preflight(environment=None, *, vulkan_icd_dirs=None, runner=None):
    """Validate Linux desktop display, Vulkan, and session bus preconditions without side effects."""
    env = os.environ if environment is None else environment
    display = bool(env.get("WAYLAND_DISPLAY") or env.get("DISPLAY"))
    if not display:
        raise ValueError("Linux GUI launch requires an active Wayland or X11 display (WAYLAND_DISPLAY or DISPLAY)")
    icd_candidates = list(vulkan_icd_dirs) if vulkan_icd_dirs is not None else [
        Path(p) for p in (env.get("VK_ICD_FILENAMES", ""), env.get("VK_DRIVER_FILES", "")) if p
    ] + [
        Path("/usr/share/vulkan/icd.d"),
        Path("/etc/vulkan/icd.d"),
        Path("/usr/local/share/vulkan/icd.d"),
    ]
    has_icd = any(
        (candidate.is_file() or (candidate.is_dir() and any(candidate.glob("*.json"))))
        for candidate in icd_candidates
    )
    if not has_icd:
        # Allow vulkaninfo confirmation if custom driver path is configured outside standard dirs.
        ok = False
        if shutil.which("vulkaninfo") or runner is not None:
            try:
                out = _run(["vulkaninfo", "--summary"], runner=runner)
                ok = bool(out.strip())
            except (OSError, ValueError):
                ok = False
        if not ok:
            raise ValueError("Linux GUI launch requires a working Vulkan ICD driver configuration")
    return {
        "display": "wayland" if env.get("WAYLAND_DISPLAY") else "x11",
        "dbus_session": bool(env.get("DBUS_SESSION_BUS_ADDRESS")),
        "vulkan": True,
    }


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


def _tar_gz_bundle(bundle_dir, archive):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as output:
        for path in [bundle_dir, *sorted(bundle_dir.rglob("*"))]:
            relative = path.relative_to(bundle_dir.parent).as_posix()
            info = tarfile.TarInfo(relative)
            info.mtime = 0
            info.uid = 0
            info.gid = 0
            info.uname = "root"
            info.gname = "root"
            if path.is_dir():
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                output.addfile(info)
            else:
                info.type = tarfile.REGTYPE
                info.mode = 0o755 if os.access(path, os.X_OK) else 0o644
                info.size = path.stat().st_size
                with path.open("rb") as source:
                    output.addfile(info, source)
    with archive.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            compressed.write(buffer.getvalue())


def _linux_desktop_entry():
    return (
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Fleetworks\n"
        "Comment=Fleetworks Workstation Client\n"
        f"Exec={NATIVE_BINARY}\n"
        "Icon=fleetworks\n"
        "Terminal=false\n"
        "Categories=Network;Utility;\n"
        "StartupWMClass=Fleetworks\n"
    )


def _atomic_install_stage(destination, stage, outputs, bundle_name, manifest_name):
    backup = stage / "previous"
    backup.mkdir()
    replaced, installed = [], []
    try:
        for name in outputs:
            existing = destination / name
            if existing.exists() or existing.is_symlink():
                if name not in (bundle_name, manifest_name):
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


def _strip_debug_symbols(path, target, *, runner=None):
    """Strip DWARF/Mach-O debug sections from a staged native executable."""
    flag = "--strip-debug" if target in LINUX_NATIVE_TARGETS else "-S"
    _run(["strip", flag, str(path)], runner=runner)


def _stage_executable(source, destination, target, *, runner=None):
    """Copy a native binary into staging, mark executable, and strip debug sections."""
    shutil.copyfile(source, destination)
    destination.chmod(0o755)
    _strip_debug_symbols(destination, target, runner=runner)
    return destination.stat().st_size


def package_native(destination, binary, resources, *, version, target, revision, profile,
                   rune_revision, toolchain, dirty, minimum_os=None, bundle_version=None,
                   run_id=None, attempt="1", workflow="local", archive=False,
                   cargo_lock_sha256="", companion_binary=None, runner=None):
    """Stage a checked complete app and replace only this helper's outputs.

    Packaging/architecture/library checks are not a GUI, signing, notarization
    or minimum-OS pass. Existing unrelated apps and immutable archive identities
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
    binary = Path(binary)
    if binary.is_symlink():
        raise ValueError("Native binary cannot be a symlink")
    binary = binary.resolve()
    libraries = inspect_native_binary(binary, target, runner=runner)
    companion_meta = None
    if companion_binary is not None:
        companion_binary = Path(companion_binary)
        if companion_binary.is_symlink():
            raise ValueError("Companion backend binary cannot be a symlink")
        companion_binary = companion_binary.resolve()
        companion_libraries = inspect_native_binary(companion_binary, target, runner=runner)
        relative_companion = (
            f"bin/{COMPANION_BINARY}"
            if target in LINUX_NATIVE_TARGETS
            else f"Contents/MacOS/{COMPANION_BINARY}"
        )
        architecture_map = LINUX_NATIVE_TARGETS if target in LINUX_NATIVE_TARGETS else DARWIN_NATIVE_TARGETS
        companion_meta = {
            "name": COMPANION_BINARY,
            "relative_path": relative_companion,
            "architecture": architecture_map[target],
            "sha256": sha256(companion_binary),
            "libraries": companion_libraries,
        }
    destination = native_output(destination, target, profile)
    run_id = str(run_id or f"local-{time.time_ns()}")
    archive_file = native_archive_name(version, target, revision, run_id, attempt)
    if archive and (destination / archive_file).exists():
        raise ValueError("Native candidate archive identity already exists; it is immutable")

    bundle_name = native_bundle_name(target)
    manifest_name = native_manifest_name(target)
    bundle_identifier = "dev.fleetworks.desktop"

    if target in LINUX_NATIVE_TARGETS:
        icon_path = resources / "fleetworks.svg"
        if icon_path.is_symlink() or not icon_path.is_file():
            raise ValueError("Native Linux bundle requires resources/fleetworks.svg icon asset")
        manifest = {
            "format": NATIVE_FORMAT, "platform": "linux", "version": version, "revision": revision,
            "dirty": bool(dirty), "target": target, "architecture": LINUX_NATIVE_TARGETS[target],
            "profile": profile, "rune_revision": rune_revision, "toolchain": toolchain,
            "bundle_identifier": bundle_identifier,
            "workflow": workflow, "run_id": run_id, "run_attempt": str(attempt),
            "backend_compatibility": {"api_version": 1, "required_connect_contract": 1},
            "dependency_provenance": {"cargo_lock_sha256": cargo_lock_sha256},
            "signing": {"status": "unsigned-review", "notarization": "not_requested", "verified": False},
            "verification": {"architecture": "pass", "libraries": "pass", "bundle": "pass",
                             "gui_launch": "not_run", "desktop_environment": "not_tested", "product_tests": "not_recorded"},
            "libraries": libraries,
        }
    else:
        plist_path = resources / "Info.plist"
        if plist_path.is_symlink() or not plist_path.is_file():
            raise ValueError("Native resources require a regular Info.plist")
        with plist_path.open("rb") as source:
            plist = plistlib.load(source)
        if not isinstance(plist, dict) or not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", str(plist.get("CFBundleIdentifier", ""))):
            raise ValueError("Native Info.plist requires a valid bundle identifier")
        bundle_identifier = plist["CFBundleIdentifier"]
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
        manifest = {
            "format": NATIVE_FORMAT, "platform": "macos", "version": version, "revision": revision,
            "dirty": bool(dirty), "target": target, "architecture": DARWIN_NATIVE_TARGETS[target],
            "profile": profile, "rune_revision": rune_revision, "toolchain": toolchain,
            "minimum_macos": minimum_os, "bundle_identifier": bundle_identifier,
            "bundle_short_version": short_version, "bundle_version": build_version,
            "workflow": workflow, "run_id": run_id, "run_attempt": str(attempt),
            "backend_compatibility": {"api_version": 1, "required_connect_contract": 1},
            "dependency_provenance": {"cargo_lock_sha256": cargo_lock_sha256},
            "signing": {"status": "unsigned-review", "notarization": "not_requested", "verified": False},
            "verification": {"architecture": "pass", "libraries": "pass", "bundle": "pass",
                             "gui_launch": "not_run", "minimum_os": "not_tested", "product_tests": "not_recorded"},
            "libraries": libraries,
        }
    if companion_meta is not None:
        manifest["companion"] = companion_meta

    old_app = destination / bundle_name
    old_manifest = destination / manifest_name
    if old_app.exists() or old_app.is_symlink() or old_manifest.exists() or old_manifest.is_symlink():
        if old_app.is_symlink() or old_manifest.is_symlink() or not old_app.is_dir() or not old_manifest.is_file():
            raise ValueError("Existing native output is not an owned app/manifest pair")
        previous = json.loads(old_manifest.read_text(encoding="utf-8"))
        if not isinstance(previous, dict) or previous.get("format") != NATIVE_FORMAT or previous.get("bundle_identifier") != manifest["bundle_identifier"]:
            raise ValueError("Refusing to replace an unrelated native app")
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".fleetworks-stage-", dir=destination) as temporary:
        stage = Path(temporary)
        app = stage / bundle_name
        companion_bytes = None
        if target in LINUX_NATIVE_TARGETS:
            bin_dir = app / "bin"
            bin_dir.mkdir(parents=True)
            executable = bin_dir / NATIVE_BINARY
            executable_bytes = _stage_executable(binary, executable, target, runner=runner)
            staged_companion = None
            if companion_binary is not None:
                staged_companion = bin_dir / COMPANION_BINARY
                companion_bytes = _stage_executable(companion_binary, staged_companion, target, runner=runner)
                companion_meta["sha256"] = sha256(staged_companion)
            desktop_dir = app / "share/applications"
            desktop_dir.mkdir(parents=True)
            desktop_file = desktop_dir / "fleetworks.desktop"
            desktop_file.write_text(_linux_desktop_entry(), encoding="utf-8")
            desktop_file.chmod(0o644)
            icon_dir = app / "share/icons/hicolor/scalable/apps"
            icon_dir.mkdir(parents=True)
            staged_icon = icon_dir / "fleetworks.svg"
            shutil.copyfile(resources / "fleetworks.svg", staged_icon)
            staged_icon.chmod(0o644)
            share_res = app / "share/fleetworks/resources"
            share_res.mkdir(parents=True)
            for path in resource_files:
                target_path = share_res / path.relative_to(resources)
                target_path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target_path)
                target_path.chmod(0o644)
            manifest["files"] = {path.relative_to(app).as_posix(): sha256(path)
                                 for path in sorted(app.rglob("*")) if path.is_file()}
            _write_json(app / "share/fleetworks/fleetworks-manifest.json", manifest)
        else:
            contents = app / "Contents"
            executable = contents / "MacOS" / NATIVE_BINARY
            executable.parent.mkdir(parents=True)
            bundled_resources = contents / "Resources"
            bundled_resources.mkdir()
            executable_bytes = _stage_executable(binary, executable, target, runner=runner)
            staged_companion = None
            if companion_binary is not None:
                staged_companion = contents / "MacOS" / COMPANION_BINARY
                companion_bytes = _stage_executable(companion_binary, staged_companion, target, runner=runner)
                companion_meta["sha256"] = sha256(staged_companion)
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
        unpacked_bytes = sum(path.stat().st_size for path in app.rglob("*") if path.is_file())
        if unpacked_bytes > MAX_NATIVE_UNPACKED_BYTES:
            raise ValueError(f"Native bundle exceeds 1 GiB unpacked limit ({unpacked_bytes} bytes)")
        _write_json(stage / manifest_name, manifest)
        outputs = [bundle_name, manifest_name]
        digest = None
        archive_bytes = None
        if archive:
            if target in LINUX_NATIVE_TARGETS:
                _tar_gz_bundle(app, stage / archive_file)
            else:
                _zip_app(app, stage / archive_file)
            archive_bytes = (stage / archive_file).stat().st_size
            if archive_bytes > MAX_NATIVE_ARCHIVE_BYTES:
                raise ValueError(f"Native archive exceeds 512 MiB compressed limit ({archive_bytes} bytes)")
            digest = sha256(stage / archive_file)
            (stage / (archive_file + ".sha256")).write_text(checksum_line(stage / archive_file, digest), encoding="utf-8")
            outputs.extend([archive_file, archive_file + ".sha256"])
        _atomic_install_stage(destination, stage, outputs, bundle_name, manifest_name)
    summary = {
        "app": str(destination / bundle_name),
        "executable": str(destination / bundle_name / ("bin" if target in LINUX_NATIVE_TARGETS else "Contents/MacOS") / NATIVE_BINARY),
        "manifest": str(destination / manifest_name),
        "archive": str(destination / archive_file) if archive else None,
        "sha256": digest,
        "unpacked_bytes": unpacked_bytes,
        "archive_bytes": archive_bytes,
        "executable_bytes": executable_bytes,
        "identity": manifest,
    }
    if companion_binary is not None:
        summary["companion"] = str(
            destination / bundle_name / ("bin" if target in LINUX_NATIVE_TARGETS else "Contents/MacOS") / COMPANION_BINARY
        )
        summary["companion_bytes"] = companion_bytes
    return summary


def _validate_extracted_companion(app, manifest, target, relative_exe, *, runner=None):
    """Validate bundled companion binary presence/absence and metadata against manifest."""
    companion_path = app / relative_exe
    companion_info = manifest.get("companion")
    if companion_info is None:
        if companion_path.exists() or companion_path.is_symlink():
            raise ValueError("Native archive contains undeclared companion executable")
        return
    if not isinstance(companion_info, dict) or companion_info.get("relative_path") != relative_exe:
        raise ValueError("Companion backend manifest metadata is invalid")
    if companion_path.is_symlink() or not companion_path.is_file() or not os.access(companion_path, os.X_OK):
        raise ValueError("Native archive is missing its declared companion executable")
    if sha256(companion_path) != companion_info.get("sha256"):
        raise ValueError("Companion backend executable digest does not match manifest")
    libraries = inspect_native_binary(companion_path, target, runner=runner)
    if libraries != companion_info.get("libraries"):
        raise ValueError("Companion backend library dependencies do not match manifest")


def _redact_diagnostic_text(text):
    """Mask bearer tokens and credential key-value assignments from captured output."""
    redacted = re.sub(r"(?i)(authorization\s*:\s*(?:bearer|basic)\s+)\S+", r"\1[REDACTED]", text)
    redacted = re.sub(r"(?i)(\bbearer\s+)[A-Za-z0-9._~+/=-]+", r"\1[REDACTED]", redacted)
    return re.sub(r"(?i)((?:api[_-]?token|token|password|secret)\s*[=:]\s*)\S+", r"\1[REDACTED]", redacted)


def _open_diagnostic_logs(log_dir):
    """Create retained stdout/stderr log files; return (dir, out, err) or None when unavailable.

    Log setup is best effort so it never becomes another source of launch failure.
    """
    try:
        if log_dir is None:
            log_dir = tempfile.mkdtemp(prefix="fleetworks-diagnostic-")
        log_dir = Path(log_dir)
        log_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        handles = []
        for name in ("stdout.log", "stderr.log"):
            fd = os.open(log_dir / name, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            handles.append(os.fdopen(fd, "wb"))
        return log_dir, handles[0], handles[1]
    except OSError:
        return None


def _diagnostic_log_excerpt(log_dir, limit):
    """Read a bounded, redacted tail of the retained logs."""
    parts = []
    for name in ("stderr.log", "stdout.log"):
        try:
            with open(log_dir / name, "rb") as handle:
                handle.seek(0, os.SEEK_END)
                handle.seek(max(0, handle.tell() - limit))
                parts.append(handle.read())
        except OSError:
            continue
    return _redact_diagnostic_text(b"\n".join(parts).decode("utf-8", errors="replace")).strip()[-limit:]


def _diagnostic_foreground_launch(executable, app, *, startup_probe_seconds=DIAGNOSTIC_STARTUP_PROBE_SECONDS,
                                  popen=None, log_dir=None):
    """Run the packaged binary with `--diagnostic` and probe startup.

    The child writes stdout/stderr straight to retained log files, so it can never block on a
    full pipe, and errors emitted after the probe window remain on disk. A process that
    survives the probe is reported as "still running", not "healthy", with the log location.
    """
    logs = _open_diagnostic_logs(log_dir)
    if logs is None:
        log_dir, out_handle, err_handle = None, subprocess.DEVNULL, subprocess.DEVNULL
        log_note = "diagnostic logs unavailable"
    else:
        log_dir, out_handle, err_handle = logs
        log_note = f"diagnostic logs: {log_dir}"
    try:
        try:
            proc = (popen or subprocess.Popen)(
                [str(executable), "--diagnostic"],
                cwd=Path(app),
                stdin=subprocess.DEVNULL,
                stdout=out_handle,
                stderr=err_handle,
            )
        except OSError as error:
            raise ValueError(
                f"Diagnostic foreground launch failed (process_started=false, window_readiness_observed=false; "
                f"{log_note}): {error}"
            ) from error
    finally:
        # The child holds its own descriptors; the parent copies are no longer needed.
        if logs is not None:
            out_handle.close()
            err_handle.close()

    deadline = time.monotonic() + max(0.0, float(startup_probe_seconds))
    while proc.poll() is None and time.monotonic() < deadline:
        time.sleep(min(0.05, max(0.0, deadline - time.monotonic())))

    rc = proc.poll()
    if rc is not None:
        excerpt = _diagnostic_log_excerpt(log_dir, 4000) if log_dir is not None else ""
        if rc != 0:
            raise ValueError(
                f"Diagnostic foreground launch exited early ({rc}; process_started=true, pid={proc.pid}, "
                f"window_readiness_observed=false; {log_note}): {excerpt or 'no output captured'}"
            )
        return (
            f"Diagnostic foreground launch exited cleanly (0; process_started=true, pid={proc.pid}, "
            f"window_readiness_observed=false: exited before window readiness observation; {log_note})"
            f"{f': {excerpt}' if excerpt else '.'}"
        )

    excerpt = _diagnostic_log_excerpt(log_dir, 500) if log_dir is not None else ""
    return (
        f"Diagnostic foreground launch still running (process_started=true, pid={proc.pid}, "
        f"window_readiness_observed=false: startup probe confirms process survival, not GUI readiness; "
        f"later output continues in {log_note})."
        f"{f' Early output: {excerpt}' if excerpt else ''}"
    )


def open_native(app, info, target, *, diagnostic=False, startup_probe_seconds=DIAGNOSTIC_STARTUP_PROBE_SECONDS,
                runner=None, preflight=None, popen=None):
    if info.system not in ("Darwin", "Linux") or target != info.native_target:
        raise ValueError("--open requires an artifact matching this physical host architecture")
    _progress("Checking whether Fleetworks is already running")
    commands = _run(["ps", "-axo", "comm="], runner=runner)
    if any(Path(line.strip()).name == NATIVE_BINARY for line in commands.splitlines()):
        return "Fleetworks is already running; close it normally, then open the new printed app path. No process was stopped."
    if info.system == "Linux":
        if os.environ.get("FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET"):
            raise ValueError("Local review approval socket is macOS-only; use separate CLI approval on Linux")
        if preflight is None:
            linux_runtime_preflight(runner=runner)
        else:
            preflight()
        executable = Path(app) / "bin" / NATIVE_BINARY
        if executable.is_symlink() or not executable.is_file() or not os.access(executable, os.X_OK):
            raise ValueError("Linux bundle executable is missing or not executable")
        if diagnostic:
            _progress(f"Launching {executable} in foreground diagnostic mode")
            return _diagnostic_foreground_launch(
                executable, app, startup_probe_seconds=startup_probe_seconds, popen=popen
            )
        _progress(f"Launching {executable}")
        if runner is not None:
            _run([str(executable)], runner=runner)
        else:
            (popen or subprocess.Popen)(
                [str(executable)],
                cwd=Path(app),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        return "Launched Linux native executable in an isolated session; actual GUI readiness and keyring behavior remain unverified."
    review_socket = os.environ.get("FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET")
    if review_socket is not None:
        if not Path(review_socket).is_absolute() or len(os.fsencode(review_socket)) > 103:
            raise ValueError("Local review approval socket must be an absolute macOS UNIX socket path")
    if diagnostic:
        executable = Path(app) / "Contents/MacOS" / NATIVE_BINARY
        if executable.is_symlink() or not executable.is_file() or not os.access(executable, os.X_OK):
            raise ValueError("macOS bundle executable is missing or not executable")
        _progress(f"Launching {executable} in foreground diagnostic mode")
        return _diagnostic_foreground_launch(
            executable, app, startup_probe_seconds=startup_probe_seconds, popen=popen
        )
    _progress(f"Opening {app} through Launch Services")
    command = ["open"]
    if review_socket is not None:
        command.extend(["--env", f"FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET={review_socket}"])
    command.append(str(app))
    _run(command, runner=runner)
    return "Requested Launch Services opening; actual GUI readiness and authenticated operations remain unverified."


def _extract_linux_archive(archive, destination, *, expected_digest, version, target, revision,
                           rune_revision, profile=None, require_clean=False, runner=None):
    embedded_name = f"{LINUX_APP}/share/fleetworks/fleetworks-manifest.json"
    exe_name = f"{LINUX_APP}/bin/{NATIVE_BINARY}"
    companion_name = f"{LINUX_APP}/bin/{COMPANION_BINARY}"
    with tarfile.open(archive, "r:gz") as tar:
        members = tar.getmembers()
        names = [member.name for member in members]
        if not members or len(members) > 10000 or len(set(names)) != len(names):
            raise ValueError("Native archive has missing, duplicate or excessive entries")
        if sum(member.size for member in members) > MAX_NATIVE_UNPACKED_BYTES:
            raise ValueError("Native archive exceeds its uncompressed size limit")
        for member in members:
            path = PurePosixPath(member.name)
            if len(member.name) > 1024 or any(ord(char) < 32 for char in member.name) \
                    or path.is_absolute() or ".." in path.parts or "\\" in member.name \
                    or not path.parts or path.parts[0] != LINUX_APP \
                    or member.name.rstrip("/") != path.as_posix():
                raise ValueError("Native archive contains an unsafe or unexpected path")
            if member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                raise ValueError("Native archive contains forbidden symlinks or special files")
            expected_mode = 0o755 if (member.isdir() or member.name in (exe_name, companion_name)) else 0o644
            if (member.mode & 0o777) != expected_mode:
                raise ValueError("Native archive entry permissions are invalid")
        if embedded_name not in names or tar.getmember(embedded_name).size > 1024 * 1024:
            raise ValueError("Native archive has no bounded embedded manifest")
        with tar.extractfile(tar.getmember(embedded_name)) as manifest_stream:
            manifest = json.loads(manifest_stream.read())
        if not isinstance(manifest, dict) or manifest.get("format") != NATIVE_FORMAT \
                or manifest.get("platform") != "linux" \
                or manifest.get("version") != version or manifest.get("revision") != revision \
                or manifest.get("target") != target or manifest.get("rune_revision") != rune_revision \
                or manifest.get("architecture") != LINUX_NATIVE_TARGETS[target] \
                or (profile is not None and manifest.get("profile") != profile):
            raise ValueError("Native manifest does not match expected version, revision, target and Rune pin")
        if require_clean and manifest.get("dirty") is not False:
            raise ValueError("Verified CI/release artifact must have a clean source identity")
        if native_archive_name(version, target, revision, manifest.get("run_id"), manifest.get("run_attempt")) != archive.name:
            raise ValueError("Native archive filename does not match its immutable identity")
        hashes = manifest.get("files")
        if not isinstance(hashes, dict) or not hashes:
            raise ValueError("Native manifest has no file hashes")
        file_names = {f"{LINUX_APP}/{name}" for name in hashes} | {embedded_name}
        if file_names != {member.name for member in members if member.isfile()}:
            raise ValueError("Native archive files do not match the manifest")
        for name, expected in hashes.items():
            if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
                raise ValueError("Native manifest contains an invalid file digest")
            with tar.extractfile(tar.getmember(f"{LINUX_APP}/{name}")) as source:
                digest = hashlib.sha256()
                for block in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(block)
            if digest.hexdigest() != expected:
                raise ValueError(f"Native bundle file digest mismatch: {name}")
        destination.mkdir(parents=True, exist_ok=True)
        for member in members:
            target_path = destination.joinpath(*PurePosixPath(member.name).parts)
            if member.isdir():
                target_path.mkdir(parents=True, exist_ok=True)
                target_path.chmod(0o755)
            else:
                target_path.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as source, target_path.open("xb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                target_path.chmod(member.mode & 0o777)
    app = destination / LINUX_APP
    if not (app / "share/applications/fleetworks.desktop").is_file() \
            or not (app / "share/icons/hicolor/scalable/apps/fleetworks.svg").is_file():
        raise ValueError("Linux bundle is missing desktop entry or application icon")
    libraries = inspect_native_binary(app / "bin" / NATIVE_BINARY, target, runner=runner)
    if libraries != manifest.get("libraries"):
        raise ValueError("Native library dependencies do not match the manifest")
    _validate_extracted_companion(app, manifest, target, f"bin/{COMPANION_BINARY}", runner=runner)
    return app, manifest


def extract_native(archive, destination, *, expected_digest, version, target, revision,
                   rune_revision, profile=None, require_clean=False, runner=None):
    """Validate a downloaded immutable archive before bounded private extraction.

    The caller obtains the expected digest/identity from its trusted packaging
    job or review record, not from the archive being verified. Extraction does
    not install or launch the app and cannot prove signing or GUI behavior.
    """
    archive, destination = Path(archive), Path(destination)
    if not re.fullmatch(r"[0-9a-f]{64}", expected_digest):
        raise ValueError("A packaging-job SHA-256 is required for native verification")
    if target not in NATIVE_TARGETS or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("An exact native target and source revision are required")
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size > MAX_NATIVE_ARCHIVE_BYTES:
        raise ValueError("Native archive must be a bounded regular file")
    if sha256(archive) != expected_digest:
        raise ValueError("Native archive does not match the packaging job's SHA-256")
    if archive.with_name(archive.name + ".sha256").read_text(encoding="utf-8") != checksum_line(archive, expected_digest):
        raise ValueError("Native archive checksum sidecar does not match the packaging job")
    if destination.is_symlink() or (destination.exists() and (not destination.is_dir() or any(destination.iterdir()))):
        raise ValueError("Native extraction requires an empty private directory")
    if target in LINUX_NATIVE_TARGETS:
        return _extract_linux_archive(
            archive, destination, expected_digest=expected_digest, version=version,
            target=target, revision=revision, rune_revision=rune_revision,
            profile=profile, require_clean=require_clean, runner=runner,
        )
    embedded_name = f"{NATIVE_APP}/Contents/Resources/fleetworks-manifest.json"
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        names = [entry.filename for entry in entries]
        if not entries or len(entries) > 10000 or len(set(names)) != len(names):
            raise ValueError("Native archive has missing, duplicate or excessive entries")
        if sum(entry.file_size for entry in entries) > MAX_NATIVE_UNPACKED_BYTES:
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
                (stat.S_IFREG | (0o755 if entry.filename in (
                    f"{NATIVE_APP}/Contents/MacOS/{NATIVE_BINARY}",
                    f"{NATIVE_APP}/Contents/MacOS/{COMPANION_BINARY}",
                ) else 0o644))
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
    _validate_extracted_companion(app, manifest, target, f"Contents/MacOS/{COMPANION_BINARY}", runner=runner)
    return app, manifest


def native_main(args):
    if sys.version_info < (3, 9):
        raise ValueError("Native helper requires Python 3.9 or newer")
    if not args.dry_run:
        _progress("Inspecting host, toolchain and native target")
    info = detect()
    target = resolve_native_target(info, args.target)
    version = artifact_version(getattr(args, "version", None), root=ROOT)
    if args.open and target != info.native_target:
        raise ValueError("--open cannot launch an artifact for the other physical Mac architecture")
    toolchain = validate_toolchain(info)
    destination = native_output(args.output, target, args.profile)
    with_companion = getattr(args, "with_companion", False)
    command = cargo_native_command(target, args.profile, agent=args.agent, implicit_host=args.target == "host") if args.command == "build-native" else None
    companion_command = (
        cargo_companion_command(
            target, args.profile, agent=args.agent, implicit_host=args.target == "host",
            features=getattr(args, "companion_features", None),
            no_default_features=getattr(args, "companion_no_default_features", False),
        )
        if args.command == "build-native" and with_companion
        else None
    )
    bundle_path = str(destination / native_bundle_name(target))
    if args.dry_run:
        payload = {"target": target, "platform": info.as_dict(), "profile": args.profile, "version": version,
                   "build_command": command, "app": bundle_path,
                   "archive_requested": args.archive, "open_requested": args.open,
                   "prerequisites": "SDK/package inputs checked during build/package, not this dry run"}
        if companion_command is not None:
            payload["companion_build_command"] = companion_command
        print(json.dumps(payload, indent=2))
        return 0
    _progress("Checking source identity and pinned Rune dependencies")
    revision, dirty = repository_identity(revision=args.revision)
    if (args.require_clean or os.environ.get("CI") == "true") and dirty:
        raise ValueError("CI/release native candidates require a clean exact-revision checkout")
    rune_revision = locked_rune_revision()
    resources = args.resources or ROOT / "apps/fleetworks-desktop/resources"
    if command:
        if target in DARWIN_NATIVE_TARGETS:
            _progress("Checking Xcode command-line tools and the macOS SDK")
            _run(["xcode-select", "-p"])
            _run(["xcrun", "--sdk", "macosx", "--show-sdk-path"])
        else:
            _progress("Checking Linux x86_64 native build tools")
            _run(["readelf", "--version"])
        environment = dict(os.environ)
        environment.pop("ARGOS_EMBED", None)
        environment["RUSTUP_AUTO_INSTALL"] = "0"
        _progress(f"Building {NATIVE_BINARY} ({target}, {args.profile})")
        _progress(shlex.join(command))
        binary = cargo_executable(_run_cargo(command, environment=environment, timeout=3600))
        if companion_command:
            _progress(f"Building {COMPANION_BINARY} ({target}, {args.profile})")
            _progress(shlex.join(companion_command))
            companion_binary = cargo_executable(
                _run_cargo(companion_command, environment=environment, timeout=3600),
                name=COMPANION_BINARY, package=COMPANION_PACKAGE,
            )
        else:
            companion_binary = None
    else:
        binary = args.binary
        companion_binary = getattr(args, "companion_binary", None)
    _progress("Packaging and validating the native app bundle")
    package_kwargs = dict(
        version=version, target=target, revision=revision, profile=args.profile,
        rune_revision=rune_revision, toolchain=toolchain, dirty=dirty,
        minimum_os=args.minimum_macos, bundle_version=args.bundle_version,
        run_id=args.run_id, attempt=args.run_attempt, workflow=args.workflow,
        archive=args.archive, cargo_lock_sha256=sha256(ROOT / "Cargo.lock"),
    )
    if companion_binary is not None:
        package_kwargs["companion_binary"] = companion_binary
    result = package_native(destination, binary, resources, **package_kwargs)
    print(result["app"], flush=True)
    print(json.dumps({key: value for key, value in result.items() if key != "identity"}, indent=2), flush=True)
    diagnostic = getattr(args, "diagnostic", False)
    if diagnostic and not args.open:
        raise ValueError("--diagnostic requires --open")
    if args.open:
        open_kwargs = {"diagnostic": True} if diagnostic else {}
        print(open_native(Path(result["app"]), info, target, **open_kwargs), flush=True)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_native = commands.add_parser("build-native", help="Build and package the engine-free Mac or Linux x86_64 desktop app and companion local-IPC backend")
    build_native.add_argument("--with-companion", dest="with_companion", action="store_true", default=True)
    build_native.add_argument("--without-companion", dest="with_companion", action="store_false")
    build_native.add_argument("--companion-features", help="Explicit Cargo feature list for argos-api-host")
    build_native.add_argument("--companion-no-default-features", action="store_true")
    pack_native = commands.add_parser("package-native", help="Package a built native executable using the same checked app packager")
    for command in (build_native, pack_native):
        command.add_argument("--target", choices=("host", *NATIVE_TARGETS), default="host")
        command.add_argument("--profile", choices=("debug", "release"), default="debug")
        command.add_argument("--output", type=Path)
        command.add_argument("--resources", type=Path)
        command.add_argument("--revision")
        command.add_argument("--version", help="Explicit calculated artifact SemVer; source workspace versions are preserved")
        command.add_argument("--minimum-macos")
        command.add_argument("--bundle-version")
        command.add_argument("--run-id", default=os.environ.get("GITHUB_RUN_ID"))
        command.add_argument("--run-attempt", default=os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
        command.add_argument("--workflow", default=os.environ.get("GITHUB_WORKFLOW", "local"))
        command.add_argument("--archive", action="store_true")
        command.add_argument("--open", action="store_true")
        command.add_argument("--diagnostic", action="store_true", help="Launch foreground --diagnostic mode with early-crash capture (requires --open)")
        command.add_argument("--dry-run", action="store_true")
        command.add_argument("--require-clean", action="store_true")
        command.add_argument("--agent", action="store_true", help="Use scripts/agent_cargo.py and target/agents for a build")
    pack_native.add_argument("--binary", type=Path, required=True)
    pack_native.add_argument("--companion-binary", type=Path)
    commands.add_parser("verify-engine-free", help="Verify fleetworks-desktop/contracts/workstation stay engine-free and backend DB bounds hold")
    verify_native = commands.add_parser("verify-native", help="Verify an exact downloaded app archive; does not install or launch by default")
    verify_native.add_argument("--archive", type=Path, required=True)
    verify_native.add_argument("--sha256", required=True)
    verify_native.add_argument("--revision", required=True)
    verify_native.add_argument("--version", help="Require this exact calculated artifact SemVer instead of version.txt")
    verify_native.add_argument("--target", choices=NATIVE_TARGETS, required=True)
    verify_native.add_argument("--profile", choices=("debug", "release"))
    verify_native.add_argument("--rune-revision")
    verify_native.add_argument("--directory", type=Path, help="Retain verified extraction in this new/empty review directory")
    verify_native.add_argument("--require-clean", action="store_true")
    verify_native.add_argument("--open", action="store_true")
    verify_native.add_argument("--diagnostic", action="store_true", help="Launch foreground --diagnostic mode with early-crash capture (requires --open)")
    args = parser.parse_args()
    try:
        if args.command == "verify-engine-free":
            print(json.dumps(verify_engine_free(ROOT), indent=2))
            return 0
        if args.command in ("build-native", "package-native"):
            return native_main(args)
        if args.command == "verify-native":
            version = artifact_version(args.version, root=ROOT)
            if args.diagnostic and not args.open:
                raise ValueError("--diagnostic requires --open")
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
                open_kwargs = {"diagnostic": True} if args.diagnostic else {}
                print(open_native(app, detect(), args.target, **open_kwargs))
            print(json.dumps({"archive": str(args.archive.resolve()), "sha256": args.sha256,
                              "version": manifest["version"], "revision": manifest["revision"], "target": manifest["target"],
                              "bundle_checks": "pass", "gui_launch": "not_run",
                              "signing": manifest["signing"]}, indent=2))
            return 0
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError, zipfile.BadZipFile, plistlib.InvalidFileException) as error:
        print(f"fleetworks-candidate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
