#!/usr/bin/env python3
"""Package Fleetworks CLI candidates and verify the downloaded payloads."""

import argparse
import gzip
import hashlib
import io
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARIES = ("venture", *(f"fleetworks-{app}" for app in ("atlas", "brain", "jobs", "tools", "models", "profile")))
TARGETS = {
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64", r"ELF 64-bit .*x86-64"),
    "aarch64-unknown-linux-gnu": ("Linux", "aarch64", r"ELF 64-bit .*(aarch64|ARM aarch64)"),
    "x86_64-apple-darwin": ("Darwin", "x86_64", r"Mach-O 64-bit executable x86_64"),
    "aarch64-apple-darwin": ("Darwin", "arm64", r"Mach-O 64-bit executable arm64"),
}
FILE_MODES = {"LICENSE": 0o644, **{f"bin/{name}": 0o755 for name in BINARIES}}


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
    args = parser.parse_args()
    try:
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
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(f"fleetworks-candidate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
