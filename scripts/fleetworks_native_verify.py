"""Selected same-view Rune fixture evidence; no backend, install or visual verdict.

Called through fleetworks_verify.py native-ui. Python 3.9+ standard library only.
The fixture is built once in debug, then its actual inventory supplies scene names.
"""

import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
import time
import zlib

from fleetworks_candidate import NATIVE_CPU_TYPES, cargo_executable, locked_rune_revision
from fleetworks_candidate import macho_architecture, native_output, repository_identity, sha256, validate_toolchain
from fleetworks_platform import NATIVE_TARGETS, detect, resolve_native_target

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = "native_fixture"
NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,127}")
MAX_CASES = 128
MAX_PIXELS = 16 * 1024 * 1024
MAX_LOG = 2 * 1024 * 1024
MAX_TREE = 8 * 1024 * 1024
MAX_TRACE = 16 * 1024 * 1024
MAX_PNG = 64 * 1024 * 1024


def add_parser(commands):
    parser = commands.add_parser("native-ui", help="build one debug fixture and generate selected same-view evidence; no visual/GUI verdict")
    parser.add_argument("--scene", action="append", default=[], help="repeat exact runtime scene names; no implicit all-scenes run")
    parser.add_argument("--size", action="append", default=[], metavar="WIDTHxHEIGHT", help="repeat viewports in points, default 1280x800")
    parser.add_argument("--scale", action="append", default=[], help="repeat integer scales 1–3, default 2; sizes/scales form a product")
    parser.add_argument("--script", action="append", default=[], metavar="SCENE=FILE.json", help="additional scripted cases for a selected scene; repository-relative file")
    parser.add_argument("--script-expect-key", action="append", default=[], metavar="SCENE=KEY", help="require an accessibility/element key in that scene's scripted final tree")
    parser.add_argument("--at", type=float, default=0.0, help="snapshot virtual seconds for baseline cases, 0–30")
    parser.add_argument("--script-at", type=float, default=1.0, help="snapshot virtual seconds for scripted cases, 0–30")
    parser.add_argument("--list-scenes", action="store_true", help="build once and record the actual fixture inventory only")
    parser.add_argument("--target", choices=("host", *NATIVE_TARGETS), default="host")
    parser.add_argument("--agent", action="store_true", help="use agent_cargo.py with the shared target/agents debug graph")
    parser.add_argument("--output", type=Path, help="new/empty review directory; default new directory under .agent-scratch/native-ui")
    parser.add_argument("--revision", help="require the checkout HEAD to match this exact SHA")
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--dry-run", action="store_true", help="print unverified selection/build plan; generate no evidence")
    return parser


def build_command(*, agent=False, target="host", root=ROOT):
    command = [sys.executable, str(root / "scripts/agent_cargo.py")] if agent else ["cargo"]
    command += ["build", "--locked", "-p", "fleetworks-desktop", "--example", EXAMPLE,
                "--message-format=json-render-diagnostics"]
    # An implicit host build preserves the existing host target/agents/debug
    # graph. Resolve compiler-artifact paths rather than guessing its layout.
    if target != "host":
        command += ["--target", target]
    return command


def parse_profiles(sizes, scales):
    sizes = list(dict.fromkeys(sizes or ["1280x800"]))
    scales = list(dict.fromkeys(scales or ["2"]))
    profiles = []
    for size in sizes:
        match = re.fullmatch(r"([1-9][0-9]{1,3})x([1-9][0-9]{1,3})", size)
        if not match:
            raise ValueError("Native view size must be WIDTHxHEIGHT in points")
        width, height = map(int, match.groups())
        if not (64 <= width <= 4096 and 64 <= height <= 4096):
            raise ValueError("Native viewport dimensions must be 64–4096 points")
        for raw in scales:
            if str(raw) not in ("1", "2", "3"):
                raise ValueError("Native scale must be an integer 1, 2 or 3")
            scale = int(raw)
            if width * height * scale * scale > MAX_PIXELS:
                raise ValueError("Selected native pixel dimensions exceed the bounded capture budget")
            profiles.append((size, scale))
    return profiles


def bounded_bytes(path, limit):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= limit:
        raise ValueError(f"Missing, empty or oversized artifact: {path.name}")
    with path.open("rb") as source:
        value = source.read(limit + 1)
    if len(value) > limit:
        raise ValueError(f"Artifact byte bound exceeded: {path.name}")
    return value


def inventory(text):
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if not 1 <= len(lines) <= 512 or any(not NAME.fullmatch(line) for line in lines) or len(set(lines)) != len(lines):
        raise ValueError("Runtime --list-scenes must return a nonempty unique bounded scene inventory")
    return lines


def mapping(values, *, label):
    result = {}
    for item in values:
        scene, separator, value = item.partition("=")
        if not separator or not NAME.fullmatch(scene) or not value or scene in result:
            raise ValueError(f"{label} must use one unique SCENE=value entry per scene")
        result[scene] = value
    return result


def selections(args, root):
    scenes = list(dict.fromkeys(args.scene))
    if any(not NAME.fullmatch(scene) for scene in scenes):
        raise ValueError("Invalid native scene name")
    if args.list_scenes:
        if scenes or args.script or args.script_expect_key:
            raise ValueError("Choose runtime listing or selected captures")
    elif not scenes:
        raise ValueError("Select at least one --scene; zero captures cannot pass")
    for value in (args.at, args.script_at):
        if not math.isfinite(value) or not 0 <= value <= 30:
            raise ValueError("Snapshot virtual time must be finite and between 0 and 30 seconds")
    scripts = {}
    for scene, raw in mapping(args.script, label="--script").items():
        relative = Path(raw)
        path = root / relative
        if scene not in scenes or relative.is_absolute() or ".." in relative.parts or path.is_symlink():
            raise ValueError("Scripts must belong to selected scenes and be regular repository-relative files")
        path = path.resolve()
        if root.resolve() not in path.parents or path.suffix != ".json":
            raise ValueError("Script must be a JSON file inside the repository")
        value = json.loads(bounded_bytes(path, 256 * 1024))
        events = value.get("events") if isinstance(value, dict) else value
        if not isinstance(events, list) or not 1 <= len(events) <= 1000:
            raise ValueError("Fixture script must contain 1–1000 events")
        for event in events:
            at = event.get("t") if isinstance(event, dict) else None
            if isinstance(at, bool) or not isinstance(at, (float, int)) or not math.isfinite(at) or not 0 <= at <= args.script_at:
                raise ValueError("Script event time must be finite, nonnegative and no later than --script-at")
        scripts[scene] = (path, sha256(path))
    expected = mapping(args.script_expect_key, label="--script-expect-key")
    if any(scene not in scripts or len(key) > 256 or any(character.isspace() for character in key) for scene, key in expected.items()):
        raise ValueError("Script key expectations need a selected scripted scene and bounded key")
    profiles = parse_profiles(args.size, args.scale)
    cases = [{"scene": scene, "size": size, "scale": scale, "at": args.at, "script": None}
             for scene in scenes for size, scale in profiles]
    cases += [{"scene": scene, "size": size, "scale": scale, "at": args.script_at,
               "script": path, "script_sha256": digest, "expected_key": expected.get(scene)}
              for scene, (path, digest) in scripts.items() for size, scale in profiles]
    if not args.list_scenes and not 1 <= len(cases) <= MAX_CASES:
        raise ValueError(f"Select between 1 and {MAX_CASES} captures")
    return scenes, cases


def png_dimensions(path):
    size = path.stat().st_size if path.is_file() and not path.is_symlink() else 0
    if not 0 < size <= MAX_PNG:
        raise ValueError("Missing, empty or oversized PNG")
    with path.open("rb") as source:
        if source.read(8) != b"\x89PNG\r\n\x1a\n":
            raise ValueError("Invalid PNG signature")
        dimensions, has_data = None, False
        while True:
            header = source.read(8)
            if len(header) != 8:
                raise ValueError("Truncated PNG chunks")
            length, kind = struct.unpack(">I4s", header)
            if length > MAX_PNG or source.tell() + length + 4 > size:
                raise ValueError("Invalid PNG chunk length")
            if kind == b"IHDR" and length != 13:
                raise ValueError("Invalid PNG header length")
            crc = zlib.crc32(kind)
            data = b""
            remaining = length
            while remaining:
                part = source.read(min(remaining, 1024 * 1024))
                if not part:
                    raise ValueError("Truncated PNG chunk")
                crc = zlib.crc32(part, crc)
                if kind == b"IHDR":
                    data += part
                remaining -= len(part)
            checksum = source.read(4)
            if len(checksum) != 4:
                raise ValueError("Truncated PNG checksum")
            expected_crc, = struct.unpack(">I", checksum)
            if crc & 0xffffffff != expected_crc:
                raise ValueError("PNG chunk checksum mismatch")
            if dimensions is None:
                if kind != b"IHDR" or length != 13:
                    raise ValueError("PNG must begin with one valid IHDR")
                width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", data)
                if not width or not height or width * height > MAX_PIXELS or depth not in (8, 16) or color not in (0, 2, 3, 4, 6) or compression or filtering or interlace not in (0, 1):
                    raise ValueError("Invalid PNG dimensions or encoding")
                dimensions = (width, height)
            elif kind == b"IHDR":
                raise ValueError("Duplicate PNG header")
            if kind == b"IDAT" and length:
                has_data = True
            if kind == b"IEND":
                if length or not has_data or source.read(1):
                    raise ValueError("PNG has no image data or an invalid end")
                return dimensions


def tree_keys(value):
    keys, pending, count = set(), [value], 0
    while pending:
        item = pending.pop()
        count += 1
        if count > 200000:
            raise ValueError("Fixture tree node budget exceeded")
        if isinstance(item, dict):
            if isinstance(item.get("key"), str):
                keys.add(item["key"])
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
    return keys


def validate_case(output, case):
    expected = tuple(int(value) * case["scale"] for value in case["size"].split("x"))
    if png_dimensions(output / case["png"]) != expected:
        raise ValueError("PNG dimensions do not match selected viewport/scale")
    value = json.loads(bounded_bytes(output / case["tree"], MAX_TREE))
    viewport = [int(value) for value in case["size"].split("x")]
    if not isinstance(value, dict) or value.get("viewport") != viewport or value.get("scale") != case["scale"] or isinstance(value.get("scale"), bool) or not isinstance(value.get("elements"), list) or not value.get("elements") or not isinstance(value.get("accessibility"), dict):
        raise ValueError("Fixture tree is empty or has invalid viewport/scale metadata")
    keys = tree_keys(value)
    if "fixture-marker" not in keys or (case.get("expected_key") and case["expected_key"] not in keys):
        raise ValueError("Fixture tree is missing the production-view marker or expected script key")
    if case.get("trace"):
        lines = bounded_bytes(output / case["trace"], MAX_TRACE).splitlines()
        if not 2 <= len(lines) <= 50000:
            raise ValueError("Script trace is empty or exceeds its event budget")
        records = [json.loads(line) for line in lines]
        header = records[0]
        if not isinstance(header, dict) or header.get("type") != "header" or header.get("clock") != "virtual" or [header.get("width"), header.get("height")] != viewport or header.get("scale") != case["scale"]:
            raise ValueError("Script trace does not match the virtual capture profile")
    return {name: {"file": case[name], "sha256": sha256(output / case[name]), "bytes": (output / case[name]).stat().st_size}
            for name in ("png", "tree", "trace") if case.get(name)}


def run_logged(command, output, label, records, *, environment, runner, timeout, stdout_limit=MAX_LOG, root=ROOT):
    stdout, stderr = output / (label + ".stdout.log"), output / (label + ".stderr.log")
    record = {"argv": list(map(str, command)), "stdout": stdout.name, "stderr": stderr.name, "exit": None}
    records.append(record)
    started = time.monotonic()
    try:
        with stdout.open("xb") as out, stderr.open("xb") as err:
            result = runner(command, cwd=root, env=environment, stdout=out, stderr=err, check=False, timeout=timeout)
        record["exit"] = result.returncode
    except subprocess.TimeoutExpired as error:
        record["timeout"] = True
        raise ValueError(f"Native command timed out: {label}") from error
    finally:
        record["seconds"] = round(time.monotonic() - started, 3)
        for path, limit in ((stdout, stdout_limit), (stderr, MAX_LOG)):
            if path.exists() and path.stat().st_size > limit:
                original = path.stat().st_size
                with path.open("r+b") as stream:
                    stream.truncate(limit)
                record["truncated"] = {"file": path.name, "original_bytes": original, "retained_bytes": limit}
        record["logs"] = [{"file": path.name, "bytes": path.stat().st_size, "sha256": sha256(path)}
                          for path in (stdout, stderr) if path.is_file()]
    if record.get("truncated"):
        raise ValueError(f"Native command output exceeded its retained byte bound: {label}")
    if record["exit"] != 0:
        raise ValueError(f"Native command failed: {label}; see bounded logs")
    return stdout


def execute(args, *, root=ROOT, runner=None, info=None):
    runner = runner or subprocess.run
    root = root.resolve()
    info = info or detect()
    target = resolve_native_target(info, args.target)
    if target != info.native_target:
        raise ValueError("Fixture execution requires the physical Mac target; cross-target execution is not platform evidence")
    toolchain = validate_toolchain(info, root)
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, "", target):
        raise ValueError("CARGO_BUILD_TARGET differs from the physical fixture target")
    scenes, selected = selections(args, root)
    command = build_command(agent=args.agent, target=args.target, root=root)
    if args.dry_run:
        print(json.dumps({"build_command": command, "target": target, "profile": "debug", "requested_scenes": scenes,
                          "capture_count": len(selected), "runtime_inventory": "not_run", "generated": False,
                          "visual_inspection": "not_performed", "real_gui": "not_run"}, indent=2))
        return 0
    revision, dirty = repository_identity(root=root, revision=args.revision, runner=runner)
    if (args.require_clean or os.environ.get("CI") == "true") and dirty:
        raise ValueError("Clean exact-revision native evidence requires a clean checkout")
    destination = native_output(args.output or root / ".agent-scratch/native-ui", target, "debug", root=root)
    if args.output:
        if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
            raise ValueError("Native evidence output must be new or empty; stale captures cannot pass")
        destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    else:
        destination.mkdir(mode=0o700, parents=True, exist_ok=True)
        destination = Path(tempfile.mkdtemp(prefix="selected-", dir=destination))
    manifest = {"format": "fleetworks-native-fixtures-1", "source_revision": revision, "dirty": dirty,
                "rune_revision": locked_rune_revision(root), "toolchain": toolchain, "target": target,
                "profile": "debug", "cargo_target_mode": ("environment_target" if os.environ.get("CARGO_BUILD_TARGET") else "implicit_host") if args.target == "host" else "explicit_target",
                "cargo_build_target_env": os.environ.get("CARGO_BUILD_TARGET") or None,
                "platform": info.as_dict(), "cargo_lock_sha256": sha256(root / "Cargo.lock"),
                "renderer": "Rune headless; actual adapter diagnostics retained in per-command logs",
                "fonts": "repository-pinned Rune embedded-fonts feature", "virtual_clock": True,
                "state": "started", "commands": [], "captures": [], "visual_inspection": "not_performed",
                "real_gui": "not_run", "unit_tests": "not_run_by_this_command"}
    try:
        environment = dict(os.environ)
        environment.pop("ARGOS_EMBED", None)
        environment["RUSTUP_AUTO_INSTALL"] = "0"
        cargo_output = run_logged(command, destination, "cargo-fixture", manifest["commands"], environment=environment, runner=runner, timeout=3600, stdout_limit=64 * 1024 * 1024, root=root)
        binary = cargo_executable(bounded_bytes(cargo_output, 64 * 1024 * 1024).decode("utf-8"), name=EXAMPLE, kind="example")
        if macho_architecture(binary) != NATIVE_CPU_TYPES[target]:
            raise ValueError("Fixture compiler artifact architecture differs from the physical Mac")
        manifest["fixture"] = {"path": str(binary), "sha256": sha256(binary)}
        with tempfile.TemporaryDirectory(prefix="fixture-home-", dir=destination) as home:
            environment = {key: value for key, value in os.environ.items() if key in ("PATH", "TMPDIR", "LANG", "LC_ALL", "RUST_BACKTRACE", "DYLD_FRAMEWORK_PATH", "DYLD_LIBRARY_PATH")}
            environment.update({"HOME": home, "ARGOS_HOME": home, "FLEETWORKS_WORKSPACE": home, "XDG_CACHE_HOME": home, "RUSTUP_AUTO_INSTALL": "0"})
            listing = run_logged([str(binary), "--list-scenes"], destination, "runtime-scenes", manifest["commands"], environment=environment, runner=runner, timeout=30, stdout_limit=64 * 1024, root=root)
            available = inventory(bounded_bytes(listing, 64 * 1024).decode("utf-8"))
            manifest["runtime_scenes"] = available
            if not set(scenes).issubset(available):
                raise ValueError("Selected scene is absent from the actual runtime --list-scenes inventory")
            for selection in selected:
                case = {**selection, "script": str(selection["script"].relative_to(root)) if selection["script"] else None,
                        "generated": False, "visual_inspection": "not_performed", "real_gui": "not_run"}
                label = f"{case['scene']}-{case['size']}-{case['scale']}x" + ("-script-" + case["script_sha256"][:12] if case["script"] else "")
                case.update(png=label + ".png", tree=label + ".tree.json")
                argv = [str(binary), "--scene", case["scene"], "--size", case["size"], "--scale", str(case["scale"]), "--at", str(case["at"]), "--snap", str(destination / case["png"]), "--tree", str(destination / case["tree"])]
                if selection["script"]:
                    case["trace"] = label + ".trace.jsonl"
                    argv += ["--script", str(selection["script"]), "--trace", str(destination / case["trace"])]
                manifest["captures"].append(case)
                run_logged(argv, destination, label, manifest["commands"], environment=environment, runner=runner, timeout=90, root=root)
                case["artifacts"] = validate_case(destination, case)
                case["generated"] = True
            if not args.list_scenes and (not manifest["captures"] or not all(case["generated"] for case in manifest["captures"])):
                raise ValueError("Zero or incomplete captures cannot pass")
        if repository_identity(root=root, runner=runner) != (revision, dirty) or sha256(binary) != manifest["fixture"]["sha256"]:
            raise ValueError("Source identity or fixture executable changed during generation")
        for selection in selected:
            if selection["script"] and sha256(selection["script"]) != selection["script_sha256"]:
                raise ValueError("Fixture script changed during generation")
        manifest["state"] = "inventory_recorded" if args.list_scenes else "generated"
        print(f"Native {manifest['state']}: {destination}; visual inspection and real GUI evidence remain separate")
        return 0
    except (OSError, ValueError, RecursionError, subprocess.SubprocessError) as error:
        manifest["state"] = "failed"
        manifest["error"] = str(error)[:1000]
        # Keep failed evidence bounded too. A truncated artifact is explicitly
        # diagnostic-only and can never become a successful generated capture.
        for case in manifest["captures"]:
            for name, limit in (("png", MAX_PNG), ("tree", MAX_TREE), ("trace", MAX_TRACE)):
                path = destination / case.get(name, "__absent__")
                if not path.is_symlink() and path.is_file() and path.stat().st_size > limit:
                    original = path.stat().st_size
                    with path.open("r+b") as stream:
                        stream.truncate(limit)
                    case.setdefault("truncated_diagnostics", []).append({"file": path.name, "original_bytes": original, "retained_bytes": limit})
        raise
    finally:
        encoded = json.dumps(manifest, indent=2, allow_nan=False) + "\n"
        if len(encoded.encode()) > 2 * 1024 * 1024:
            raise ValueError("Native evidence manifest byte budget exceeded")
        (destination / "manifest.json").write_text(encoded, encoding="utf-8")
