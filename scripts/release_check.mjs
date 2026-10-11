#!/usr/bin/env node
/** Offline commit and shared-version checks. Node 20+, no npm dependencies. */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const types = new Set(["feat", "fix", "perf", "refactor", "docs", "test", "build", "ci", "chore", "style", "revert"]);
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

export function checkMessage(message, version = "0.1.0") {
  const [header, ...body] = message.replace(/\r\n/g, "\n").trimEnd().split("\n");
  const match = /^([a-z]+)(?:\(([a-z0-9][a-z0-9,._/-]*)\))?(!)?: (\S.*)$/.exec(header);
  if (!match || !types.has(match[1])) {
    throw new Error("Use <type>(optional-scope): description; see docs/versioning.md for allowed types");
  }
  if (header !== header.trimEnd()) throw new Error("Commit title must not end in whitespace");
  const text = body.join("\n");
  const footers = [...text.matchAll(/^BREAKING(?: CHANGE|-CHANGE):([^\n]*)/gm)];
  if (footers.some(footer => !footer[1].trim())) throw new Error("BREAKING CHANGE needs a compatibility and migration description");
  const breaking = Boolean(match[3]) || footers.length > 0;
  if (breaking && footers.length === 0) throw new Error("A ! title also needs a BREAKING CHANGE: footer describing migration");
  const override = /^Release-As: (\S+)[ \t]*$/im.exec(text);
  if (/^Release-As:/im.test(text) && (!override || !semver.test(override[1]))) throw new Error("Release-As must contain a valid SemVer");
  const impact = breaking ? (version.startsWith("0.") ? "minor" : "major")
    : match[1] === "feat" ? "minor"
    : ["fix", "perf"].includes(match[1]) ? "patch" : "none";
  return { type: match[1], scope: match[2], breaking, impact, releaseAs: override?.[1] };
}

// Read the explicitly structured version tables, not arbitrary TOML values.
// Cargo's existing --locked CI build remains the full TOML/resolver check.
function table(text, name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`^\\[${escaped}\\][ \\t]*(?:#[^\\n]*)?\\r?\\n([\\s\\S]*?)(?=^\\[|$(?![\\s\\S]))`, "m").exec(text);
  if (!match) throw new Error(`Missing [${name}] table`);
  return match[1];
}

function stringField(text, key) {
  const match = new RegExp(`^${key}\\s*=\\s*"([^"\\n]+)"[ \\t]*(?:#[^\\n]*)?$`, "m").exec(text.replace(/\r\n/g, "\n"));
  if (!match) throw new Error(`Missing quoted ${key}`);
  return match[1];
}

export function checkVersions(root = ROOT, tag) {
  const read = path => readFileSync(join(root, path), "utf8");
  const cargo = read("Cargo.toml");
  const version = stringField(table(cargo, "workspace.package"), "version");
  if (!semver.test(version)) throw new Error(`Invalid SemVer: ${version}`);
  const manifest = JSON.parse(read(".release-please-manifest.json"));
  if (Object.keys(manifest).join() !== "." || manifest["."] !== version) throw new Error("Release manifest differs from workspace version");
  if (read("version.txt").trim() !== version) throw new Error("version.txt differs from workspace version");
  if (tag && tag !== `v${version}`) throw new Error(`Expected tag v${version}, received ${tag}`);

  const members = /\bmembers\s*=\s*\[([^\]]+)\]/.exec(table(cargo, "workspace"));
  if (!members) throw new Error("Explicit workspace members are required");
  const paths = [...members[1].replace(/#[^\n]*/g, "").matchAll(/"([^"\n]+)"/g)].map(match => match[1]);
  if (!paths.length || paths.some(path => /[*?]/.test(path))) throw new Error("Use explicit member paths and update release selectors when adding crates");
  const names = paths.map(path => {
    const member = table(read(`${path}/Cargo.toml`), "package");
    if (!/^version\.workspace\s*=\s*true[ \t]*(?:#[^\n]*)?$/m.test(member) || /^version\s*=/m.test(member)) {
      throw new Error(`${path} must inherit version.workspace = true`);
    }
    return stringField(member, "name");
  });
  const lock = read("Cargo.lock").split(/^\[\[package\]\][ \t]*\r?\n/m).slice(1);
  for (const name of names) {
    const entries = lock.filter(entry => stringField(entry, "name") === name);
    if (entries.length !== 1 || stringField(entries[0], "version") !== version || /^source\s*=/m.test(entries[0])) {
      throw new Error(`Cargo.lock local package ${name} differs from workspace version`);
    }
  }
  const config = JSON.parse(read("release-please-config.json"));
  const packages = config.packages ?? {};
  if (Object.keys(packages).join() !== ".") throw new Error("Configure one root release for the workspace");
  const pkg = packages["."];
  if (pkg["release-type"] !== "simple" || pkg["version-file"] !== "version.txt"
      || pkg["include-component-in-tag"] !== false || pkg["include-v-in-tag"] !== true
      || pkg["bump-minor-pre-major"] !== true || pkg["bump-patch-for-minor-pre-major"] !== false) {
    throw new Error("Release strategy differs from the documented single-version/0.x policy");
  }
  const expected = new Set(["feat", "fix", "perf"]);
  for (const section of pkg["changelog-sections"] ?? []) {
    if (!section.hidden) expected.delete(section.type);
    if (!section.hidden && !["feat", "fix", "perf"].includes(section.type)) throw new Error(`Unexpected releasable type: ${section.type}`);
  }
  if (expected.size) throw new Error("Feature, fix and performance changes must be releasable");
  const extras = pkg["extra-files"] ?? [];
  if (extras.length !== 2) throw new Error("Use exactly the workspace and local lockfile version updaters");
  if (!extras.some(file => file.type === "toml" && file.path === "Cargo.toml" && file.jsonpath === "$.workspace.package.version")) {
    throw new Error("Missing workspace version updater");
  }
  const selector = '$.package[?(' + names.map(name => `@.name.value == ${JSON.stringify(name)}`).join(" || ") + ')].version';
  if (!extras.some(file => file.type === "toml" && file.path === "Cargo.lock" && file.jsonpath === selector)) {
    throw new Error("Cargo.lock updater must select exactly the workspace package names in member order");
  }
  return version;
}

export function checkRange(base, head, cwd = ROOT) {
  if (!/^[0-9a-f]{40}$/.test(base) || !/^[0-9a-f]{40}$/.test(head)) throw new Error("Commit range endpoints must be full Git SHAs");
  if (/^0+$/.test(base)) throw new Error("Missing base commit; fetch history rather than guessing the range");
  const output = execFileSync("git", ["log", "--format=%H%x00%B%x00", `${base}..${head}`], { cwd, encoding: "utf8" });
  const fields = output.split("\0");
  let count = 0;
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const sha = fields[i].trim();
    try { checkMessage(fields[i + 1]); }
    catch (error) { throw new Error(`${sha}: ${error.message}`); }
    count++;
  }
  return count;
}

function main(args) {
  const [command, ...rest] = args;
  if (command === "versions") {
    const tag = rest.length ? rest[0] : undefined;
    console.log(`Shared version ${checkVersions(ROOT, tag)} verified`);
  } else if (command === "message" && rest.length === 2 && rest[0] === "--file") {
    const version = checkVersions();
    const result = checkMessage(readFileSync(rest[1], "utf8"), version);
    console.log(`Commit valid; release impact: ${result.releaseAs ?? result.impact}`);
  } else if (command === "range" && rest.length === 2) {
    console.log(`${checkRange(...rest)} commit messages verified`);
  } else if (command === "event") {
    const version = checkVersions();
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
    if (event.pull_request) {
      const pr = event.pull_request;
      if (pr.title.includes("\n") || pr.title.includes("\r")) throw new Error("PR title must be one line");
      const result = checkMessage(`${pr.title}\n\n${pr.body ?? ""}`, version);
      console.log(`PR title/body valid; release impact: ${result.releaseAs ?? result.impact}`);
    } else if (process.env.GITHUB_EVENT_NAME === "push") {
      console.log(`${checkRange(event.before, event.after)} commit messages verified`);
    } else throw new Error("Expected a pull_request or push event");
  } else {
    throw new Error("Usage: node scripts/release_check.mjs versions [vX.Y.Z] | message --file <path> | range <base-sha> <head-sha> | event");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(process.argv.slice(2)); }
  catch (error) { console.error(`release-check: ${error.message}`); process.exitCode = 1; }
}
