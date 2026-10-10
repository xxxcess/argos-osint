import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const script = join(dirname(fileURLToPath(import.meta.url)), "../../scripts/tui_review.py");
test("cell verification rejects absent and truncated screenshot evidence", () => {
  const cells = mkdtempSync(join(tmpdir(), "argos-tui-cells-"));
  const check = () => spawnSync("python3", [script, "validate", cells], { encoding: "utf8" });
  try {
    assert.equal(check().status, 1, "empty galleries must not pass");
    const cell = { s: "▌", fg: "Rgb(151, 193, 125)", bg: "Reset", b: false, u: true };
    writeFileSync(join(cells, "fixture.json"), JSON.stringify({ width: 2, height: 1, cells: [[cell]] }));
    const truncated = check();
    assert.equal(truncated.status, 1);
    assert.match(truncated.stderr, /dimensions do not match/);
    writeFileSync(join(cells, "fixture.json"), JSON.stringify({ width: 1, height: 1, cells: [[cell]] }));
    assert.equal(check().status, 0, "valid actual-cell schema remains supported");
  } finally {
    rmSync(cells, { recursive: true, force: true });
  }
});
