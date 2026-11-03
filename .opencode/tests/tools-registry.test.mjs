import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "opencode.json"), "utf8"));

function rules(agent) {
  return config.agents[agent].permissions;
}

function effect(agent, action, resource) {
  const matched = rules(agent).filter((rule) => rule.action === action && rule.resource === resource);
  assert.ok(matched.length > 0, `${agent} ${action} ${resource}`);
  return matched[matched.length - 1].effect;
}

function workerModel(name) {
  return JSON.parse(readFileSync(join(root, name), "utf8")).model;
}

const activeAgents = ["coordinator", "orchestrator", "architect", "rust-coder", "react-coder", "reviewer", "tester", "auditor", "scribe", "utility"];

test("coordinator is the default agent with configured models and teamwork catalog roles enforce nested handoffs and write boundaries", () => {
  assert.equal(config.default_agent, "coordinator");
  // Model choices are customer configuration; validate usable IDs, not stale provider pins.
  const modelId = /^[^/\s]+\/[^\s]+$/;
  assert.match(config.model, modelId);
  for (const id of activeAgents) assert.match(config.agents[id].model, modelId);
  assert.equal(config.experimental.subagent_depth, 4);
  assert.equal(config.subagent_depth, undefined);
  for (const id of ["build", "plan"]) assert.ok(!config.agents[id] || config.agents[id].disabled === true);
  for (const id of ["general", "explore"]) assert.equal(config.agents[id], undefined);
  for (const id of activeAgents) {
    assert.equal(config.agents[id].mode, "all");
    assert.equal(effect(id, "execute", "*"), "allow");
    assert.equal(effect(id, "browser", "*"), "deny");
    assert.equal(effect(id, "shell", "*"), "allow");
    assert.equal(effect(id, "subagent", "*"), "deny");
    assert.equal(effect(id, "skill", "*"), "allow");
  }
  for (const child of ["orchestrator", "architect", "rust-coder", "react-coder", "reviewer", "tester", "auditor", "scribe", "utility"]) {
    assert.equal(effect("coordinator", "subagent", child), "allow");
  }
  assert.equal(effect("coordinator", "question", "*"), "allow");
  for (const parent of ["orchestrator"]) {
    for (const child of ["architect", "rust-coder", "react-coder", "reviewer", "tester", "auditor", "scribe", "utility"]) {
      assert.equal(effect(parent, "subagent", child), "allow");
    }
  }
  assert.match(workerModel("architect-worker.json"), /^[^/]+\/[^/]+(\/[^/]+)*(#[^/]+)?$/);
  for (const id of ["architect", "rust-coder", "react-coder"]) {
    assert.equal(rules(id).some((rule) => rule.action === "subagent" && rule.effect === "allow"), false);
  }
  assert.equal(effect("tester", "subagent", "architect"), "allow");
  assert.equal(effect("reviewer", "subagent", "architect"), "allow");
  for (const id of ["coordinator", "rust-coder", "react-coder"]) assert.equal(effect(id, "edit", "*"), "allow");
  for (const id of ["orchestrator", "architect", "reviewer", "tester", "auditor"]) {
    assert.equal(effect(id, "edit", "*"), "deny");
    assert.equal(effect(id, "edit", "**/.planning/**"), "allow");
  }
  for (const id of ["orchestrator", "architect", "reviewer", "tester", "auditor"]) {
    assert.equal(rules(id).some((rule) => rule.action === "edit" && rule.effect === "allow" && /opencode/.test(rule.resource)), false);
  }
  assert.equal(effect("tester", "edit", ".agent-scratch/**"), "allow");
  assert.equal(effect("tester", "edit", "**/.agent-scratch/**"), "allow");
});

test("utility agent performs quick non-code edits only", () => {
  const doc = readFileSync(join(root, "agents/utility.md"), "utf8");
  assert.match(doc, /Never edit source code, tests, or CI workflows/);
  assert.match(doc, /do not spawn children/i);
  assert.equal(effect("utility", "edit", "*"), "deny");
  for (const resource of ["docs/**", "**/docs/**", "README.md", "**/README.md", ".planning/**", "**/.planning/**", ".opencode/**", ".agy/**", ".agent-scratch/**"]) {
    assert.equal(effect("utility", "edit", resource), "allow");
  }
  for (const resource of [".opencode/tests/**", "crates/**", "packages/**", "apps/**", "scripts/**", ".github/**"]) {
    assert.equal(effect("utility", "edit", resource), "deny");
  }
  assert.equal(rules("utility").some((rule) => rule.action === "subagent" && rule.effect === "allow"), false);
});

test("commands map to catalog specialists and verify uses the embedded integration gate", () => {
  const command = readFileSync(join(root, "commands/tui-verify.md"), "utf8");
  assert.match(command, /argos-tui-verify/);
  const verify = readFileSync(join(root, "commands/ecc-verify.md"), "utf8");
  const review = readFileSync(join(root, "commands/ecc-review.md"), "utf8");
  assert.match(command, /agent: tester/);
  assert.match(review, /agent: reviewer/);
  assert.match(verify, /local_matrix\.sh --storage embedded/);
  assert.doesNotMatch(verify, /--no-default-features/);
  assert.match(verify, /ARGOS_EMBED.*unset/);
  assert.match(verify, /no Rust build for docs\/config-only/);
});

test("teamwork prompts enforce interview triage, leaf modes, and caller-owned continuation", () => {
  const coordinator = readFileSync(join(root, "agents/coordinator.md"), "utf8");
  const orchestrator = readFileSync(join(root, "agents/orchestrator.md"), "utf8");
  const architect = readFileSync(join(root, "agents/architect.md"), "utf8");
  const reviewer = readFileSync(join(root, "agents/reviewer.md"), "utf8");
  const auditor = readFileSync(join(root, "agents/auditor.md"), "utf8");
  assert.match(coordinator, /Concise Pre-Execution Interview/);
  assert.match(coordinator, /question/);
  assert.match(coordinator, /existing session authorization/);
  assert.match(coordinator, /no ask tool exists/);
  assert.match(coordinator, /Translating Team Needs/);
  assert.match(coordinator, /Dynamic Team Selection from Catalog/);
  assert.match(orchestrator, /INTEGRITY_VIOLATION/);
  assert.match(architect, /Do not spawn children/);
  const rustCoder = readFileSync(join(root, "agents/rust-coder.md"), "utf8");
  const reactCoder = readFileSync(join(root, "agents/react-coder.md"), "utf8");
  assert.match(rustCoder, /Do not spawn children/);
  assert.match(reactCoder, /Do not spawn children/);
  assert.match(reviewer, /APPROVE \| REJECT/);
  assert.match(auditor, /CLEAN/);
  assert.match(auditor, /INTEGRITY_VIOLATION/);
  assert.match(auditor, /do not delegate/i);
  for (const id of ["coordinator", "orchestrator", "reviewer", "tester"]) {
    assert.match(readFileSync(join(root, "agents", `${id}.md`), "utf8"), /Resume only your own child sessionID/);
  }
});

test("shared guide, planning skill, and OpenCode catalog models are configured with catalog providers", () => {
  assert.deepEqual(config.instructions, [".opencode/AGENTS.md"]);
  assert.ok(config.skills.includes("~/.agents/skills/planning-with-files"));
  const guide = readFileSync(join(root, "AGENTS.md"), "utf8");
  assert.ok(guide.length < readFileSync(join(root, "../AGENTS.md"), "utf8").length);
  for (const id of activeAgents) {
    assert.match(config.agents[id].model, /^(opencode(-go)?|openrouter|nvidia|openai)\//);
    assert.doesNotMatch(readFileSync(join(root, "agents", `${id}.md`), "utf8"), /google\/|openai\/|anthropic\/|zenmux\//i);
  }
});

test("Scribe and Auditor preserve leaf non-delegation and strict edit scopes", () => {
  assert.equal(config.agents.scribe.model, config.agents.auditor.model);
  assert.equal(effect("scribe", "edit", "*"), "deny");
  for (const resource of ["docs/**", "**/docs/**", "README.md", "**/README.md", "**/.planning/**"]) {
    assert.equal(effect("scribe", "edit", resource), "allow");
  }
  assert.equal(rules("scribe").some(r => r.action === "subagent" && r.effect === "allow"), false);
  assert.equal(rules("auditor").some(r => r.action === "subagent" && r.effect === "allow"), false);
  assert.equal(rules("scribe").some(r => r.action === "edit" && r.effect === "allow" && /crates|scripts|opencode/.test(r.resource)), false);
  for (const name of ["setup-wiki", "refresh-wiki", "docs-review"]) {
    assert.match(readFileSync(join(root, `commands/${name}.md`), "utf8"), /agent: scribe/);
  }
  for (const id of ["rust-coder", "react-coder"]) {
    const coderDoc = readFileSync(join(root, `agents/${id}.md`), "utf8");
    assert.match(coderDoc, /parent.*delegate.*scribe/);
    assert.match(coderDoc, /documentation updates must precede verification/);
    assert.match(coderDoc, /fixes affecting documented behavior/);
    assert.match(coderDoc, /implement and format the assigned changes yourself/);
    assert.doesNotMatch(coderDoc, /Fan-out mandate/);
  }
  const scribe = readFileSync(join(root, "agents/scribe.md"), "utf8");
  assert.match(scribe, /do not delegate/i);
  assert.match(scribe, /no-op review/i);
});
