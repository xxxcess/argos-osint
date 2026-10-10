#!/usr/bin/env python3
"""Run agent Cargo commands in a cache separate from the user's target/debug."""
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
TARGET_DIR = ROOT / "target" / "agents"


def cargo_command(args):
    if not args or args[0] not in ("build", "check", "clippy", "test", "run"):
        raise ValueError("expected build, check, clippy, test or run followed by Cargo arguments")
    cargo_args = args[:args.index("--")] if "--" in args else args
    if any(arg == "--target-dir" or arg.startswith("--target-dir=") for arg in cargo_args):
        raise ValueError("agent target directory is fixed; omit --target-dir")
    return ["cargo", args[0], "--target-dir", str(TARGET_DIR), *args[1:]]


def cargo_env():
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TARGET_DIR)
    return env


def main():
    if sys.argv[1:] in ([], ["--help"], ["-h"]):
        print(__doc__)
        print("Usage: python3 scripts/agent_cargo.py <build|check|clippy|test|run> [Cargo arguments]")
        print(f"Agent artifacts: {TARGET_DIR}")
        return
    try:
        command = cargo_command(sys.argv[1:])
        os.chdir(ROOT)
        os.execvpe(command[0], command, cargo_env())
    except (ValueError, OSError) as error:
        sys.exit(f"agent-cargo: {error}")


if __name__ == "__main__":
    main()
