import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "opencode.json"), "utf8"));
const skills = ["graphify", "argos-plan", "argos-implement", "ecc-plan", "ecc-review", "ecc-verify", "ecc-checkpoint", "ecc-learn"];

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
  assert.match(skill, /cargo test --workspace/);
});
