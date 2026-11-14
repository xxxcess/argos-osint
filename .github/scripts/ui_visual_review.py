#!/usr/bin/env python3
"""Validate a dispatched UI review and prepare bounded PNG attachments as data."""

import argparse
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import zlib
import zipfile
from pathlib import Path

HEAD_RE = re.compile(r"[0-9a-f]{40}\Z")
SCENE_RE = re.compile(r"[a-z][a-z0-9-]{0,63}\Z")
TAG_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")
WEB_SLUG = {
    "bulletin": "bulletin-quiet-action",
    "brief": "focus-brief-primary-related",
    "topology": "topology-node-list",
    "actors": "narrow-tabs-default-button",
}
PNG_LIMIT = 32 * 1024 * 1024


def fail(message):
    raise ValueError(message)


def scenes(value):
    selected = [item.strip() for item in value.split(",")]
    if not 1 <= len(selected) <= 6 or any(not SCENE_RE.fullmatch(item) for item in selected):
        fail("Select 1–6 comma-separated Rune scene names")
    if len(set(selected)) != len(selected):
        fail("Rune scene names must be unique")
    return selected


def api_json(*parts):
    output = subprocess.check_output(["gh", "api", *parts], text=True)
    return json.loads(output)


def validate(args):
    if args.repo != os.environ.get("GITHUB_REPOSITORY"):
        fail("Review repository differs from the workflow repository")
    if not HEAD_RE.fullmatch(args.head) or not args.pr.isdecimal() or int(args.pr) < 1:
        fail("An open PR number and exact 40-character head SHA are required")
    if args.scope not in {"native", "web", "both"}:
        fail("Choose native, web or both")
    selected = scenes(args.native_scenes)
    if args.native_script == "settings" and "registration-ready" not in selected:
        fail("The Settings script requires the registration-ready scene")
    if args.native_script not in {"none", "settings"}:
        fail("Unsupported Rune input script")
    if args.web_surface not in {*WEB_SLUG, "all"}:
        fail("Unsupported web surface")
    if args.release_tag and not TAG_RE.fullmatch(args.release_tag):
        fail("Release tag contains unsupported characters")

    pr = api_json(f"repos/{args.repo}/pulls/{args.pr}")
    if (pr["state"] != "open" or pr["head"]["sha"] != args.head
            or pr["head"]["repo"]["full_name"] != args.repo
            or pr["base"]["repo"]["full_name"] != args.repo
            or pr["base"]["ref"] not in {"dev", "nightly", "main"}):
        fail("Review requires an open same-repository PR at the exact selected head")
    changed = subprocess.check_output(
        ["gh", "api", "--paginate", f"repos/{args.repo}/pulls/{args.pr}/files", "--jq", ".[].filename"],
        text=True,
    ).splitlines()
    native = any(path.startswith("apps/fleetworks-desktop/") for path in changed)
    web = any(path.startswith(("apps/web-desktop/", "packages/ui/", "packages/intel/",
                               "packages/api-client/"))
              or path.startswith("tests/web/") for path in changed)
    if args.scope in {"native", "both"} and not native:
        fail("Requested native previews require native UI changes in this PR")
    if args.scope in {"web", "both"} and not web:
        fail("Requested web previews require web UI changes in this PR")
    if args.release_tag:
        api_json(f"repos/{args.repo}/releases/tags/{args.release_tag}")
        tagged = api_json(f"repos/{args.repo}/commits/{args.release_tag}")
        if tagged["sha"] != args.head:
            fail("The existing Release tag must resolve to the exact reviewed head")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            print("head=" + args.head, file=output)
            print("pr=" + args.pr, file=output)


def capture_native(args):
    selected = scenes(args.native_scenes)
    command = [
        sys.executable, "scripts/fleetworks_verify.py", "native-ui", "--agent", "--require-clean",
        "--revision", args.head, "--output", args.output, "--size", "1440x860", "--size", "640x480",
        "--scale", "1",
    ]
    for scene in selected:
        command += ["--scene", scene]
    if args.native_script == "settings":
        command += [
            "--script", "registration-ready=apps/fleetworks-desktop/tests/fixtures/scripts/workstation-settings-open.json",
            "--script-expect-key", "registration-ready=workstation-settings-body",
        ]
    subprocess.run(command, check=True)


def png_dimensions(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size < 45 or info.st_size > PNG_LIMIT:
        fail(f"Invalid PNG file type or bound: {path.name}")
    data = path.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        fail(f"PNG signature missing: {path.name}")
    offset = 8
    width = height = None
    ended = False
    while offset + 12 <= len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        end = offset + 12 + length
        if end > len(data):
            fail(f"Truncated PNG chunk: {path.name}")
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        expected = struct.unpack_from(">I", data, offset + 8 + length)[0]
        if zlib.crc32(kind + payload) & 0xFFFFFFFF != expected:
            fail(f"PNG CRC mismatch: {path.name}")
        if offset == 8:
            if kind != b"IHDR" or length != 13:
                fail(f"PNG IHDR missing: {path.name}")
            width, height = struct.unpack_from(">II", payload)
            if not 64 <= width <= 8192 or not 64 <= height <= 8192:
                fail(f"PNG dimensions outside review bound: {path.name}")
        if kind == b"IEND":
            ended = length == 0 and end == len(data)
            break
        offset = end
    if not ended:
        fail(f"PNG IEND missing: {path.name}")
    return width, height


def safe_file(root, relative):
    path = root / relative
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        fail("Artifact path escapes the selected download")
    return path


def selected_native(root, head, selected, script):
    fixture = root / f"Fleetworks-ui-native-{head}" / "fixtures"
    manifest_path = safe_file(fixture, Path("manifest.json"))
    if manifest_path.stat().st_size > 2 * 1024 * 1024:
        fail("Rune manifest exceeds review bound")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if (manifest.get("source_revision") != head or manifest.get("dirty") is not False
            or manifest.get("state") != "generated"):
        fail("Rune fixture manifest does not describe the exact clean source")
    captures = manifest.get("captures")
    if not isinstance(captures, list) or not captures:
        fail("Rune fixture manifest has no captures")
    all_files = [manifest_path]
    for case in captures:
        if case.get("generated") is not True or not isinstance(case.get("png"), str):
            fail("Rune capture is incomplete")
        name = case["png"]
        if not re.fullmatch(r"[a-z0-9-]+\.png", name):
            fail("Rune capture has an invalid PNG name")
        path = safe_file(fixture, Path(name))
        png_dimensions(path)
        all_files.append(path)
    chosen = []
    for scene in selected:
        matching = [case for case in captures if case.get("scene") == scene
                    and case.get("size") == "1440x860" and case.get("scale") == 1
                    and case.get("script") is None]
        if len(matching) != 1:
            fail(f"Expected one wide baseline for {scene}")
        chosen.append((safe_file(fixture, Path(matching[0]["png"])), f"Native {scene} at 1440×860"))
    if script == "settings":
        matching = [case for case in captures if case.get("scene") == "registration-ready"
                    and case.get("size") == "1440x860" and case.get("scale") == 1
                    and case.get("script") is not None]
        if len(matching) != 1:
            fail("Expected one wide Settings dialog capture")
        chosen.append((safe_file(fixture, Path(matching[0]["png"])), "Native Workstation Settings dialog"))
    return chosen, all_files


def selected_web(root, head, surface):
    screenshots = root / f"Fleetworks-ui-web-{head}" / "screenshots"
    names = WEB_SLUG.values() if surface == "all" else [WEB_SLUG[surface]]
    chosen = []
    all_files = []
    for slug in names:
        for viewport in ("1440x1000", "768x1000", "390x844", "320x800"):
            path = safe_file(screenshots, Path(f"{viewport}-{slug}.png"))
            png_dimensions(path)
            all_files.append(path)
            if viewport in {"1440x1000", "390x844"}:
                chosen.append((path, f"Web {slug.replace('-', ' ')} at {viewport}"))
    return chosen, all_files


def prepare(args):
    if not HEAD_RE.fullmatch(args.head):
        fail("Exact reviewed head is required")
    root = Path(args.artifact_root).resolve(strict=True)
    selected = []
    all_files = []
    if args.scope in {"native", "both"}:
        images, files = selected_native(root, args.head, scenes(args.native_scenes), args.native_script)
        selected += images
        all_files += files
    if args.scope in {"web", "both"}:
        images, files = selected_web(root, args.head, args.web_surface)
        selected += images
        all_files += files
    if not selected or len(selected) > 16:
        fail("Review requires 1–16 selected images")
    body = [
        f"<!-- fleetworks-ui-review:{args.head}:{args.scope} -->",
        f"### UI visual review · `{args.head[:12]}`",
        "",
        f"Generated from [GitHub Actions run]({args.run_url}) for the selected `{args.scope}` UI scope.",
        ("Rune scenes: " + ", ".join(scenes(args.native_scenes)) + ".") if args.scope in {"native", "both"}
        else ("Web surface: " + args.web_surface + "."),
        "These are fixture screenshots; human visual inspection and installed-app behavior remain separate checks.",
        "",
    ]
    if args.scope == "both":
        body.insert(5, "Web surface: " + args.web_surface + ".")
    args.body.parent.mkdir(parents=True, exist_ok=True)
    image_directory = args.body.parent / "images"
    image_directory.mkdir(exist_ok=True)
    attachments = []
    for index, (path, alt) in enumerate(selected, 1):
        copied = image_directory / f"{index:02d}-{path.name}"
        shutil.copyfile(path, copied)
        attachments.append(str(copied))
        body += [f"**{alt}**", "", f"![{alt}](images/{copied.name})", ""]
    args.body.write_text("\n".join(body), encoding="utf-8")
    args.attachments.write_text(json.dumps(attachments), encoding="utf-8")
    with zipfile.ZipFile(args.archive, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("source.json", json.dumps({"head": args.head, "scope": args.scope, "run": args.run_url}) + "\n")
        for path in sorted(set(all_files)):
            archive.write(path, path.relative_to(root))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("validate")
    check.add_argument("--repo", required=True)
    check.add_argument("--pr", required=True)
    check.add_argument("--head", required=True)
    check.add_argument("--scope", required=True)
    check.add_argument("--native-scenes", required=True)
    check.add_argument("--native-script", required=True)
    check.add_argument("--web-surface", required=True)
    check.add_argument("--release-tag", default="")
    capture = commands.add_parser("capture-native")
    capture.add_argument("--head", required=True)
    capture.add_argument("--native-scenes", required=True)
    capture.add_argument("--native-script", required=True)
    capture.add_argument("--output", required=True)
    build = commands.add_parser("prepare")
    build.add_argument("--artifact-root", type=Path, required=True)
    build.add_argument("--head", required=True)
    build.add_argument("--scope", choices=("native", "web", "both"), required=True)
    build.add_argument("--native-scenes", required=True)
    build.add_argument("--native-script", choices=("none", "settings"), required=True)
    build.add_argument("--web-surface", choices=(*WEB_SLUG, "all"), required=True)
    build.add_argument("--run-url", required=True)
    build.add_argument("--body", type=Path, required=True)
    build.add_argument("--attachments", type=Path, required=True)
    build.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    {"validate": validate, "capture-native": capture_native, "prepare": prepare}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"UI visual review failed: {error}", file=sys.stderr)
        sys.exit(1)
