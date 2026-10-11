import test from "node:test";
import assert from "node:assert/strict";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { ROOT } from "../../scripts/release_check.mjs";

test("Git retains shared planning and curated deliverables while excluding session evidence", t => {
  const root = mkdtempSync(join(tmpdir(), "argos-planning-policy-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  copyFileSync(join(ROOT, ".gitignore"), join(root, ".gitignore"));
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  const ignored = path => {
    const result = spawnSync("git", ["check-ignore", "--no-index", "--quiet", path], { cwd: root });
    assert.ok([0, 1].includes(result.status), `git check-ignore failed for ${path}`);
    return result.status === 0;
  };
  for (const path of ["docs/architecture.md", "docs/requirements.md", "docs/planning-artifacts.md",
    ...["design.md", "decisions.md", "acceptance.md", "handoff.md"].map(name => `.planning/2026-10-11-task/${name}`)]) {
    assert.equal(ignored(path), false, `${path} should be eligible for scoped staging`);
  }
  for (const path of [".planning/.active_plan", "task_plan.md", ".planning/phases/01-release/progress.md",
    ...["task_plan.md", "findings.md", "progress.md", "validation.md", "release-simulation.json",
      "worker.md", "tui/run-01/manifest.json", "tui/run-01/screen.png", "logs/run.txt",
      "nested/acceptance.md"].map(name => `.planning/2026-10-11-task/${name}`)]) {
    assert.equal(ignored(path), true, `${path} should stay local`);
  }
});
