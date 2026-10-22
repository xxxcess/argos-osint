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

test("replacement roles permit nested handoffs and restrict source edits", () => {
  assert.equal(config.default_agent, "architect");
  assert.equal(config.experimental.subagent_depth, 3);
  assert.equal(config.subagent_depth, undefined);
  for (const id of ["build", "plan", "general", "explore"]) assert.equal(config.agents[id].disabled, true);
  for (const id of ["architect", "coder", "tester"]) {
    assert.equal(config.agents[id].mode, "all");
    assert.equal(effect(id, "execute", "*"), "deny");
    assert.equal(effect(id, "browser", "*"), "deny");
    assert.equal(effect(id, "shell", "*"), "allow");
    assert.equal(effect(id, "subagent", "*"), "deny");
    assert.equal(effect(id, "skill", "*"), "allow");
  }
  for (const parent of ["architect", "coder"]) {
    for (const child of ["architect", "coder", "tester"]) assert.equal(effect(parent, "subagent", child), "allow");
    const worker = JSON.parse(readFileSync(join(root, `${parent}-worker.json`), "utf8"));
    assert.match(worker.model, /^[^/]+\/[^/]+$/);
  }
  assert.equal(effect("tester", "subagent", "architect"), "allow");
  assert.equal(effect("coder", "edit", "*"), "allow");
  for (const id of ["architect", "tester"]) {
    assert.equal(effect(id, "edit", "*"), "deny");
    assert.equal(effect(id, "edit", "**/.planning/**"), "allow");
  }
});

test("TUI command loads the registered screenshot skill and verify keeps offline flags", () => {
  const command = readFileSync(join(root, "commands/tui-verify.md"), "utf8");
  assert.match(command, /argos-tui-verify/);
  const verify = readFileSync(join(root, "commands/ecc-verify.md"), "utf8");
  assert.match(verify, /--locked --no-default-features/);
  assert.match(verify, /ARGOS_EMBED.*unset/);
});

test("worker prompts retain leaf roles and caller-owned continuation", () => {
  const architect = readFileSync(join(root, "agents/architect.md"), "utf8");
  const coder = readFileSync(join(root, "agents/coder.md"), "utf8");
  assert.match(architect, /AUDIT_UNIT and CONSULT are leaf modes/);
  assert.match(coder, /EDIT_UNIT/);
  assert.match(coder, /Do not delegate, build or test/);
  for (const id of ["architect", "coder", "tester"]) {
    assert.match(readFileSync(join(root, "agents", `${id}.md`), "utf8"), /Resume only your own child sessionID/);
  }
});
