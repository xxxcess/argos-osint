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

const activeAgents = ["coordinator", "orchestrator", "architect", "coder", "reviewer", "tester", "auditor", "scribe"];

test("coordinator is default GLM medium agent and teamwork catalog roles enforce nested handoffs and write boundaries", () => {
  assert.equal(config.default_agent, "coordinator");
  assert.equal(config.model, "opencode/glm-5.3-flash-free#medium");
  assert.equal(config.experimental.subagent_depth, 4);
  assert.equal(config.subagent_depth, undefined);
  for (const id of ["build", "plan", "general", "explore"]) assert.equal(config.agents[id].disabled, true);
  for (const id of activeAgents) {
    assert.equal(config.agents[id].mode, "all");
    assert.equal(effect(id, "execute", "*"), "allow");
    assert.equal(effect(id, "browser", "*"), "deny");
    assert.equal(effect(id, "shell", "*"), "allow");
    assert.equal(effect(id, "subagent", "*"), "deny");
    assert.equal(effect(id, "skill", "*"), "allow");
  }
  for (const child of ["orchestrator", "architect", "coder", "reviewer", "tester", "auditor", "scribe"]) {
    assert.equal(effect("coordinator", "subagent", child), "allow");
  }
  for (const parent of ["orchestrator", "architect", "coder"]) {
    for (const child of ["architect", "coder", "reviewer", "tester", "auditor", "scribe"]) {
      assert.equal(effect(parent, "subagent", child), "allow");
    }
  }
  for (const parent of ["architect", "coder"]) {
    const worker = JSON.parse(readFileSync(join(root, `${parent}-worker.json`), "utf8"));
    assert.match(worker.model, /^[^/]+\/[^/]+$/);
  }
  assert.equal(effect("tester", "subagent", "architect"), "allow");
  assert.equal(effect("reviewer", "subagent", "architect"), "allow");
  assert.equal(effect("coder", "edit", "*"), "allow");
  for (const id of ["coordinator", "orchestrator", "architect", "reviewer", "tester", "auditor"]) {
    assert.equal(effect(id, "edit", "*"), "deny");
    assert.equal(effect(id, "edit", "**/.planning/**"), "allow");
  }
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
});

test("teamwork prompts enforce interview triage, leaf modes, and caller-owned continuation", () => {
  const coordinator = readFileSync(join(root, "agents/coordinator.md"), "utf8");
  const orchestrator = readFileSync(join(root, "agents/orchestrator.md"), "utf8");
  const architect = readFileSync(join(root, "agents/architect.md"), "utf8");
  const coder = readFileSync(join(root, "agents/coder.md"), "utf8");
  const reviewer = readFileSync(join(root, "agents/reviewer.md"), "utf8");
  const auditor = readFileSync(join(root, "agents/auditor.md"), "utf8");
  assert.match(coordinator, /Concise Pre-Execution Interview/);
  assert.match(coordinator, /Dynamic Team Selection from Catalog/);
  assert.match(orchestrator, /INTEGRITY_VIOLATION/);
  assert.match(architect, /AUDIT_UNIT and CONSULT are leaf modes/);
  assert.match(coder, /EDIT_UNIT/);
  assert.match(coder, /Do not delegate, build or test/);
  assert.match(reviewer, /APPROVE \| REJECT/);
  assert.match(auditor, /CLEAN/);
  assert.match(auditor, /INTEGRITY_VIOLATION/);
  assert.match(auditor, /do not delegate/i);
  for (const id of ["coordinator", "orchestrator", "architect", "coder", "reviewer", "tester"]) {
    assert.match(readFileSync(join(root, "agents", `${id}.md`), "utf8"), /Resume only your own child sessionID/);
  }
});

test("shared guide, planning skill, and OpenCode catalog models are configured without Google/Claude refs", () => {
  assert.deepEqual(config.instructions, [".opencode/AGENTS.md"]);
  assert.ok(config.skills.includes("~/.agents/skills/planning-with-files"));
  const guide = readFileSync(join(root, "AGENTS.md"), "utf8");
  assert.ok(guide.length < readFileSync(join(root, "../AGENTS.md"), "utf8").length);
  assert.equal(config.agents.coordinator.model, "opencode/glm-5.3-flash-free#medium");
  assert.equal(config.agents.orchestrator.model, "opencode/glm-5.3-flash-free#medium");
  assert.equal(config.agents.architect.model, "opencode/glm-5.3-flash-free#medium");
  assert.equal(config.agents.coder.model, config.agents.architect.model);
  assert.equal(config.agents.reviewer.model, "opencode/nemotron-3.5-super-vl-free#medium");
  assert.equal(config.agents.tester.model, "opencode/nemotron-3.5-super-vl-free#medium");
  assert.equal(config.agents.auditor.model, "opencode/nemotron-3.5-super-vl-free#medium");
  assert.equal(JSON.parse(readFileSync(join(root, "coder-worker.json"), "utf8")).model, "opencode/mimo-v2.6-flash-free");
  for (const id of activeAgents) {
    assert.doesNotMatch(config.agents[id].model, /google|claude|anthropic/i);
    assert.doesNotMatch(readFileSync(join(root, "agents", `${id}.md`), "utf8"), /google\/|anthropic\/|claude-/i);
  }
});

test("Scribe and Auditor preserve leaf non-delegation and strict edit scopes", () => {
  assert.equal(config.agents.scribe.model, config.agents.tester.model);
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
  const coder = readFileSync(join(root, "agents/coder.md"), "utf8");
  assert.match(coder, /agents\.<role>\.model/);
  assert.match(coder, /documentation updates before verification/);
  assert.match(coder, /Re-run `scribe` after fixes/);
  const scribe = readFileSync(join(root, "agents/scribe.md"), "utf8");
  assert.match(scribe, /do not delegate/i);
  assert.match(scribe, /no-op review/i);
});
