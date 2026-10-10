import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "opencode.json"), "utf8"));
const skills = ["graphify", "argos-plan", "argos-implement", "ecc-plan", "ecc-review", "ecc-verify", "ecc-checkpoint", "ecc-learn", "argos-tui-verify"];

function rules(agent) {
  return config.agents[agent].permissions;
}

function effect(agent, action, resource) {
  const matched = rules(agent).filter((rule) => rule.action === action && rule.resource === resource);
  assert.ok(matched.length > 0, `${agent} ${action} ${resource}`);
  return matched[matched.length - 1].effect;
}

test("primary agents keep a short skill list and split edit subagents", () => {
  assert.equal(config.default_agent, "plan");
  for (const agent of ["build", "plan"]) {
    assert.equal(effect(agent, "execute", "*"), "deny");
    assert.equal(effect(agent, "browser", "*"), "deny");
    assert.equal(effect(agent, "gsd_*", "*"), "deny");
    assert.equal(effect(agent, "subagent", "*"), "deny");
    assert.equal(effect(agent, "skill", "*"), "deny");
    for (const id of skills) assert.equal(effect(agent, "skill", id), "allow", id);
    for (const id of skills) assert.ok(readFileSync(join(root, "skills", id, "SKILL.md"), "utf8"));
    const skillRules = rules(agent).filter((rule) => rule.action === "skill");
    assert.equal(skillRules[0].resource, "*");
    assert.deepEqual(skillRules.slice(1).map((rule) => rule.resource), skills);
  }
  for (const id of ["explore", "ecc-planner", "ecc-reviewer"]) {
    assert.equal(effect("build", "subagent", id), "allow");
    assert.equal(effect("plan", "subagent", id), "allow");
  }
  assert.equal(effect("build", "subagent", "ecc-edit"), "allow");
  assert.equal(rules("plan").some((rule) => rule.action === "subagent" && rule.resource === "ecc-edit"), false);
  assert.equal(effect("plan", "subagent", "*"), "deny");
  assert.equal(effect("plan", "edit", "*"), "deny");
});

test("TUI command loads the registered screenshot skill and verify keeps offline flags", () => {
  const command = readFileSync(join(root, "commands/tui-verify.md"), "utf8");
  assert.match(command, /argos-tui-verify/);
  const verify = readFileSync(join(root, "commands/ecc-verify.md"), "utf8");
  assert.match(verify, /--locked --no-default-features/);
  assert.match(verify, /ARGOS_EMBED.*unset/);
});

test("ecc-edit formats its files and cannot run the test suite", () => {
  const text = readFileSync(join(root, "agents/ecc-edit.md"), "utf8");
  assert.match(text, /mode: subagent/);
  assert.match(text, /cargo fmt -- <paths>/);
  const shellDeny = text.indexOf('resource: "*"\n    effect: deny');
  const fmtAllow = text.indexOf('resource: "cargo fmt *"\n    effect: allow');
  assert.ok(shellDeny > -1 && fmtAllow > shellDeny);
  const skill = readFileSync(join(root, "skills/argos-implement/SKILL.md"), "utf8");
  assert.match(skill, /"agent": "ecc-edit"/);
  assert.match(skill, /agent_cargo\.py test --workspace/);
});
