import test from "node:test";
import assert from "node:assert/strict";
import plugin from "../plugins/gsd-v2.js";

test("GSD V2 registers tool and compaction hooks plus graph guidance", async () => {
  const hooks = { tool: {}, session: {} };
  let transform;
  await plugin.setup({
    location: { directory: process.cwd() },
    agent: { list: async () => [], transform: async (callback) => { transform = callback; } },
    tool: { hook: async (name, callback) => { hooks.tool[name] = callback; } },
    session: { hook: async (name, callback) => { hooks.session[name] = callback; } },
    event: { subscribe: async function* () {} },
  });
  assert.equal(typeof hooks.tool["execute.before"], "function");
  assert.equal(typeof hooks.tool["execute.after"], "function");
  assert.equal(typeof hooks.session.compaction, "function");
  const updates = new Map();
  transform({ update: (id, callback) => { const agent = { system: "Original" }; callback(agent); updates.set(id, agent.system); } });
  assert.match(updates.get("gsd-codebase-mapper"), /^For Argos code exploration/);
  const context = { agent: "gsd-codebase-mapper", system: [] };
  hooks.session.context(context);
  assert.match(context.system[0].text, /graphify query/);
  assert.throws(
    () => hooks.tool["execute.before"]({ tool: "read", input: { filePath: ".env" } }),
    /Secret read guard/,
  );
});
