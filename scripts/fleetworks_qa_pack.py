#!/usr/bin/env python3
"""Build or import a target-specific, separately verified offline Inbox QA pack."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tarfile
import tempfile


MODEL_REVISION = "564e9b582944a57a3e586bbb98fd6f0a4118db7f"
MODEL = {
    "model/config.json": "0b5cb15ec08645604ef7085acfaf9c4131158ac22207a76634574cf2771b1515",
    "model/vocab.txt": "eeaa9875b23b04b4c54ef759d03db9d1ba1554838f8fb26c5d96fa551df93d02",
    "model/rust_model.ot": "8a9f9b2f153ac9ff230aca4548fa3286be9d2f9ea4eb7e9169665b1a8e983f44",
}
TARGETS = {
    "x86_64-unknown-linux-gnu": ("elf", 62, ".so"),
    "aarch64-unknown-linux-gnu": ("elf", 183, ".so"),
    "x86_64-apple-darwin": ("mach", 0x1000007, ".dylib"),
    "aarch64-apple-darwin": ("mach", 0x100000C, ".dylib"),
}
WORKER = "bin/fleetworks-assistant-bert"
LISTENER = "bin/fleetworks-assistant-bert-listener"
EXECUTABLES = {WORKER, LISTENER}
MANIFEST = "manifest.json"
MAX_COMPRESSED = 512 * 1024 * 1024
MAX_UNPACKED = 1024 * 1024 * 1024
MAX_FILES = 32
MAX_MANIFEST = 64 * 1024


def check(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def is_digest(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def allowed_file(path: str, target: str) -> bool:
    if path in MODEL or path in EXECUTABLES:
        return True
    if path == "licenses/torch-LICENSE" or re.fullmatch(r"licenses/torch-NOTICE[a-zA-Z0-9_.-]*", path):
        return True
    suffix = TARGETS[target][2]
    if suffix == ".so":
        return re.fullmatch(r"lib/lib[a-zA-Z0-9_.+-]+\.so(?:\.[0-9]+)*", path) is not None
    return re.fullmatch(r"lib/lib[a-zA-Z0-9_.+-]+\.dylib", path) is not None


def native_architecture(path: Path, target: str) -> None:
    with path.open("rb") as source:
        header = source.read(20)
    kind, machine, _ = TARGETS[target]
    if kind == "elf":
        valid = header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == machine
    else:
        valid = header[:4] == b"\xcf\xfa\xed\xfe" and int.from_bytes(header[4:8], "little") == machine
    check(valid, f"{path.name} does not match {target}")


def unique_pairs(pairs: list[tuple[str, object]]) -> dict:
    result = dict(pairs)
    check(len(result) == len(pairs), "duplicate pack manifest field")
    return result


def verify_directory(root: Path, target: str, expected_manifest: str) -> dict:
    check(is_digest(expected_manifest), "a trusted manifest SHA-256 is required")
    check(root.is_dir() and not root.is_symlink(), "pack directory is missing or linked")
    manifest = root / MANIFEST
    check(manifest.is_file() and not manifest.is_symlink(), "pack manifest is missing or linked")
    check(0 < manifest.stat().st_size <= MAX_MANIFEST, "pack manifest is out of bounds")
    raw = manifest.read_bytes()
    check(hashlib.sha256(raw).hexdigest() == expected_manifest, "pack manifest SHA-256 mismatch")
    data = json.loads(raw, object_pairs_hook=unique_pairs)
    check(isinstance(data, dict) and set(data) == {"protocol", "target", "model_revision", "files"},
          "unexpected pack manifest fields")
    check(type(data["protocol"]) is int and data["protocol"] == 1, "unsupported pack protocol")
    check(data["target"] == target and data["model_revision"] == MODEL_REVISION,
          "pack target or QA model revision mismatch")
    files = data["files"]
    check(isinstance(files, dict) and set(MODEL) | {WORKER} <= files.keys() and len(files) <= MAX_FILES,
          "pack file inventory is incomplete or too large")
    for name, expected in MODEL.items():
        check(files[name] == expected, f"{name} does not match the pinned QA checkpoint")
    listed = {MANIFEST, *files}
    total = len(raw)
    for name, expected in files.items():
        check(allowed_file(name, target) and is_digest(expected), "invalid pack file record")
        path = root / name
        check(path.parent.is_dir() and not path.parent.is_symlink(), "pack component directory is linked")
        check(path.is_file() and not path.is_symlink(), f"{name} is missing or linked")
        mode = path.stat().st_mode
        check(not mode & (stat.S_IWGRP | stat.S_IWOTH | stat.S_ISUID | stat.S_ISGID),
              f"{name} permissions are unsafe")
        size = path.stat().st_size
        check(0 < size <= MAX_UNPACKED - total, "unpacked QA pack exceeds its size limit")
        total += size
        check(sha256(path) == expected, f"{name} SHA-256 mismatch")
        if name.startswith(("bin/", "lib/")):
            native_architecture(path, target)
        if name in EXECUTABLES:
            check(bool(mode & stat.S_IXUSR), f"{name} is not executable")
    actual = {entry.name for entry in root.iterdir()}
    directories = {Path(name).parts[0] for name in files}
    check(actual == {MANIFEST, *directories}, "pack contains unexpected entries")
    for name in directories:
        check({f"{name}/{entry.name}" for entry in (root / name).iterdir()}
              == {item for item in listed if item.startswith(name + "/")},
              "pack contains unexpected component files")
    check(f"lib/libtorch_cpu{TARGETS[target][2]}" in files
          and f"lib/libc10{TARGETS[target][2]}" in files, "Torch CPU libraries are missing")
    return data


def write_archive(root: Path, output: Path, files: dict) -> None:
    with output.open("xb") as raw:
        with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as tar:
                for name in sorted({MANIFEST, *files}):
                    path = root / name
                    info = tarfile.TarInfo(name)
                    info.size = path.stat().st_size
                    info.mode = 0o755 if name in EXECUTABLES else 0o644
                    with path.open("rb") as source:
                        tar.addfile(info, source)


def build(args: argparse.Namespace) -> dict:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    check(not args.output.exists() and not args.output.is_symlink(), "pack output already exists")
    with tempfile.TemporaryDirectory(prefix=".qa-pack-", dir=args.output.parent) as staging:
        root = Path(staging)
        install = {name: args.model / Path(name).name for name in MODEL}
        install[WORKER] = args.worker
        if args.listener:
            install[LISTENER] = args.listener
        for directory, paths in (("lib", args.lib), ("licenses", args.license)):
            for path in paths:
                name = directory + "/" + path.name
                check(name not in install and allowed_file(name, args.target), "invalid or duplicate pack file")
                install[name] = path
        check(len(install) <= MAX_FILES, "pack contains too many files")
        files = {}
        total = 0
        for name, source in install.items():
            check(source.is_file(), f"missing input {name}")
            total += source.stat().st_size
            check(total < MAX_UNPACKED - MAX_MANIFEST, "unpacked QA pack exceeds its size limit")
            dest = root / name
            dest.parent.mkdir(exist_ok=True)
            shutil.copyfile(source, dest)
            dest.chmod(0o755 if name in EXECUTABLES else 0o644)
            files[name] = sha256(dest)
        manifest = {"protocol": 1, "model_revision": MODEL_REVISION, "target": args.target, "files": files}
        (root / MANIFEST).write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        manifest_digest = sha256(root / MANIFEST)
        verify_directory(root, args.target, manifest_digest)
        temporary = root / "archive.tar.gz"
        write_archive(root, temporary, files)
        check(temporary.stat().st_size <= MAX_COMPRESSED, "compressed QA pack exceeds its size limit")
        archive_digest = sha256(temporary)
        os.link(temporary, args.output)
        return {"target": args.target, "model_revision": MODEL_REVISION,
                "manifest_sha256": manifest_digest, "archive_sha256": archive_digest}


def extract(archive: Path, root: Path, target: str) -> None:
    total = 0
    seen: set[str] = set()
    with tarfile.open(archive, mode="r:gz") as tar:
        for entry in tar:
            name = entry.name
            check(len(seen) <= MAX_FILES and name not in seen and entry.isfile()
                  and not entry.pax_headers and (name == MANIFEST or allowed_file(name, target)),
                  "pack contains an invalid, duplicate, or excessive archive entry")
            member_limit = MAX_MANIFEST if name == MANIFEST else MAX_UNPACKED
            check(0 < entry.size <= min(member_limit, MAX_UNPACKED - total),
                  "pack archive entry exceeds its size limit")
            seen.add(name)
            total += entry.size
            dest = root / name
            dest.parent.mkdir(mode=0o700, exist_ok=True)
            source = tar.extractfile(entry)
            check(source is not None, "regular pack entry could not be read")
            with source, dest.open("xb") as output:
                shutil.copyfileobj(source, output, 64 * 1024)
            check(dest.stat().st_size == entry.size, "truncated pack archive entry")
            dest.chmod(0o700 if name in EXECUTABLES else 0o600)


def install(args: argparse.Namespace) -> dict:
    check(is_digest(args.archive_sha256), "a trusted archive SHA-256 is required")
    check(args.archive.is_file() and not args.archive.is_symlink()
          and 0 < args.archive.stat().st_size <= MAX_COMPRESSED, "QA pack archive is missing or oversized")
    parent = args.output.parent.resolve(strict=True)
    parent_stat = parent.stat()
    check(parent.is_dir() and parent_stat.st_uid == os.getuid() and not parent_stat.st_mode & 0o022,
          "QA pack destination parent must be owned and protected from other writers")
    check(args.output.name not in {"", ".", ".."}, "QA pack requires a new installation directory")
    target = parent / args.output.name
    check(not target.exists() and not target.is_symlink(), "QA pack destination already exists")
    state = args.state_dir
    if state is not None:
        info = state.lstat()
        check(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
              and not info.st_mode & 0o077, "companion state directory must be private and owned")
        state = state.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix=".qa-import-", dir=parent) as tmp:
        # Extract the same bytes that were verified, even if the source archive is replaced.
        archive = Path(tmp) / "archive.tar.gz"
        with args.archive.open("rb") as source, archive.open("xb") as output:
            remaining = MAX_COMPRESSED
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                check(len(chunk) <= remaining, "compressed QA pack exceeds its size limit")
                remaining -= len(chunk)
                output.write(chunk)
        check(sha256(archive) == args.archive_sha256, "pack archive SHA-256 mismatch")
        root = Path(tmp) / "pack"
        root.mkdir(mode=0o700)
        extract(archive, root, args.target)
        data = verify_directory(root, args.target, args.manifest_sha256)
        check(not target.exists() and not target.is_symlink(), "QA pack destination already exists")
        root.rename(target)
        if state is not None:
            pointer = state / "qa-pack.json"
            entry = {"root": str(target), "manifest_sha256": args.manifest_sha256}
            # Replace the pointer only after the complete target is installed. The previous
            # version remains usable if copying, extraction or validation is interrupted.
            with tempfile.NamedTemporaryFile("w", dir=state, prefix=".qa-pack-", delete=False) as file:
                staged_pointer = Path(file.name)
                try:
                    json.dump(entry, file, sort_keys=True)
                    file.write("\n")
                    file.flush()
                    os.fsync(file.fileno())
                    os.replace(staged_pointer, pointer)
                finally:
                    staged_pointer.unlink(missing_ok=True)
        return {"target": data["target"], "model_revision": data["model_revision"],
                "manifest_sha256": args.manifest_sha256}


def install_gated_fixture(archive: Path, output: Path, gated_worker: Path, digests: dict) -> tuple[Path, dict]:
    """Re-pin a separate smoke-only installation with the exact gated worker."""
    info = gated_worker.lstat()
    check(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= MAX_UNPACKED,
          "gated fixture must be a bounded regular executable")
    gate_digest = sha256(gated_worker)
    install(argparse.Namespace(archive=archive, output=output, state_dir=None, target=digests["target"],
                               archive_sha256=digests["archive_sha256"], manifest_sha256=digests["manifest_sha256"]))
    directory = output.resolve(strict=True)
    worker = directory / WORKER
    shutil.copyfile(gated_worker, worker)
    worker.chmod(0o700)
    check(sha256(worker) == gate_digest, "gated worker copy differs from the verified fixture artifact")
    manifest = directory / MANIFEST
    contents = json.loads(manifest.read_text())
    contents["files"][WORKER] = gate_digest
    manifest.write_text(json.dumps(contents, sort_keys=True) + "\n")
    digest = sha256(manifest)
    verify_directory(directory, digests["target"], digest)
    return worker, {"root": str(directory), "manifest_sha256": digest}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("build")
    create.add_argument("--worker", type=Path, required=True)
    create.add_argument("--listener", type=Path)
    create.add_argument("--model", type=Path, required=True)
    create.add_argument("--lib", type=Path, action="append", required=True)
    create.add_argument("--license", type=Path, action="append", default=[])
    create.add_argument("--output", type=Path, required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--directory", type=Path, required=True)
    verify.add_argument("--manifest-sha256", required=True)
    importer = commands.add_parser("install")
    importer.add_argument("--archive", type=Path, required=True)
    importer.add_argument("--archive-sha256", required=True)
    importer.add_argument("--manifest-sha256", required=True)
    importer.add_argument("--output", type=Path, required=True)
    importer.add_argument("--state-dir", type=Path, help="Register this pack in a private Embedded state directory")
    for command in (create, verify, importer):
        command.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
    try:
        if args.command == "build":
            result = build(args)
        elif args.command == "install":
            result = install(args)
        else:
            result = verify_directory(args.directory, args.target, args.manifest_sha256)
        print(json.dumps(result, sort_keys=True))
    except (OSError, TypeError, ValueError, tarfile.TarError) as error:
        parser.exit(1, f"QA pack: {error}\n")


if __name__ == "__main__":
    main()
