import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import plugin from "../plugins/handoff.js";

function harness(root, storage = new Map()) {
  const hooks = { tool: {}, session: {} };
  const ctx = {
    location: { directory: root, project: { id: root } },
    tool: { hook: async (name, callback) => { hooks.tool[name] = callback; } },
    session: {
      hook: async (name, callback) => { hooks.session[name] = callback; },
      context: async () => [{ type: "assistant", content: [{ type: "text", text: "Implemented and checked." }] }],
    },
    storage: { get: async (key) => storage.get(key), set: async (key, value) => storage.set(key, value) },
    event: { subscribe: async function* () {} },
  };
  return { hooks, ctx, storage };
}

test("source searches need no index or prerequisite query", async () => {
  const { hooks, ctx } = harness("/repo-without-index");
  const dispose = await plugin.setup(ctx);
  for (const [tool, input] of [["bash", { command: "rg --files" }], ["grep", { pattern: "binding" }], ["glob", { pattern: "**/*.rs" }]]) {
    assert.doesNotThrow(() => hooks.tool["execute.before"]({ sessionID: "search", tool, input }));
  }
  dispose();
});

test("compaction stores bounded handoff and new session loads it once", async () => {
  const root = mkdtempSync(join(tmpdir(), "argos-handoff-"));
  const shared = new Map();
  const first = harness(root, shared);
  await plugin.setup(first.ctx);
  first.hooks.session.prompt({ sessionID: "old", prompt: { text: "Investigate the binder" } });
  first.hooks.tool["execute.before"]({ sessionID: "old", tool: "read", input: { filePath: "crates/core/binder.rs" } });
  await first.hooks.session.compaction({ sessionID: "old", system: [] });
  const second = harness(root, shared);
  await plugin.setup(second.ctx);
  const request = { sessionID: "new", system: [] };
  await second.hooks.session.context(request);
  await second.hooks.session.context(request);
  assert.equal(request.system.length, 1);
  assert.match(request.system[0].text, /Investigate the binder/);
  assert.match(request.system[0].text, /binder.rs/);
});

test("idle saves a bounded latest request", async () => {
  const root = mkdtempSync(join(tmpdir(), "argos-handoff-"));
  const { hooks, ctx, storage } = harness(root);
  let send;
  ctx.event.subscribe = async function* () {
    const event = await new Promise((resolve) => { send = resolve; });
    yield event;
  };
  await plugin.setup(ctx);
  hooks.session.prompt({ sessionID: "idle", prompt: { text: "x".repeat(800) } });
  send({ type: "session.idle", properties: { sessionID: "idle" } });
  await new Promise((resolve) => setTimeout(resolve, 0));
  const saved = storage.get(`handoff/${root}`);
  assert.equal(saved.request.length, 500);
  assert.equal(saved.outcome, "Implemented and checked.");
});
