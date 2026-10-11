import test from "node:test";
import assert from "node:assert/strict";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { checkMessage, checkRange, checkVersions, ROOT } from "../../scripts/release_check.mjs";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "argos-release-policy-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const path of ["Cargo.toml", "Cargo.lock", "version.txt", ".release-please-manifest.json", "release-please-config.json",
    "crates/argos-osint-core/Cargo.toml", "crates/argos-osint-bin/Cargo.toml"]) {
    const dest = join(root, path);
    mkdirSync(join(dest, ".."), { recursive: true });
    cpSync(join(ROOT, path), dest);
  }
  return root;
}

test("commit impact distinguishes features, fixes, efficiency and maintenance", () => {
  for (const [type, impact] of [["feat", "minor"], ["fix", "patch"], ["perf", "patch"], ["docs", "none"], ["chore", "none"], ["refactor", "none"], ["test", "none"], ["ci", "none"], ["build", "none"], ["style", "none"], ["revert", "none"]]) {
    assert.equal(checkMessage(`${type}(recon,intel): describe the result`).impact, impact);
  }
});

test("breaking changes are minor in 0.x and major in stable releases", () => {
  const message = "feat(cli)!: replace export schema\n\nBREAKING CHANGE: Consumers must migrate their parser.";
  assert.equal(checkMessage(message, "0.1.0").impact, "minor");
  assert.equal(checkMessage(message, "1.4.2").impact, "major");
  assert.equal(checkMessage("chore: remove legacy API\n\nBREAKING-CHANGE: Migrate callers.", "1.0.0").impact, "major");
});

test("policy rejects typo types, missing descriptions and unreviewable breaking subjects", () => {
  for (const message of ["WIP", "feature: new app", "Fix(cli): repair", "fix(): repair", "fix: ", "fix(cli)!: remove flag", "fix: repair\n\nBREAKING CHANGE:", "fix: repair\n\nRelease-As: 01.2.3"]) {
    assert.throws(() => checkMessage(message), undefined, message);
  }
  assert.equal(checkMessage("feat(release): establish stable contracts\n\nRelease-As: 1.0.0").releaseAs, "1.0.0");
});

test("workspace, local lockfile, mirrors and prospective tag agree", t => {
  const root = fixture(t);
  const version = checkVersions(root);
  assert.equal(checkVersions(root, `v${version}`), version);
  assert.throws(() => checkVersions(root, "v99.0.0"), /Expected tag/);
});

test("drift in each release-managed version is rejected", t => {
  for (const path of ["version.txt", ".release-please-manifest.json", "Cargo.lock"]) {
    const root = fixture(t);
    const file = join(root, path);
    const content = readFileSync(file, "utf8");
    const changed = path === "Cargo.lock"
      ? content.replace(/(name = "argos-osint-core"\nversion = ")[^"]+/, "$199.0.0")
      : content.replace(checkVersions(root), "99.0.0");
    assert.notEqual(changed, content);
    writeFileSync(file, changed);
    assert.throws(() => checkVersions(root), /differs/);
  }
});

test("crate version overrides and broad dependency updaters are rejected", t => {
  const root = fixture(t);
  const member = join(root, "crates/argos-osint-bin/Cargo.toml");
  const original = readFileSync(member, "utf8");
  writeFileSync(member, original.replace("version.workspace = true", 'version = "0.1.0"'));
  assert.throws(() => checkVersions(root), /must inherit/);
  writeFileSync(member, original);
  const configPath = join(root, "release-please-config.json");
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  config.packages["."]["extra-files"].find(file => file.path === "Cargo.lock").jsonpath = "$.package[*].version";
  writeFileSync(configPath, JSON.stringify(config));
  assert.throws(() => checkVersions(root), /exactly the workspace package names/);
});

test("release configuration preserves 0.x behavior and maintenance-only releases stay disabled", t => {
  for (const mutate of [
    pkg => { pkg["bump-patch-for-minor-pre-major"] = true; },
    pkg => { pkg["include-component-in-tag"] = true; },
    pkg => { pkg["changelog-sections"].find(section => section.type === "chore").hidden = false; },
  ]) {
    const root = fixture(t);
    const file = join(root, "release-please-config.json");
    const config = JSON.parse(readFileSync(file, "utf8"));
    mutate(config.packages["."]);
    writeFileSync(file, JSON.stringify(config));
    assert.throws(() => checkVersions(root));
  }
});

test("main commit range validates each commit and rejects missing history", t => {
  const root = mkdtempSync(join(tmpdir(), "argos-commit-range-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  git("init", "--quiet");
  const commit = message => git("-c", "user.name=Release policy test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", message);
  commit("Legacy baseline outside the new convention");
  const base = git("rev-parse", "HEAD");
  commit("feat(cli): add export");
  commit("fix(cli): repair output");
  assert.equal(checkRange(base, git("rev-parse", "HEAD"), root), 2);
  commit("oops malformed message");
  assert.throws(() => checkRange(base, git("rev-parse", "HEAD"), root), /Use <type>/);
  assert.throws(() => checkRange("0".repeat(40), git("rev-parse", "HEAD"), root), /Missing base/);
});

test("PR title/body are checked as event data and title-only breaking changes fail", t => {
  const root = fixture(t);
  const path = join(root, "event.json");
  const run = pr => {
    writeFileSync(path, JSON.stringify({ pull_request: pr }));
    return spawnSync(process.execPath, [join(ROOT, "scripts/release_check.mjs"), "event"], {
      encoding: "utf8", env: { ...process.env, GITHUB_EVENT_PATH: path, GITHUB_EVENT_NAME: "pull_request" },
    });
  };
  assert.equal(run({ title: "fix(cli): preserve literal $(printf text)", body: null }).status, 0);
  assert.equal(run({ title: "feat(cli)!: replace schema", body: "## Validation\nChecked output" }).status, 1);
  assert.equal(run({ title: "feat(cli)!: replace schema", body: "BREAKING CHANGE: Migrate consumers." }).status, 0);
});
