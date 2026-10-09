import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "opencode.json"), "utf8"));
const skills = ["graphify", "argos-plan", "ecc-plan", "ecc-review", "ecc-verify", "ecc-checkpoint", "ecc-learn"];

function rules(agent) {
  return config.agents[agent].permissions;
}

test("primary agents share a short tool allowlist", () => {
  assert.equal(config.default_agent, "plan");
  assert.deepEqual(rules("build"), rules("plan"));
  const effects = new Map(rules("build").map((rule) => [`${rule.action}:${rule.resource}`, rule.effect]));
  for (const action of ["execute:*", "browser:*", "gsd_*:*"]) {
    assert.equal(effects.get(action), "deny", action);
  }
  assert.equal(effects.get("subagent:gsd-*"), "deny");
  assert.equal(effects.get("skill:*"), "deny");
  for (const id of skills) assert.equal(effects.get(`skill:${id}`), "allow", id);
  const skillRules = rules("build").filter((rule) => rule.action === "skill");
  assert.equal(skillRules[0].resource, "*");
  assert.deepEqual(skillRules.slice(1).map((rule) => rule.resource), skills);
});

test("explore can run graphify and compaction keeps its model", () => {
  const explore = new Map(rules("explore").map((rule) => [`${rule.action}:${rule.resource}`, rule.effect]));
  assert.equal(explore.get("shell:*"), "allow");
  assert.equal(explore.get("edit:*"), "deny");
  assert.equal(config.agents.compaction.model, "openrouter/nemotron-3-ultra-550b-a55b:free#high");
  readFileSync(join(root, "skills/argos-plan/SKILL.md"), "utf8");
});
