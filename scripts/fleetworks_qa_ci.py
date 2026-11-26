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
FLAN_INPUT_WINDOW = 512
GPT2_INPUT_WINDOW = 1024 - 128
GPT2_MODELS = {"gpt2-small", "gpt2-medium"}
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


def linux_libraries(binary, directory, env, installed=False, *, torch_root=None):
    if installed:
        library_dirs = (directory.resolve(),)
    else:
        pack.check(torch_root is not None, "Copying Linux libraries requires the pinned Torch wheel")
        library_dirs = (torch_root / "lib", torch_root.parent / "torch.libs")
    result = run("ldd", str(binary), env=env, capture=True, timeout=60)
    pack.check("not found" not in result, "Inbox QA worker library could not be resolved")
    paths = []
    for name, source in re.findall(r"^\s*(\S+)\s+=>\s+(/\S+)", result, re.M):
        if name in LINUX_SYSTEM:
            continue
        source = Path(source)
        pack.check(source.resolve().parent in library_dirs, "Inbox QA linked a library outside its expected root")
        if not installed:
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


def prepare_assets(directory, chat_model):
    pin = pack.CHAT_MODELS[chat_model]
    directory.mkdir(mode=0o700, parents=True)
    url = f"https://huggingface.co/{pin['repository']}/resolve/{pin['revision']}"
    for name, asset in pin["files"].items():
        with tempfile.NamedTemporaryFile(dir=directory, prefix=f".{name}-", delete=False) as output:
            temporary = Path(output.name)
        try:
            run("curl", "--fail", "--silent", "--show-error", "--location",
                "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "5",
                "--retry", "3", "--max-time", "1200", "--max-filesize", str(asset["bytes"]),
                f"{url}/{name}", "--output", str(temporary), timeout=1240)
            pack.check(temporary.stat().st_size == asset["bytes"] and pack.sha256(temporary) == asset["sha256"],
                       f"{name} does not match the pinned chat model")
            os.replace(temporary, directory / name)
        finally:
            temporary.unlink(missing_ok=True)


def verify_inference(root, loader_key, target, chat_model=pack.DEFAULT_CHAT_MODEL, *, model_directory=None):
    # The extracted pack must find all runtime libraries and use the exact
    # checkpoint without inheriting Torch or model-cache environment variables.
    model_env = {loader_key: str(root / "lib"), "OMP_NUM_THREADS": "1", "MKL_NUM_THREADS": "1",
                 "RAYON_NUM_THREADS": "1", "FLEETWORKS_QA_DIALOGUE": "1",
                 "FLEETWORKS_QA_CONTEXT_USAGE": "1", "FLEETWORKS_QA_STREAMING": "1",
                 "FLEETWORKS_QA_MODEL_TARGET": chat_model}
    extended_context = chat_model in GPT2_MODELS
    if extended_context:
        model_env["FLEETWORKS_QA_MODEL_CONTEXT"] = "1"
    window = GPT2_INPUT_WINDOW if extended_context else FLAN_INPUT_WINDOW
    started = time.monotonic()
    worker = root / pack.WORKER
    model = model_directory or root / "model"
    command = [str(worker), "--model-dir", str(model)]
    if target in DARWIN_WHEELS:
        launcher = ROOT / "services/api-host/src/qa_embedded_macos.sh"
        profile = launcher.with_suffix(".sb")
        command = ["/bin/sh", "-c", launcher.read_text(encoding="utf-8"), "fleetworks-inbox-qa",
                   target.split("-", 1)[0], "-f", str(profile), "-D", f"pack={root}",
                   "-D", f"worker={worker}", "-D", f"model={model}", *command]
    error_log = tempfile.TemporaryFile()
    process = subprocess.Popen(command, env=model_env, cwd=model, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=error_log)
    deadline = started + 45
    peak_rss = None
    rss_samples = 0

    def sample_rss():
        nonlocal peak_rss, rss_samples
        try:
            if target in DARWIN_WHEELS:
                result = subprocess.run(["/bin/ps", "-o", "rss=", "-p", str(process.pid)],
                                        capture_output=True, text=True, timeout=1)
                if result.returncode != 0:
                    return
                kib = int(result.stdout.strip())
            else:
                status = Path(f"/proc/{process.pid}/status").read_text()
                match = re.search(r"^VmHWM:\s+(\d+) kB$", status, re.M)
                if match is None:
                    return
                kib = int(match[1])
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return
        if kib > 0:
            rss_samples += 1
            peak_rss = max(peak_rss or 0, kib * 1024)

    def send(message):
        encoded = json.dumps(message, separators=(",", ":")).encode()
        process.stdin.write(struct.pack(">I", len(encoded)) + encoded)
        process.stdin.flush()

    def receive():
        fd = process.stdout.fileno()

        def read(size):
            output = bytearray()
            while len(output) < size:
                remaining = deadline - time.monotonic()
                pack.check(remaining > 0, "QA checkpoint smoke timed out")
                if not select.select([fd], [], [], min(remaining, 0.25))[0]:
                    sample_rss()
                    continue
                chunk = os.read(fd, size - len(output))
                pack.check(chunk, "QA checkpoint worker stopped unexpectedly")
                output.extend(chunk)
            return output

        size, = struct.unpack(">I", read(4))
        pack.check(0 < size <= 512 * 1024, "QA checkpoint reply exceeds its limit")
        return json.loads(read(size))

    def has_prompt_marker(text):
        text = text.lower()
        return any(marker in text for marker in (
            "earlier user", "earlier assistant", "current user", "approved observations:", "reply:",
        ))

    def valid_reply(reply):
        return (isinstance(reply, str) and bool(reply.strip()) and reply.isprintable()
                and len(reply.encode("utf-8")) <= 1024 and len(reply.split()) <= 60
                and not has_prompt_marker(reply))

    def receive_event(request, sequence):
        envelope = receive()
        event = envelope.get("event", {})
        pack.check(envelope.get("reply") == "event" and event.get("turn_id") == request["turn_id"]
                   and event.get("sequence") == sequence, "generator returned an out-of-sequence event")
        return event

    def read_drafts(request):
        chunks = []
        size = 0
        first_delta = None
        for sequence in range(1, 130):
            event = receive_event(request, sequence)
            if event.get("kind") != "delta":
                return event, sequence, "".join(chunks), first_delta
            if first_delta is None:
                first_delta = time.monotonic()
            text = event.get("text")
            pack.check(isinstance(text, str) and text and sequence <= 128,
                       "generator returned an invalid draft delta")
            size += len(text.encode("utf-8"))
            pack.check(size <= 4096, "generator exceeded its draft budget")
            chunks.append(text)
        raise ValueError("generator did not complete its bounded draft")

    def complete_turn(event):
        reply = event.get("reply")
        usage = event.get("context", {})
        pack.check(event.get("kind") == "complete" and event.get("outcome") == "answered"
                   and valid_reply(reply),
                   "generator did not complete the bounded turn")
        pack.check(usage.get("window_tokens") == window and usage.get("model_windows") == 1
                   and type(usage.get("peak_input_tokens")) is int and 0 < usage["peak_input_tokens"] <= window,
                   "generator did not report its bounded input window")
        return usage, reply

    def probe_operational(turn_id, question):
        nonlocal deadline
        facts = [
            {"source_id": "actor-search", "text": "Actor search is ready."},
            {"source_id": "gateway", "text": "Gateway reported “ready” for local requests."},
        ]
        request = {
            "turn_id": turn_id, "snapshot_id": turn_id, "question": question, "passages": facts,
            "dialogue": {"kind": "operational", "history": []},
        }
        turn_started = time.monotonic()
        deadline = turn_started + 25
        send({"command": "ask", "request": request})
        event, first_finding, draft, _ = read_drafts(request)
        for sequence, fact in enumerate(facts, first_finding):
            pack.check(event == {"kind": "finding", "turn_id": turn_id, "sequence": sequence,
                                 "start": 0, "end": len(fact["text"]), **fact},
                       "operational turn did not cite an exact frozen fact in order")
            event = receive_event(request, sequence + 1)
        context, reply = complete_turn(event)
        body = " ".join(fact["text"] for fact in facts)
        pack.check(reply in (body, f"Here is the current status. {body}", f"Here are the current checks. {body}"),
                   "operational reply did not preserve all approved facts")
        pack.check(draft == reply, "operational turn did not stream its verified reply before completion")
        turn_finished = time.monotonic()
        sample_rss()
        return {"context": context, "delta_events": first_finding - 1, "composed": bool(reply),
                "reply_ms": round((turn_finished - turn_started) * 1000, 1)}

    try:
        ready = {"reply": "ready_with_streaming", "protocol": 1,
                 "model_revision": pack.CHAT_MODELS[chat_model]["revision"]}
        if extended_context:
            ready.update(reply="ready_with_streaming_context", window_tokens=window)
        pack.check(receive() == ready, "chat checkpoint did not become ready with streaming")
        warm = time.monotonic()
        if extended_context:
            deadline = warm + 5
            send({"command": "ping"})
            pack.check(receive() == ready, "chat checkpoint changed its advertised input window")
        sample_rss()
        greeting = {
            "turn_id": "pack-dialogue-probe", "snapshot_id": "pack-dialogue-probe",
            "question": "Hello!", "passages": [],
            # Keep the older companion input shape to check that the worker
            # can stream a greeting even when unrelated history is supplied.
            "dialogue": {"kind": "greeting", "history": [
                {"question": "What can I call you?", "reply": "You can call me Fleetworks."},
                {"question": "Good morning.", "reply": "Good morning!"},
            ]},
        }
        greeting_started = time.monotonic()
        deadline = greeting_started + 25
        send({"command": "ask", "request": greeting})
        greeting_terminal, greeting_sequence, greeting_draft, first_delta = read_drafts(greeting)
        pack.check(first_delta is not None and greeting_draft.strip(),
                   "generator did not stream any conversational text before completion")
        pack.check(not has_prompt_marker(greeting_draft), "generator streamed a conversational prompt marker")
        greeting_context, _ = complete_turn(greeting_terminal)
        greeted = time.monotonic()
        sample_rss()

        question = "How are actor search and the gateway doing?"
        operational = probe_operational("pack-operational-probe", question)
        extended = {}
        if extended_context:
            # Each " test" is one token in the pinned GPT-2 vocabulary. The
            # question stays under the 4096-byte wire limit with room for the menu.
            long_question = "Fixture labels:" + " test" * 600 + ". " + question
            long_operational = probe_operational("pack-long-operational-probe", long_question)
            pack.check(long_operational["context"]["peak_input_tokens"] > FLAN_INPUT_WINDOW,
                       "GPT-2 did not exercise an input above the FLAN window")
            extended = {"long_operational_context": long_operational["context"],
                        "long_operational_reply_ms": long_operational["reply_ms"]}

        send({"command": "shutdown"})
        pack.check(process.wait(timeout=10) == 0, "QA checkpoint worker did not shut down")
        return {"model_warm_ms": round((warm - started) * 1000, 1),
                "first_delta_ms": round((first_delta - greeting_started) * 1000, 1),
                "first_reply_ms": round((greeted - greeting_started) * 1000, 1),
                "dialogue_delta_events": greeting_sequence - 1,
                "operational_delta_events": operational["delta_events"],
                "operational_reply_ms": operational["reply_ms"],
                "dialogue_context": greeting_context, "operational_context": operational["context"],
                "operational_composed": operational["composed"], "peak_sampled_rss_bytes": peak_rss,
                "rss_samples": rss_samples, **extended}
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


def probe(target, chat_model, runtime, manifest_sha256, assets=None):
    readonly_toolchain(target)
    pack.verify_directory(runtime, target, manifest_sha256)
    loader_key = "DYLD_LIBRARY_PATH" if target in DARWIN_WHEELS else "LD_LIBRARY_PATH"
    runtime = runtime.resolve()
    if assets is not None:
        pack.verify_model_assets(assets, chat_model)
        result = verify_inference(runtime, loader_key, target, chat_model, model_directory=assets.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix="inbox-qa-probe-") as scratch:
            model = Path(scratch).resolve() / "model"
            prepare_assets(model, chat_model)
            result = verify_inference(runtime, loader_key, target, chat_model, model_directory=model)
    return {"chat_model": chat_model, "target": target, "model_revision": pack.CHAT_MODELS[chat_model]["revision"],
            "runtime_manifest_sha256": manifest_sha256, "checkpoint_smoke": result}


def build(target, directory, chat_model=pack.DEFAULT_CHAT_MODEL):
    readonly_toolchain(target)
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    staging = directory / "staging"
    libraries = staging / "runtime-libraries"
    libraries.mkdir(parents=True)
    model = staging / "model"
    prepare_assets(model / "generator", chat_model)
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
              else linux_libraries(binary, libraries, env, torch_root=torch_root))
    notices = []
    for info in sorted(torch_root.parent.glob("torch-*.dist-info")):
        for source in sorted([*info.glob("LICENSE"), *info.glob("NOTICE*")]):
            target_path = staging / ("torch-" + source.name)
            shutil.copyfile(source, target_path)
            notices.append(target_path)
    pack.check(staging / "torch-LICENSE" in notices, "pinned CPU Torch distribution license is missing")

    archive = directory / f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz"
    digests = pack.build(argparse.Namespace(model=model, worker=binary, listener=listener,
                                           lib=native, license=notices, output=archive, target=target,
                                           chat_model=chat_model))
    installed = directory / "installed"
    pack.install(argparse.Namespace(archive=archive, output=installed, state_dir=None, target=target,
                                    chat_model=chat_model, archive_sha256=digests["archive_sha256"],
                                    manifest_sha256=digests["manifest_sha256"]))
    if target not in DARWIN_WHEELS:
        linux_libraries(installed / pack.WORKER, installed / "lib",
                        {**env, loader_key: str(installed / "lib")}, installed=True)
    digests["checkpoint_smoke"] = verify_inference(installed, loader_key, target, chat_model)
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
    command.add_argument("--chat-model", choices=pack.CHAT_MODELS, default=pack.DEFAULT_CHAT_MODEL)
    command = commands.add_parser("probe")
    command.add_argument("--target", required=True, choices=pack.TARGETS)
    command.add_argument("--chat-model", required=True, choices=pack.CHAT_MODELS)
    command.add_argument("--runtime", type=Path, required=True)
    command.add_argument("--manifest-sha256", required=True)
    command.add_argument("--assets", type=Path)
    command = commands.add_parser("prepare-assets")
    command.add_argument("--chat-model", required=True, choices=pack.CHAT_MODELS)
    command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "wheel":
        print(wheel(args.target))
    elif args.command == "build":
        build(args.target, args.output, args.chat_model)
    elif args.command == "probe":
        result = probe(args.target, args.chat_model, args.runtime, args.manifest_sha256, args.assets)
        print(json.dumps(result, sort_keys=True))
    else:
        prepare_assets(args.output, args.chat_model)


if __name__ == "__main__":
    main()
