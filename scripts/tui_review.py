#!/usr/bin/env python3
"""Capture real TUI fixtures into a fresh, traceable review directory."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

sys.dont_write_bytecode = True
from agent_cargo import TARGET_DIR, cargo_command, cargo_env

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(cells):
    paths = sorted(cells.glob("*.json"))
    if not paths:
        raise ValueError(f"No cell dumps in {cells}; the chosen test must emit ARGOS_SCREEN_DIR fixtures")
    records = []
    for path in paths:
        data = json.loads(path.read_text())
        width, height = data["width"], data["height"]
        if type(width) is not int or type(height) is not int or width < 1 or height < 1:
            raise ValueError(f"{path}: invalid dimensions")
        if len(data["cells"]) != height or any(len(row) != width for row in data["cells"]):
            raise ValueError(f"{path}: cell dimensions do not match {width}x{height}")
        for row in data["cells"]:
            for cell in row:
                if not all(isinstance(cell.get(k), str) for k in ("s", "fg", "bg")):
                    raise ValueError(f"{path}: invalid symbol/color fields")
                if not all(type(cell.get(k)) is bool for k in ("b", "u")):
                    raise ValueError(f"{path}: invalid modifier fields")
        records.append({"name": path.stem, "width": width, "height": height,
                        "cells_sha256": digest(path)})
    return records


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def capture(args):
    # Preflight before running Cargo or creating an artifact directory.
    import PIL  # noqa: F401
    from render_tui_cells import FONT
    if FONT is None:
        raise ValueError("No supported monospace font. Install DejaVu Sans Mono or Courier New.")
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".planning"):
        raise ValueError("--output must be a new directory under this repo's .planning/")
    output.mkdir(parents=True, exist_ok=False)
    cells, screenshots = output / "cells", output / "screenshots"
    cells.mkdir()
    screenshots.mkdir()
    command = cargo_command(["test", "-p", "argos-osint-bin", "--locked",
                             "--no-default-features", args.test])
    if args.ignored:
        command.extend(["--", "--ignored"])
    manifest = {"created_utc": datetime.now(timezone.utc).isoformat(),
                "capture_status": "incomplete",
                "git_head": git("rev-parse", "HEAD"),
                "tracked_diff_sha256": hashlib.sha256(subprocess.check_output(
                    ["git", "diff", "HEAD", "--binary"], cwd=ROOT)).hexdigest(),
                "git_status": git("status", "--short"), "command": command,
                "ARGOS_EMBED": "unset", "CARGO_TARGET_DIR": str(TARGET_DIR), "font": FONT,
                "pillow_version": PIL.__version__, "python_version": sys.version,
                "renderer_sha256": digest(ROOT / "scripts/render_tui_cells.py"),
                "visual_review": "pending", "interaction_review": "not certified by capture",
                "reference_comparison": "pending", "live_providers": "not exercised"}
    # Include new, untracked implementation files that a git diff alone omits.
    manifest["source_sha256"] = {
        str(path.relative_to(ROOT)): digest(path)
        for folder in ("crates", "scripts", ".opencode")
        for path in sorted((ROOT / folder).rglob("*"))
        if path.is_file() and path.suffix in (".rs", ".py", ".json", ".md", ".mjs", ".toml")
        and not any(part in ("__pycache__", "node_modules") for part in path.parts)
    }
    scratch = ROOT / ".agent-scratch"
    scratch.mkdir(exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", prefix="tui-review-", suffix=".log",
                                     dir=scratch, delete=False) as log:
        log_path = Path(log.name)
        with tempfile.TemporaryDirectory(prefix="argos-tui-review-") as home:
            env = cargo_env()
            env.pop("ARGOS_EMBED", None)
            env.update(ARGOS_HOME=home, ARGOS_SCREEN_DIR=str(cells))
            process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, text=True)
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
            manifest["test_exit_code"] = process.wait()
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if manifest["test_exit_code"]:
        raise ValueError(f"Cargo failed; incomplete artifacts: {output}; log: {log_path}")
    try:
        records = validate(cells)
        subprocess.run([sys.executable, str(ROOT / "scripts/render_tui_cells.py"),
                        str(cells), str(screenshots)], cwd=ROOT, check=True)
        for record in records:
            record["png_sha256"] = digest(screenshots / (record["name"] + ".png"))
        manifest["fixtures"] = records
        manifest["artifact_count"] = len(records)
        manifest["capture_status"] = "complete"
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        lines = ["# TUI fixture review", "", "Actual TestBackend cells from deterministic fixtures. Review is pending.",
                 "", "See `manifest.json` for command, revision, environment and artifact hashes.", "",
                 "| Fixture | Cells | Screenshot |", "| --- | --- | --- |"]
        for record in records:
            name = record["name"]
            lines.append(f"| {name} | {record['width']}×{record['height']} | [{name}](screenshots/{name}.png) |")
        lines.extend(["", "## Review ledger", "", "Record reviewed files, expectations, observed defects, fixes and reruns here.",
                      "Record reference-image availability, interaction test names/results and live-check limitations separately.", ""])
        (output / "README.md").write_text("\n".join(lines))
    except Exception:
        print(f"Incomplete artifact run; diagnostic log retained at {log_path}", file=sys.stderr)
        raise
    log_path.unlink()
    print(f"Captured {len(records)} fixtures. Open {output / 'README.md'} and perform visual review.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("capture", help="run offline fixtures, validate and render a fresh gallery")
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--test", default="analytics_viewports_and_data_states",
                     help="Cargo test filter; test must emit ARGOS_SCREEN_DIR dumps")
    run.add_argument("--ignored", action="store_true", help="run an explicitly ignored fixture test")
    check = sub.add_parser("validate", help="validate existing cell dumps without Pillow/Cargo")
    check.add_argument("cells", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "capture":
            capture(args)
        else:
            print(f"Validated {len(validate(args.cells))} cell dumps.")
    except (ValueError, KeyError, TypeError, OSError, ImportError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"tui-review: {error}\n")


if __name__ == "__main__":
    main()
