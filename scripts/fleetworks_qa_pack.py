#!/usr/bin/env python3
"""Build or import a target-specific, separately verified offline Inbox QA pack."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
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
DEFAULT_CHAT_MODEL = "gpt2-small"
LEGACY_CHAT_MODEL = "flan-t5-small"
DEFAULT_PACK_MODELS = (DEFAULT_CHAT_MODEL, LEGACY_CHAT_MODEL)
MODEL_REGISTRY = Path(__file__).resolve().parents[1] / "crates" / "fleetworks-contracts" / "src" / "qa_models.json"
CHAT_MODELS = json.loads(MODEL_REGISTRY.read_text(encoding="utf-8"))
CHAT_MODELS_BY_REVISION = {pin["revision"]: model for model, pin in CHAT_MODELS.items()}
GENERATOR_FILES = {
    model: {f"model/generator/{name}": (file["bytes"], file["sha256"]) for name, file in pin["files"].items()}
    for model, pin in CHAT_MODELS.items()
}
GENERATOR_PATHS = frozenset(name for files in GENERATOR_FILES.values() for name in files)
GENERATOR_REVISION = CHAT_MODELS[DEFAULT_CHAT_MODEL]["revision"]
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
MAX_BASE_COMPRESSED = 512 * 1024 * 1024
MAX_BASE_UNPACKED = 1024 * 1024 * 1024
GENERATOR_LIMITS = {
    model: sum(size for size, _ in files.values()) for model, files in GENERATOR_FILES.items()
}
GENERATOR_LIMITS[LEGACY_CHAT_MODEL] = 310_000_000
MAX_GENERATOR_BYTES = max(GENERATOR_LIMITS[model] for model in DEFAULT_PACK_MODELS)
MAX_COMPRESSED = MAX_BASE_COMPRESSED + MAX_GENERATOR_BYTES
MAX_UNPACKED = MAX_BASE_UNPACKED + MAX_GENERATOR_BYTES
MAX_FILES = 32
MAX_MANIFEST = 64 * 1024


def check(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def private_state_dir(path: Path) -> Path:
    info = path.lstat()
    check(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
          and not info.st_mode & 0o077, "companion state directory must be private and owned")
    return path.resolve(strict=True)


@contextmanager
def pack_progress(state: Path | None, stage: str):
    """Expose an advisory installation stage to this profile's running companion."""
    if state is None:
        yield
        return
    check(stage in {"downloading_pack", "verifying_pack"}, "invalid QA pack installation stage")
    state = private_state_dir(state)
    marker = state / "qa-pack-progress.json"
    identity = None
    try:
        with tempfile.NamedTemporaryFile("w", dir=state, prefix=".qa-pack-progress-",
                                         delete=False) as file:
            staged = Path(file.name)
            try:
                json.dump({"stage": stage}, file)
                file.write("\n")
                file.flush()
                info = os.fstat(file.fileno())
                os.replace(staged, marker)
                identity = (info.st_dev, info.st_ino)
            finally:
                staged.unlink(missing_ok=True)
    except OSError:
        # A progress display should not change archive verification or activation.
        pass
    try:
        yield
    finally:
        if identity is not None:
            try:
                current = marker.lstat()
                if (current.st_dev, current.st_ino) == identity:
                    marker.unlink()
            except OSError:
                pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def is_digest(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def allowed_file(path: str, target: str) -> bool:
    if path in MODEL or path in GENERATOR_PATHS or path in EXECUTABLES:
        return True
    if path == "licenses/torch-LICENSE" or re.fullmatch(r"licenses/torch-NOTICE[a-zA-Z0-9_.-]*", path):
        return True
    suffix = TARGETS[target][2]
    if suffix == ".so":
        return re.fullmatch(r"lib/lib[a-zA-Z0-9_.+-]+\.so(?:\.[0-9]+)*", path) is not None
    return re.fullmatch(r"lib/lib[a-zA-Z0-9_.+-]+\.dylib", path) is not None


def verify_model_assets(directory: Path, chat_model: str) -> None:
    files = CHAT_MODELS[chat_model]["files"]
    check(directory.is_dir() and not directory.is_symlink(), "chat model directory is missing or linked")
    check({path.name for path in directory.iterdir()} == files.keys(), "chat model asset inventory differs")
    for name, asset in files.items():
        path = directory / name
        info = path.lstat()
        check(stat.S_ISREG(info.st_mode) and info.st_size == asset["bytes"]
              and sha256(path) == asset["sha256"], f"{name} does not match the pinned chat model")


def materialize_model_assets(source: Path, state_dir: Path, chat_model: str) -> Path:
    """Populate an offline Embedded cache without accepting a new worker executable."""
    verify_model_assets(source, chat_model)
    state = private_state_dir(state_dir)
    cache = state / "qa-model-assets"
    cache.mkdir(mode=0o700, exist_ok=True)
    cache = private_state_dir(cache)
    destination = cache / chat_model
    check(not destination.exists() and not destination.is_symlink(), "chat model assets already exist")
    with tempfile.TemporaryDirectory(prefix=".qa-model-assets-", dir=cache) as directory:
        model = Path(directory) / "model"
        model.mkdir(mode=0o700)
        for name, asset in CHAT_MODELS[chat_model]["files"].items():
            target = model / name
            remaining = asset["bytes"]
            with (source / name).open("rb") as incoming, target.open("xb") as outgoing:
                for block in iter(lambda: incoming.read(1024 * 1024), b""):
                    check(len(block) <= remaining, f"{name} exceeded its pinned size")
                    outgoing.write(block)
                    remaining -= len(block)
            target.chmod(0o600)
        verify_model_assets(model, chat_model)
        model.rename(destination)
    return destination


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


def count_base_bytes(name: str, size: int, remaining: int, generator: dict) -> int:
    if name in GENERATOR_PATHS:
        check(name in generator and size == generator[name][0], f"{name} does not match the pinned generator size")
        return 0
    limit = min(remaining, MAX_MANIFEST) if name == MANIFEST else remaining
    check(0 < size <= limit, "unpacked QA pack exceeds its size limit")
    return size


def model_for_pack(manifest: dict) -> str | None:
    if manifest["model_revision"] == MODEL_REVISION:
        return LEGACY_CHAT_MODEL if GENERATOR_PATHS & manifest["files"].keys() else None
    return CHAT_MODELS_BY_REVISION[manifest["model_revision"]]


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
    revision = data["model_revision"]
    check(data["target"] == target and isinstance(revision, str)
          and (revision == MODEL_REVISION or revision in CHAT_MODELS_BY_REVISION),
          "pack target or QA model revision mismatch")
    files = data["files"]
    check(isinstance(files, dict) and WORKER in files and len(files) <= MAX_FILES,
          "pack file inventory is incomplete or too large")
    generator_files = GENERATOR_PATHS & files.keys()
    model = model_for_pack(data)
    generator = GENERATOR_FILES[model] if model is not None else {}
    extractive_files = MODEL.keys() & files.keys()
    if revision == MODEL_REVISION:
        check(extractive_files == MODEL.keys(), "extractive model inventory is incomplete")
        for name, expected in MODEL.items():
            check(files[name] == expected, f"{name} does not match the pinned QA checkpoint")
        check(generator_files == generator.keys(), "generator inventory is incomplete")
    else:
        check(not extractive_files and generator_files == generator.keys(),
              "primary generator inventory is incomplete or mixed")
    for name, (_, expected) in generator.items():
        check(files[name] == expected, f"{name} does not match the pinned generator digest")

    inventory = {Path("."): {MANIFEST}}
    for name, expected in files.items():
        check(allowed_file(name, target) and is_digest(expected), "invalid pack file record")
        component = Path(name)
        while component.name:
            inventory.setdefault(component.parent, set()).add(component.name)
            component = component.parent
    for directory in sorted(inventory, key=lambda path: len(path.parts)):
        directory_path = root / directory
        check(directory_path.is_dir() and not directory_path.is_symlink(), "pack component directory is linked")
        check({entry.name for entry in directory_path.iterdir()} == inventory[directory],
              "pack contains unexpected component files")

    total = len(raw)
    for name, expected in files.items():
        path = root / name
        check(path.is_file() and not path.is_symlink(), f"{name} is missing or linked")
        mode = path.stat().st_mode
        check(not mode & (stat.S_IWGRP | stat.S_IWOTH | stat.S_ISUID | stat.S_ISGID),
              f"{name} permissions are unsafe")
        total += count_base_bytes(name, path.stat().st_size, MAX_BASE_UNPACKED - total, generator)
        check(sha256(path) == expected, f"{name} SHA-256 mismatch")
        if name.startswith(("bin/", "lib/")):
            native_architecture(path, target)
        if name in EXECUTABLES:
            check(bool(mode & stat.S_IXUSR), f"{name} is not executable")
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
        requested_model = getattr(args, "chat_model", None)
        install = {name: args.model / Path(name).name for name in MODEL}
        install = {name: path for name, path in install.items() if path.exists() or path.is_symlink()}
        check(not install or install.keys() == MODEL.keys(), "extractive model source inventory is incomplete")
        model = requested_model or (LEGACY_CHAT_MODEL if install else DEFAULT_CHAT_MODEL)
        expected_generator = GENERATOR_FILES[model]
        check(not install or model == LEGACY_CHAT_MODEL,
              "legacy extractor can only share the FLAN-T5 Small generator")
        revision = MODEL_REVISION if install else CHAT_MODELS[model]["revision"]
        generator = args.model / "generator"
        if generator.exists() or generator.is_symlink():
            check(generator.is_dir() and not generator.is_symlink(), "generator source directory is linked")
            check({entry.name for entry in generator.iterdir()} == {Path(name).name for name in expected_generator},
                  "generator source inventory is incomplete or unexpected")
            for name in expected_generator:
                source = generator / Path(name).name
                check(not source.is_symlink(), "generator source file is linked")
                install[name] = source
        check(requested_model is None or expected_generator.keys() <= install.keys(),
              "the requested chat model source was not supplied")
        check(install, "no pinned model sources were supplied")
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
        base_bytes = 0
        for name, source in install.items():
            check(source.is_file(), f"missing input {name}")
            base_bytes += count_base_bytes(name, source.stat().st_size,
                                           MAX_BASE_UNPACKED - MAX_MANIFEST - base_bytes, expected_generator)
            dest = root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, dest)
            dest.chmod(0o755 if name in EXECUTABLES else 0o644)
            files[name] = sha256(dest)
        manifest = {"protocol": 1, "model_revision": revision, "target": args.target, "files": files}
        manifest_path = root / MANIFEST
        manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        manifest_digest = sha256(manifest_path)
        verify_directory(root, args.target, manifest_digest)
        temporary = root / "archive.tar.gz"
        write_archive(root, temporary, files)
        archive_bytes = temporary.stat().st_size
        has_generator = expected_generator.keys() <= files.keys()
        limit = MAX_BASE_COMPRESSED + (GENERATOR_LIMITS[model] if has_generator else 0)
        check(archive_bytes <= limit, "compressed QA pack exceeds its size limit")
        archive_digest = sha256(temporary)
        os.link(temporary, args.output)
        generator_bytes = sum(size for size, _ in expected_generator.values()) if has_generator else 0
        receipt = {"target": args.target, "model_revision": revision,
                   "manifest_sha256": manifest_digest, "archive_sha256": archive_digest,
                   "archive_bytes": archive_bytes,
                   "unpacked_bytes": sum((root / name).stat().st_size for name in (MANIFEST, *files)),
                   "generator_bytes": generator_bytes}
        if generator_bytes:
            receipt["chat_model"] = model
            receipt["generator_revision"] = CHAT_MODELS[model]["revision"]
        return receipt


def extract(archive: Path, root: Path, target: str, chat_model: str | None = None) -> None:
    total = 0
    seen: set[str] = set()
    candidates = DEFAULT_PACK_MODELS if chat_model is None else (chat_model,)
    with tarfile.open(archive, mode="r:gz") as tar:
        for entry in tar:
            name = entry.name
            check(len(seen) <= MAX_FILES and name not in seen and entry.isfile()
                  and not entry.pax_headers and (name == MANIFEST or allowed_file(name, target)),
                  "pack contains an invalid, duplicate, or excessive archive entry")
            if name in GENERATOR_PATHS:
                # Shared filenames may identify either compatible default until
                # later entries narrow the inventory to one pinned checkpoint.
                candidates = tuple(model for model in candidates
                                   if name in GENERATOR_FILES[model]
                                   and entry.size == GENERATOR_FILES[model][name][0])
                check(candidates, f"{name} does not match the pinned generator size")
            else:
                total += count_base_bytes(name, entry.size, MAX_BASE_UNPACKED - total, {})
            seen.add(name)
            dest = root / name
            parent = root
            for component in Path(name).parts[:-1]:
                parent = parent / component
                parent.mkdir(mode=0o700, exist_ok=True)
            source = tar.extractfile(entry)
            check(source is not None, "regular pack entry could not be read")
            with source, dest.open("xb") as output:
                shutil.copyfileobj(source, output, 64 * 1024)
            check(dest.stat().st_size == entry.size, "truncated pack archive entry")
            dest.chmod(0o700 if name in EXECUTABLES else 0o600)


def install(args: argparse.Namespace) -> dict:
    check(is_digest(args.archive_sha256), "a trusted archive SHA-256 is required")
    requested_model = getattr(args, "chat_model", None)
    limit = (MAX_COMPRESSED if requested_model is None
             else MAX_BASE_COMPRESSED + GENERATOR_LIMITS[requested_model])
    check(args.archive.is_file() and not args.archive.is_symlink()
          and 0 < args.archive.stat().st_size <= limit, "QA pack archive is missing or oversized")
    parent = args.output.parent.resolve(strict=True)
    parent_stat = parent.stat()
    check(parent.is_dir() and parent_stat.st_uid == os.getuid() and not parent_stat.st_mode & 0o022,
          "QA pack destination parent must be owned and protected from other writers")
    check(args.output.name not in {"", ".", ".."}, "QA pack requires a new installation directory")
    target = parent / args.output.name
    check(not target.exists() and not target.is_symlink(), "QA pack destination already exists")
    state = private_state_dir(args.state_dir) if args.state_dir is not None else None
    pointer = None
    if state is not None:
        if requested_model is None:
            pointer = state / "qa-pack.json"
        else:
            index = state / "qa-model-packs"
            index.mkdir(mode=0o700, exist_ok=True)
            pointer = private_state_dir(index) / f"{requested_model}.json"
    with pack_progress(state, "verifying_pack"), tempfile.TemporaryDirectory(prefix=".qa-import-", dir=parent) as tmp:
        # Extract the same bytes that were verified, even if the source archive is replaced.
        archive = Path(tmp) / "archive.tar.gz"
        with args.archive.open("rb") as source, archive.open("xb") as output:
            remaining = limit
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                check(len(chunk) <= remaining, "compressed QA pack exceeds its size limit")
                remaining -= len(chunk)
                output.write(chunk)
        check(sha256(archive) == args.archive_sha256, "pack archive SHA-256 mismatch")
        root = Path(tmp) / "pack"
        root.mkdir(mode=0o700)
        extract(archive, root, args.target, requested_model)
        data = verify_directory(root, args.target, args.manifest_sha256)
        installed_model = model_for_pack(data)
        check(requested_model is None or installed_model == requested_model,
              "verified pack does not contain the requested chat model")
        check(archive.stat().st_size <= MAX_BASE_COMPRESSED + GENERATOR_LIMITS.get(installed_model, 0),
              "compressed QA pack exceeds its size limit")
        check(not target.exists() and not target.is_symlink(), "QA pack destination already exists")
        root.rename(target)
        if pointer is not None:
            entry = {"root": str(target), "manifest_sha256": args.manifest_sha256}
            # Replace the pointer only after the complete target is installed. The previous
            # version remains usable if copying, extraction or validation is interrupted.
            with tempfile.NamedTemporaryFile("w", dir=pointer.parent, prefix=".qa-pack-", delete=False) as file:
                staged_pointer = Path(file.name)
                try:
                    json.dump(entry, file, sort_keys=True)
                    file.write("\n")
                    file.flush()
                    os.fsync(file.fileno())
                    os.replace(staged_pointer, pointer)
                finally:
                    staged_pointer.unlink(missing_ok=True)
        receipt = {"target": data["target"], "model_revision": data["model_revision"],
                   "manifest_sha256": args.manifest_sha256}
        if installed_model is not None:
            receipt["chat_model"] = installed_model
        return receipt


def install_gated_fixture(archive: Path, output: Path, gated_worker: Path, digests: dict) -> tuple[Path, dict]:
    """Re-pin a separate smoke-only installation with the exact gated worker."""
    info = gated_worker.lstat()
    check(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= MAX_BASE_UNPACKED,
          "gated fixture must be a bounded regular executable")
    gate_digest = sha256(gated_worker)
    install(argparse.Namespace(archive=archive, output=output, state_dir=None, target=digests["target"],
                               chat_model=digests.get("chat_model"),
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
    create.add_argument("--chat-model", choices=CHAT_MODELS, help="Select a pinned generator pack")
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
    importer.add_argument("--chat-model", choices=CHAT_MODELS,
                          help="Verify the requested generator and register its named Embedded pack")
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
