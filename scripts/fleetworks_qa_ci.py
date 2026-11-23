#!/usr/bin/env python3
"""Build the pinned Inbox QA CPU worker and its offline native pack."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import time

import fleetworks_qa_pack as pack

ROOT = Path(__file__).resolve().parents[1]
LINUX_SYSTEM = {"libc.so.6", "libm.so.6", "libpthread.so.0", "libdl.so.2", "librt.so.1",
                "libgcc_s.so.1", "libstdc++.so.6", "libutil.so.1", "libresolv.so.2"}
DARWIN_WHEELS = {
    "x86_64-apple-darwin": (
        "16/dd/1bf10180ba812afa1aa7427466083d731bc37b9a1157ec929d0cfeef87eb"
        "/torch-2.1.0-cp311-none-macosx_10_9_x86_64.whl",
        "601b0a2a9d9233fb4b81f7d47dca9680d4f3a78ca3f781078b6ad1ced8a90523",
    ),
    "aarch64-apple-darwin": (
        "7b/7c/4d8728e6f8dbe2b8af054bd92c290d94c633270443514e3ee4b768125cf9"
        "/torch-2.1.0-cp311-none-macosx_11_0_arm64.whl",
        "3cd1dedff13884d890f18eea620184fb4cd8fd3c68ce3300498f427ae93aa962",
    ),
}


def wheel(target):
    if target in DARWIN_WHEELS:
        path, digest = DARWIN_WHEELS[target]
        return f"https://files.pythonhosted.org/packages/{path}#sha256={digest}"
    equivalent = ROOT / "tools/atlas-bert/scripts/verify_linux.py"
    spec = importlib.util.spec_from_file_location("atlas_verify_linux", equivalent)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.wheel_url(target)


def run(*command, env=None, capture=False, timeout=900):
    return subprocess.run(command, cwd=ROOT, env=env, check=True, text=True, timeout=timeout,
                          stdout=subprocess.PIPE if capture else None).stdout


def readonly_toolchain(target):
    machine = platform.machine()
    expected = "arm64" if target == "aarch64-apple-darwin" else target.split("-")[0]
    pack.check(machine == expected, "Inbox QA packs require a native architecture runner")
    pack.check((platform.system() == "Darwin") == (target in DARWIN_WHEELS),
               "Inbox QA target differs from the runner OS")
    if target not in DARWIN_WHEELS:
        pack.check(platform.libc_ver() == ("glibc", "2.36"), "Inbox QA Linux pack requires Debian bookworm")


def torch_environment():
    import torch

    pack.check(torch.__version__.split("+")[0] == "2.1.0" and torch.version.cuda is None,
               "pack requires the pinned CPU Torch")
    root = Path(torch.__file__).resolve().parent
    env = dict(os.environ)
    for key in ("LIBTORCH", "LIBTORCH_INCLUDE", "LIBTORCH_LIB", "LIBTORCH_BYPASS_VERSION_CHECK"):
        env.pop(key, None)
    env["LIBTORCH_USE_PYTORCH"] = "1"
    env["PATH"] = str(Path(sys.executable).parent) + os.pathsep + env["PATH"]
    key = "DYLD_LIBRARY_PATH" if platform.system() == "Darwin" else "LD_LIBRARY_PATH"
    env[key] = os.pathsep.join(map(str, (root / "lib", root.parent / "torch.libs")))
    return root, key, env


def linux_libraries(binary, directory, env, installed=False):
    result = run("ldd", str(binary), env=env, capture=True, timeout=60)
    pack.check("not found" not in result, "Inbox QA worker library could not be resolved")
    paths = []
    for name, source in re.findall(r"^\s*(\S+)\s+=>\s+(/\S+)", result, re.M):
        if name in LINUX_SYSTEM:
            continue
        source = Path(source)
        if installed:
            pack.check(source.resolve().parent == directory.resolve(), "Inbox QA linked outside its pack")
        else:
            shutil.copyfile(source, directory / name)
        paths.append(directory / name)
    versions = run("readelf", "--version-info", str(binary), *map(str, paths), capture=True, timeout=60)
    for prefix, maximum in (("GLIBC", (2, 36)), ("GLIBCXX", (3, 4, 30)), ("CXXABI", (1, 3, 13))):
        pack.check(all(tuple(map(int, value.split("."))) <= maximum for value in
                       re.findall(r"\b" + prefix + r"_([0-9]+(?:\.[0-9]+)+)\b", versions)),
                   "Inbox QA dependencies exceed the Debian runtime ABI")
    return paths


def direct_mach_libraries(path):
    return [line.split(" (compatibility version ", 1)[0].strip()
            for line in run("otool", "-L", str(path), capture=True, timeout=60).splitlines()[1:]
            if line.strip()]


def mac_libraries(binary, directory, torch_root):
    libraries = {}
    queue = [binary]
    while queue:
        current = queue.pop()
        changes = []
        for source in direct_mach_libraries(current):
            if source.startswith(("/usr/lib/", "/System/Library/")):
                continue
            name = Path(source).name
            pack.check(name.endswith(".dylib"), "Inbox QA linked an unexpected Mach-O dependency")
            if name == current.name:
                changes.extend(("-id", "@loader_path/" + name))
                continue
            if name not in libraries:
                candidates = (torch_root / "lib" / name, torch_root.parent / "torch.libs" / name)
                library = next((path for path in candidates if path.is_file()), None)
                pack.check(library is not None, "Inbox QA linked a library outside the pinned wheel")
                output = directory / name
                shutil.copyfile(library, output)
                output.chmod(0o644)
                libraries[name] = output
                queue.append(output)
            prefix = "@loader_path/../lib/" if current == binary else "@loader_path/"
            replacement = prefix + name
            if replacement != source:
                changes.extend(("-change", source, replacement))
        if changes:
            run("install_name_tool", *changes, str(current), timeout=60)
        run("codesign", "--force", "--sign", "-", str(current), timeout=60)
    return list(libraries.values())


def verify_inference(root, loader_key, target, *, dialogue=False):
    # The extracted pack must find all runtime libraries and use the exact
    # checkpoint without inheriting Torch or model-cache environment variables.
    model_env = {loader_key: str(root / "lib"), "OMP_NUM_THREADS": "1", "MKL_NUM_THREADS": "1",
                 "RAYON_NUM_THREADS": "1"}
    if dialogue:
        model_env.update(FLEETWORKS_QA_DIALOGUE="1", FLEETWORKS_QA_CONTEXT_USAGE="1")
    started = time.monotonic()
    worker = root / pack.WORKER
    model = root / "model"
    command = [str(worker), "--model-dir", str(model)]
    if target in DARWIN_WHEELS:
        launcher = ROOT / "services/api-host/src/qa_embedded_macos.sh"
        profile = launcher.with_suffix(".sb")
        command = ["/bin/sh", "-c", launcher.read_text(encoding="utf-8"), "fleetworks-inbox-qa",
                   target.split("-", 1)[0], "-f", str(profile), "-D", f"pack={root}",
                   "-D", f"worker={worker}", *command]
    error_log = tempfile.TemporaryFile()
    process = subprocess.Popen(command, env=model_env, cwd=model, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=error_log)
    deadline = started + 60

    def send(message):
        encoded = json.dumps(message, separators=(",", ":")).encode()
        process.stdin.write(struct.pack(">I", len(encoded)) + encoded)
        process.stdin.flush()

    def receive():
        fd = process.stdout.fileno()

        def read(size):
            output = bytearray()
            while len(output) < size:
                remaining = max(deadline - time.monotonic(), 0)
                pack.check(select.select([fd], [], [], remaining)[0], "QA checkpoint smoke timed out")
                chunk = os.read(fd, size - len(output))
                pack.check(chunk, "QA checkpoint worker stopped unexpectedly")
                output.extend(chunk)
            return output

        size, = struct.unpack(">I", read(4))
        pack.check(0 < size <= 512 * 1024, "QA checkpoint reply exceeds its limit")
        return json.loads(read(size))

    try:
        ready = "ready_with_dialogue" if dialogue else "ready"
        pack.check(receive() == {"reply": ready, "protocol": 1, "model_revision": pack.MODEL_REVISION},
                   "QA checkpoint did not become ready")
        warm = time.monotonic()
        if dialogue:
            deadline = min(deadline, warm + 25)
            request = {
                "turn_id": "pack-dialogue-probe", "snapshot_id": "pack-dialogue-probe",
                "question": "Hello!", "passages": [],
                "dialogue": {"kind": "greeting", "history": [
                    {"question": "What can I call you?", "reply": "You can call me Fleetworks."},
                    {"question": "Good morning.", "reply": "Good morning!"},
                ]},
            }
            send({"command": "ask", "request": request})
            terminal = receive()
            event = terminal.get("event", {})
            reply = event.get("reply")
            usage = event.get("context", {})
            pack.check(terminal.get("reply") == "event" and event.get("kind") == "complete"
                       and event.get("turn_id") == request["turn_id"] and event.get("sequence") == 1
                       and event.get("outcome") == "answered" and isinstance(reply, str)
                       and bool(reply.strip()) and reply.isprintable() and len(reply.encode("utf-8")) <= 1024
                       and len(reply.split()) <= 60, "generator did not complete the bounded dialogue turn")
            pack.check(usage.get("window_tokens") == 256 and usage.get("model_windows") == 1
                       and type(usage.get("peak_input_tokens")) is int and 0 < usage["peak_input_tokens"] <= 256,
                       "generator did not report its bounded input window")
        else:
            send({"command": "ask", "request": {"turn_id": "pack-probe", "snapshot_id": "pack-probe",
                  "question": "What is the state of actor search?",
                  "passages": [{"source_id": "readiness", "text": "Actor search is ready."}]}})
            finding = {"reply": "event", "event": {"kind": "finding", "turn_id": "pack-probe",
                       "sequence": 1, "source_id": "readiness", "start": 16, "end": 21, "text": "ready"}}
            pack.check(receive() == finding, "pinned checkpoint did not extract the expected exact answer")
            pack.check(receive() == {"reply": "event", "event": {"kind": "complete", "turn_id": "pack-probe",
                       "sequence": 2, "outcome": "answered"}}, "QA checkpoint did not complete the turn")
        complete = time.monotonic()
        send({"command": "shutdown"})
        pack.check(process.wait(timeout=10) == 0, "QA checkpoint worker did not shut down")
        if dialogue:
            return {"model_warm_ms": round((warm - started) * 1000, 1),
                    "dialogue_ms": round((complete - warm) * 1000, 1), "context": usage}
        return {"model_warm_ms": round((warm - started) * 1000, 1),
                "single_passage_ms": round((complete - warm) * 1000, 1)}
    except Exception:
        error_log.seek(0)
        diagnostic = error_log.read(16 * 1024)
        numbers = re.findall(rb"^OMP: (?:System error|Error) #(\d{1,3}):", diagnostic, re.M)
        result = {"worker_exit": process.poll(), "openmp_error_numbers": sorted({int(n) for n in numbers}),
                  "openmp_shared_memory_error": b"Can't open SHM" in diagnostic,
                  "openmp_temporary_file_error": b"Can't open TEMP" in diagnostic}
        print(json.dumps(result, sort_keys=True), file=sys.stderr)
        raise
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        process.stdin.close()
        process.stdout.close()
        error_log.close()


def build(target, directory):
    readonly_toolchain(target)
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    staging = directory / "staging"
    libraries = staging / "runtime-libraries"
    libraries.mkdir(parents=True)
    model = staging / "model"
    run("bash", "tools/assistant-bert/scripts/prepare-model.sh", "--with-generator", str(model), timeout=1080)
    torch_root, loader_key, env = torch_environment()
    manifest = "tools/assistant-bert/Cargo.toml"
    run("cargo", "fmt", "--manifest-path", manifest, "--check")
    command = [sys.executable, "scripts/agent_cargo.py"]
    common = ["--locked", "--release", "--manifest-path", manifest]
    run(*command, "test", *common, env=env)
    run(*command, "build", *common, "--bins", env=env)
    binary = staging / Path(pack.WORKER).name
    listener = staging / Path(pack.LISTENER).name
    for path in (binary, listener):
        shutil.copy2(ROOT / "target/agents/release" / path.name, path)
    native = (mac_libraries(binary, libraries, torch_root) if target in DARWIN_WHEELS
              else linux_libraries(binary, libraries, env))
    notices = []
    for info in sorted(torch_root.parent.glob("torch-*.dist-info")):
        for source in sorted([*info.glob("LICENSE"), *info.glob("NOTICE*")]):
            target_path = staging / ("torch-" + source.name)
            shutil.copyfile(source, target_path)
            notices.append(target_path)
    pack.check(staging / "torch-LICENSE" in notices, "pinned CPU Torch distribution license is missing")

    archive = directory / f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
    digests = pack.build(argparse.Namespace(model=model, worker=binary, listener=listener,
                                           lib=native, license=notices, output=archive, target=target))
    installed = directory / "installed"
    pack.install(argparse.Namespace(archive=archive, output=installed, state_dir=None, target=target,
                                    archive_sha256=digests["archive_sha256"],
                                    manifest_sha256=digests["manifest_sha256"]))
    if target not in DARWIN_WHEELS:
        linux_libraries(installed / pack.WORKER, installed / "lib",
                        {**env, loader_key: str(installed / "lib")}, installed=True)
    digests["checkpoint_smoke"] = verify_inference(installed, loader_key, target)
    digests["dialogue_smoke"] = verify_inference(installed, loader_key, target, dialogue=True)
    (directory / "digests.json").write_text(json.dumps(digests, sort_keys=True) + "\n")
    print(json.dumps(digests, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("wheel")
    command.add_argument("--target", required=True, choices=pack.TARGETS)
    command = commands.add_parser("build")
    command.add_argument("--target", required=True, choices=pack.TARGETS)
    command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "wheel":
        print(wheel(args.target))
    else:
        build(args.target, args.output)


if __name__ == "__main__":
    main()
