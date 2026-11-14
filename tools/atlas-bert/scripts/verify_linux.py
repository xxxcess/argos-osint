#!/usr/bin/env python3
"""Build and verify a native Linux CPU worker bundle in Debian 12 only.

This developer/CI helper never installs engines, invokes Docker, publishes,
or touches a native workspace. The caller provisions the pinned CPU wheel in
an isolated Linux builder. Runtime inference is real, with the existing pinned
checkpoint preparation script, and remains distinct from gateway integration.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import struct
import subprocess
import sys
import tarfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
WORKER = "argos-atlas-bert"
MODEL_COMMIT = "cfa538a0fddbbd978fefe8966c1aeff7ad409c90"
TARGETS = {
    "x86_64-unknown-linux-gnu": {"arch": "amd64", "machine": "x86_64", "elf": 62,
        "wheel": "torch-2.1.0%2Bcpu-cp311-cp311-linux_x86_64.whl",
        "sha256": "5954924ce74bc7e6a6c811e3fa4bdda9936d9889f6369fd068420c444bfd1cae"},
    "aarch64-unknown-linux-gnu": {"arch": "arm64", "machine": "aarch64", "elf": 183,
        "wheel": "torch-2.1.0-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
        "sha256": "8132efb782cd181cc2dcca5e58effbe4217cdb2581206ac71466d535bf778867"},
}
MAX_LOG = 64 * 1024 * 1024
MAX_BUNDLE = 1024 * 1024 * 1024
MAX_FILES = 2048
BASELINE_LIBRARIES = frozenset(("libc.so.6", "libm.so.6", "libpthread.so.0", "libdl.so.2", "librt.so.1",
                               "libgcc_s.so.1", "libstdc++.so.6", "libutil.so.1", "libresolv.so.2"))


class VerifyError(Exception):
    """Static failure reason; command logs retain isolated build diagnostics."""


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def wheel_url(target):
    selected = TARGETS[target]
    return "https://download.pytorch.org/whl/cpu/" + selected["wheel"] + "#sha256=" + selected["sha256"]


def host_guard(target, *, system=None, machine=None, release=None, glibc=None):
    system = platform.system() if system is None else system
    machine = platform.machine() if machine is None else machine
    if release is None:
        release = Path("/etc/os-release").read_text() if system == "Linux" else ""
    glibc = (os.confstr("CS_GNU_LIBC_VERSION") or "") if glibc is None and system == "Linux" else glibc
    if system != "Linux" or machine != TARGETS[target]["machine"]:
        raise VerifyError("Native matching Linux hardware is required; emulation/cross execution is unsupported")
    if not re.search(r'^ID=["\']?debian["\']?$', release, re.M) or not re.search(r'^VERSION_ID=["\']?12["\']?$', release, re.M) or glibc != "glibc 2.36":
        raise VerifyError("Build in native Debian 12/bookworm with glibc 2.36 for the deployment runtime ABI")


def elf(path, target):
    with path.open("rb") as source:
        header = source.read(64)
    if len(header) != 64 or header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != TARGETS[target]["elf"]:
        raise VerifyError("Worker or native library has an unexpected ELF architecture")


def executable(output):
    paths = set()
    for line in output.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        target = event.get("target", {})
        if event.get("reason") == "compiler-artifact" and target.get("name") == WORKER and "bin" in target.get("kind", []) and event.get("executable"):
            paths.add(event["executable"])
    if len(paths) != 1:
        raise VerifyError("Cargo did not report exactly one worker compiler-artifact executable")
    return Path(paths.pop()).resolve()


def unit_result(output):
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", output)
    if not results or sum(int(result[0]) for result in results) == 0 or any(int(failed) or int(ignored) for _, failed, ignored in results):
        raise VerifyError("Worker unit tests had no executed passing cases, failures, or ignored cases")
    return sum(int(result[0]) for result in results)


ELECTION = "Parliament voted to amend election law and the electoral commission updated the ballot rules."
RESEARCH = "Researchers published an experiment improving error correction on a silicon quantum processor."
BATCHES = [[ELECTION, RESEARCH], [RESEARCH], [ELECTION], [ELECTION, RESEARCH]]


def inference_result(output):
    try:
        ready, *responses = [json.loads(line) for line in output.splitlines()]
        if ready != {"protocol": "argos-atlas-bert-v1"} or len(responses) != len(BATCHES):
            raise ValueError()
        for batch, response in zip(BATCHES, responses):
            if set(response) != {"scores"} or len(response["scores"]) != len(batch):
                raise ValueError()
            for row in response["scores"]:
                if len(row) != 6 or any(type(score) not in (int, float) or not math.isfinite(score) or not 0 <= score <= 1 for score in row):
                    raise ValueError()
        first, research_only, election_only, repeated = [response["scores"] for response in responses]
        for row, single, repeat in zip(first, [election_only[0], research_only[0]], repeated):
            if any(not math.isclose(a, b, abs_tol=1e-5) or not math.isclose(a, c, abs_tol=1e-5) for a, b, c in zip(row, single, repeat)):
                raise ValueError()
        if not first[0][0] > first[1][0] or not first[1][5] > first[0][5]:
            raise ValueError()
    except (ValueError, KeyError, TypeError, IndexError):
        raise VerifyError("Real checkpoint inference failed handshake, six-domain ordering, or batch/repeat invariance") from None
    return {"status": "passed", "requests": len(BATCHES), "texts": sum(map(len, BATCHES)), "domains": 6}


def symbol_versions(output):
    result = {}
    for prefix, ceiling in (("GLIBC", (2, 36)), ("GLIBCXX", (3, 4, 30)), ("CXXABI", (1, 3, 13))):
        versions = [tuple(map(int, text.split("."))) for text in re.findall(r"\b" + prefix + r"_([0-9]+(?:\.[0-9]+)+)\b", output)]
        if versions and max(versions) > ceiling:
            raise VerifyError("Native symbols exceed the Debian bookworm runtime ABI")
        result[prefix] = ".".join(map(str, max(versions))) if versions else None
    return result


def linked_libraries(output, bundle):
    if "not found" in output:
        raise VerifyError("Bundle has an unresolved dynamic library")
    found = set()
    for name, path in re.findall(r"^\s*(\S+)\s+=>\s+(\S+)", output, re.M):
        if name not in BASELINE_LIBRARIES:
            resolved = Path(path).resolve()
            if not resolved.is_relative_to(bundle.resolve()):
                raise VerifyError("Worker loaded a non-baseline native library outside the extracted bundle")
            found.add(name)
    if not any(name.startswith("libtorch_cpu") for name in found) or not any(name.startswith("libc10") for name in found):
        raise VerifyError("Worker did not resolve its required bundled CPU Torch libraries")
    return sorted(found)


def copy_external_libraries(output, bundle, target):
    """Retain non-baseline dependency closure, including system OpenMP.

    The deployment already provides bookworm glibc/libgcc/libstdc++; Torch may
    use a wheel-private or Debian OpenMP/BLAS dependency, which must travel.
    """
    if "not found" in output:
        raise VerifyError("Build worker has an unresolved native dependency")
    copied, origins = [], {}
    for name, path in re.findall(r"^\s*(\S+)\s+=>\s+(\S+)", output, re.M):
        if name in BASELINE_LIBRARIES:
            continue
        if not re.fullmatch(r"[A-Za-z0-9_.+-]+", name) or not Path(path).is_absolute():
            raise VerifyError("Native dependency has an unsupported library identity")
        source = Path(path).resolve()
        elf(source, target)
        # Existing wheel files keep their layout and bundled RPATH semantics.
        existing = list((bundle / "python").rglob(name))
        if existing:
            if len(existing) != 1 or sha(existing[0]) != sha(source):
                raise VerifyError("Native library name collides with a different wheel object")
            continue
        destination = bundle / "python/torch/lib" / name
        shutil.copyfile(source, destination)
        copied.append(destination)
        origins[name] = str(source)
        if len(copied) > 64:
            raise VerifyError("Native dependency closure exceeds its bound")
    return copied, origins


class Commands:
    def __init__(self, output):
        self.output, self.records = output, []

    def run(self, name, argv, *, env=None, body=None, timeout=1800, limit=MAX_LOG):
        stdout, stderr = self.output / (name + ".stdout.log"), self.output / (name + ".stderr.log")
        record = {"name": name, "argv": list(map(str, argv)), "status": "failed"}
        self.records.append(record)
        try:
            with stdout.open("wb") as out, stderr.open("wb") as err:
                result = subprocess.run(record["argv"], cwd=ROOT, env=env, input=body,
                                        stdout=out, stderr=err, timeout=timeout, check=False)
            record["exit_code"] = result.returncode
            for kind, path in (("stdout", stdout), ("stderr", stderr)):
                size = path.stat().st_size
                if size > limit:
                    with path.open("r+b") as log:
                        log.truncate(limit)
                    record["truncated"] = True
                record[kind] = {"file": path.name, "bytes": path.stat().st_size, "sha256": sha(path)}
            if result.returncode or record.get("truncated"):
                raise VerifyError("An isolated worker verification command failed; inspect its bounded logs")
            record["status"] = "passed"
            return stdout.read_text()
        except subprocess.TimeoutExpired:
            record["timed_out"] = True
            raise VerifyError("An isolated worker verification command timed out") from None


def payload_files(root):
    result, total = [], 0
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            raise VerifyError("Bundle contains a link or unsupported filesystem object")
        if path.is_file():
            total += path.stat().st_size
            result.append(path)
            if len(result) > MAX_FILES or total > MAX_BUNDLE:
                raise VerifyError("Bundle exceeds bounded file count or bytes")
    return result


def copy_libraries(torch_root, bundle, target):
    sources = [torch_root / "lib", torch_root.parent / "torch.libs"]
    copied = []
    for source in sources:
        if not source.exists():
            continue
        destination = bundle / "python" / ("torch/lib" if source.name == "lib" else "torch.libs")
        destination.mkdir(parents=True)
        for path in sorted(source.iterdir()):
            if not path.is_file() or ".so" not in path.name:
                continue
            if not path.resolve().is_relative_to(source.resolve()):
                raise VerifyError("Torch library reference escapes its selected wheel directory")
            elf(path, target)
            shutil.copyfile(path, destination / path.name)
            copied.append(destination / path.name)
            if len(copied) > 128 or sum(value.stat().st_size for value in copied) > MAX_BUNDLE:
                raise VerifyError("Native Torch library bundle exceeds its bound")
    if not copied:
        raise VerifyError("Pinned wheel has no native runtime libraries")
    return copied


def checkpoint_pins():
    script = (ROOT / "tools/atlas-bert/scripts/prepare-model.sh").read_text()
    if MODEL_COMMIT not in script:
        raise VerifyError("Checkpoint source revision changed; update the reviewed verification contract")
    values = dict((name, digest) for digest, name in re.findall(r"^([0-9a-f]{64})  (config\.json|vocab\.txt|pytorch_model\.bin)$", script, re.M))
    if set(values) != {"config.json", "vocab.txt", "pytorch_model.bin"}:
        raise VerifyError("Pinned checkpoint checksums are missing or ambiguous")
    return values


def safe_unpack(archive, destination):
    destination.mkdir(mode=0o700)
    total = 0
    with tarfile.open(archive, "r:gz") as opened:
        members = opened.getmembers()
        if len(members) > MAX_FILES:
            raise VerifyError("Archive contains too many entries")
        names = set()
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not path.parts or str(path) != member.name or member.name in names or not member.isfile():
                raise VerifyError("Archive has an unsafe, duplicate or non-file entry")
            names.add(member.name)
            total += member.size
            if total > MAX_BUNDLE or member.size < 0 or member.mode & 0o7000:
                raise VerifyError("Archive exceeds its bound or has unsupported permissions")
            target = destination.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with opened.extractfile(member) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)


def check_bundle(bundle, target, revision):
    manifest = json.loads((bundle / "manifest.json").read_text())
    if manifest.get("format_version") != 1 or manifest.get("source_sha") != revision or manifest.get("target") != target or manifest.get("status") != "runtime_verified":
        raise VerifyError("Bundle source, target or runtime receipt does not match")
    expected = manifest["files"]
    actual = {str(path.relative_to(bundle)) for path in payload_files(bundle)} - {"manifest.json", "SHA256SUMS"}
    if set(expected) != actual:
        raise VerifyError("Bundle has missing or unlisted payloads")
    for name in actual:
        if sha(bundle / name) != expected[name]["sha256"] or (bundle / name).stat().st_size != expected[name]["bytes"]:
            raise VerifyError("Bundle payload checksum mismatch")
    wanted = {str(path.relative_to(bundle)): sha(path) for path in payload_files(bundle) if path.relative_to(bundle) != Path("SHA256SUMS")}
    recorded = {}
    for line in (bundle / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        if name in recorded:
            raise VerifyError("Duplicate bundle checksum entry")
        recorded[name] = digest
    if wanted != recorded:
        raise VerifyError("Bundle checksum inventory does not match")
    binary = bundle / "bin" / WORKER
    elf(binary, target)
    if not os.access(binary, os.X_OK):
        raise VerifyError("Bundled worker executable permission is missing")
    return manifest


def run(arguments):
    host_guard(arguments.target)
    if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision):
        raise VerifyError("An exact source SHA is required")
    if subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip() != arguments.revision:
        raise VerifyError("Checked-out source does not match the requested SHA")
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=normal"], cwd=ROOT, text=True).strip())
    if arguments.require_clean and dirty:
        raise VerifyError("CI runtime evidence requires a clean source checkout")
    if os.environ.get("CARGO_BUILD_TARGET") or os.environ.get("LIBTORCH_BYPASS_VERSION_CHECK"):
        raise VerifyError("Implicit native target and unmodified Torch version checks are required")
    output = arguments.output.absolute()
    output.mkdir(mode=0o700, parents=False, exist_ok=False)
    commands = Commands(output)
    evidence = {"format_version": 1, "status": "failed", "source_sha": arguments.revision,
                "dirty": dirty, "target": arguments.target, "arch": TARGETS[arguments.target]["arch"],
                "commands": commands.records, "gateway_integration": "not_run", "publication": "not_performed"}
    try:
        rust = commands.run("rustc", ["rustc", "-vV"])
        if "host: " + arguments.target + "\n" not in rust:
            raise VerifyError("Rust toolchain does not target the native Linux builder")
        pin = re.search(r'^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', (ROOT / "rust-toolchain.toml").read_text(), re.M)
        if not pin or "release: " + pin[1] + "\n" not in rust:
            raise VerifyError("Rust version differs from the exact repository toolchain pin")
        evidence["toolchain"] = rust.strip()
        evidence["worker_lock_sha256"] = sha(ROOT / "tools/atlas-bert/Cargo.lock")
        info = json.loads(commands.run("torch", [sys.executable, "-c",
            "import json,pathlib,torch; print(json.dumps({'version':torch.__version__,'cuda':torch.version.cuda,'cxx11_abi':torch._C._GLIBCXX_USE_CXX11_ABI,'root':str(pathlib.Path(torch.__file__).parent)}))"]))
        if info["version"].split("+")[0] != "2.1.0" or info["cuda"] is not None:
            raise VerifyError("The exact pinned CPU PyTorch 2.1.0 wheel is required")
        torch_root = Path(info.pop("root")).resolve()
        evidence["torch"] = {**info, "wheel_url": wheel_url(arguments.target)}
        env = os.environ.copy()
        env["LIBTORCH_USE_PYTORCH"] = "1"
        env["LD_LIBRARY_PATH"] = str(torch_root / "lib") + ":" + str(torch_root.parent / "torch.libs")
        env["CARGO_TARGET_DIR"] = str(ROOT / "target/agents") if arguments.agent else str(output / "cargo-target")
        cargo = [sys.executable, str(ROOT / "scripts/agent_cargo.py")] if arguments.agent else ["cargo"]
        common = ["--locked", "--manifest-path", str(ROOT / "tools/atlas-bert/Cargo.toml")]
        build = commands.run("build", [*cargo, "build", *common, "--message-format=json-render-diagnostics"], env=env)
        binary = executable(build)
        elf(binary, arguments.target)
        evidence["unit_tests_passed"] = unit_result(commands.run("unit-tests", [*cargo, "test", *common], env=env))
        model = output / "checkpoint"
        commands.run("prepare-model", ["bash", str(ROOT / "tools/atlas-bert/scripts/prepare-model.sh"), str(model)], env=env)
        pins = checkpoint_pins()
        for name in ("config.json", "vocab.txt"):
            if sha(model / name) != pins[name]:
                raise VerifyError("Prepared checkpoint configuration or vocabulary changed")
        body = "".join(json.dumps({"texts": batch}) + "\n" for batch in BATCHES).encode()
        evidence["build_inference"] = inference_result(commands.run("inference", [str(binary), "serve", str(model)], env=env, body=body, timeout=180, limit=64 * 1024))
        bundle = output / "bundle"
        (bundle / "bin").mkdir(parents=True)
        shutil.copy2(binary, bundle / "bin" / WORKER)
        libraries = copy_libraries(torch_root, bundle, arguments.target)
        extra, origins = copy_external_libraries(commands.run("build-linkage", ["ldd", str(binary)], env=env), bundle, arguments.target)
        libraries += extra
        evidence["additional_runtime_libraries"] = origins
        evidence["runtime_abi"] = symbol_versions(commands.run("symbols", ["readelf", "--version-info", str(binary), *map(str, libraries)]))
        shutil.copytree(model, bundle / "model")
        license_files = sorted(torch_root.parent.glob("torch-*.dist-info/LICENSE*")) + sorted(torch_root.parent.glob("torch-*.dist-info/NOTICE*"))
        if not license_files:
            raise VerifyError("Pinned Torch distribution license is missing")
        (bundle / "licenses").mkdir()
        for source in license_files:
            shutil.copyfile(source, bundle / "licenses" / source.name)
        launcher = bundle / "run-worker"
        launcher.write_text('#!/bin/sh\nset -eu\nhere=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)\nexport LD_LIBRARY_PATH="$here/python/torch/lib:$here/python/torch.libs"\nexec "$here/bin/argos-atlas-bert" "$@"\n')
        launcher.chmod(0o755)
        manifest = {"format_version": 1, "status": "runtime_verified", "source_sha": arguments.revision,
                    "dirty": dirty, "target": arguments.target, "toolchain": evidence["toolchain"],
                    "worker_lock_sha256": evidence["worker_lock_sha256"],
                    "torch": evidence["torch"], "runtime_abi": evidence["runtime_abi"],
                    "additional_runtime_libraries": origins,
                    "checkpoint": {"repository": "typeform/distilbert-base-uncased-mnli", "commit": MODEL_COMMIT,
                                   "source_sha256": pins, "conversion_script_sha256": sha(ROOT / "tools/atlas-bert/scripts/prepare-model.sh")},
                    "files": {str(path.relative_to(bundle)): {"bytes": path.stat().st_size, "sha256": sha(path)} for path in payload_files(bundle)}}
        save(bundle / "manifest.json", manifest)
        (bundle / "SHA256SUMS").write_text("".join(sha(path) + "  " + str(path.relative_to(bundle)) + "\n" for path in payload_files(bundle)))
        archive = output / ("atlas-bert-linux-" + evidence["arch"] + ".tar.gz")
        with tarfile.open(archive, "w:gz") as opened:
            for path in payload_files(bundle):
                opened.add(path, arcname=str(path.relative_to(bundle)), recursive=False)
        extracted = output / "extracted"
        safe_unpack(archive, extracted)
        check_bundle(extracted, arguments.target, arguments.revision)
        isolated = env.copy()
        isolated.pop("LIBTORCH_USE_PYTORCH", None)
        isolated["LD_LIBRARY_PATH"] = str(extracted / "python/torch/lib") + ":" + str(extracted / "python/torch.libs")
        evidence["resolved_libraries"] = linked_libraries(commands.run("bundle-linkage", ["ldd", str(extracted / "bin" / WORKER)], env=isolated), extracted)
        evidence["extracted_inference"] = inference_result(commands.run("extracted-inference", [str(extracted / "run-worker"), "serve", str(extracted / "model")], env=isolated, body=body, timeout=180, limit=64 * 1024))
        evidence["archive"] = {"file": archive.name, "bytes": archive.stat().st_size, "sha256": sha(archive)}
        evidence["manifest_sha256"] = sha(extracted / "manifest.json")
        evidence["status"] = "passed"
    finally:
        save(output / "evidence.json", evidence)
    return evidence


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    wheel = commands.add_parser("wheel", help="Print the reviewed CPU wheel URL/hash; no install")
    wheel.add_argument("--target", choices=TARGETS, required=True)
    verify = commands.add_parser("run", help="Build/test/real-infer/package/re-extract on matching Debian Linux")
    verify.add_argument("--target", choices=TARGETS, required=True)
    verify.add_argument("--revision", required=True)
    verify.add_argument("--output", type=Path, required=True)
    verify.add_argument("--agent", action="store_true")
    verify.add_argument("--require-clean", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "wheel":
            print(wheel_url(arguments.target))
        else:
            evidence = run(arguments)
            print(json.dumps({key: evidence[key] for key in ("status", "source_sha", "target", "archive")}))
        return 0
    except (VerifyError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(str(error) if isinstance(error, VerifyError) else "Linux worker verification input or command failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
