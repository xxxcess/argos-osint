import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const script = join(root, "scripts/agent_cargo.py");

test("agent Cargo isolates inherited targets, forwards arguments and preserves failures", () => {
  const bin = mkdtempSync(join(tmpdir(), "argos-agent-cargo-"));
  try {
    const cargo = join(bin, "cargo");
    writeFileSync(cargo, '#!/bin/sh\nprintf "%s\\n" "$CARGO_TARGET_DIR" "$@"\nexit 17\n');
    chmodSync(cargo, 0o755);
    const run = (args) => spawnSync("python3", [script, ...args], {
      encoding: "utf8", cwd: tmpdir(),
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, CARGO_TARGET_DIR: join(root, "target") },
    });
    const result = run(["test", "--locked", "--no-default-features", "--", "--ignored"]);
    assert.equal(result.status, 17);
    assert.deepEqual(result.stdout.trim().split("\n"), [
      join(root, "target/agents"), "test", "--target-dir", join(root, "target/agents"),
      "--locked", "--no-default-features", "--", "--ignored",
    ]);
    for (const args of [["check", "--target-dir", "target"], ["check", "--target-dir=target"], ["clean"]]) {
      const rejected = run(args);
      assert.equal(rejected.status, 1);
      assert.equal(rejected.stdout, "", "rejected commands must never invoke Cargo");
    }
  } finally {
    rmSync(bin, { recursive: true, force: true });
  }
});

test("fixture capture uses the shared isolated Cargo policy", () => {
  const result = spawnSync("python3", ["-B", "-c", `
import sys
sys.path.insert(0, ${JSON.stringify(join(root, "scripts"))})
import tui_review
assert tui_review.cargo_env()["CARGO_TARGET_DIR"] == str(tui_review.TARGET_DIR)
command = tui_review.cargo_command(["test", "--locked", "--no-default-features"])
assert command[2:4] == ["--target-dir", str(tui_review.TARGET_DIR)]
`], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
});
