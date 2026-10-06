import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import plugin, { isBroadSearch, isGraphQuery, graphUsable } from "../plugins/graphify.js";

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

test("graph guard is per session and accepts only successful queries", async () => {
  const root = mkdtempSync(join(tmpdir(), "argos-graph-"));
  mkdirSync(join(root, "graphify-out"));
  writeFileSync(join(root, "graphify-out", "graph.json"), '{"nodes":[1],"edges":[]}');
  const { hooks, ctx } = harness(root);
  await plugin.setup(ctx);
  const broad = (id) => hooks.tool["execute.before"]({ sessionID: id, tool: "bash", input: { command: "rg --files" } });
  assert.throws(() => broad("one"), /graphify query/);
  hooks.tool["execute.after"]({ sessionID: "one", tool: "bash", input: { command: 'graphify query "brain"' }, status: "error" });
  assert.throws(() => broad("one"), /graphify query/);
  hooks.tool["execute.after"]({ sessionID: "one", tool: "bash", input: { command: 'graphify query "brain"' }, status: "completed", result: { content: "Graph: 10 nodes" } });
  assert.doesNotThrow(() => broad("one"));
  assert.throws(() => broad("two"), /graphify query/);
  hooks.tool["execute.after"]({ sessionID: "two", tool: "bash", input: { command: 'node gsd-tools.cjs graphify query "brain"' }, status: "completed", result: { content: "Graph: 10 nodes" } });
  assert.doesNotThrow(() => broad("two"));
  assert.doesNotThrow(() => hooks.tool["execute.before"]({ sessionID: "three", tool: "bash", input: { command: "rg brain crates/argos-osint-core" } }));
  hooks.session.prompt({ sessionID: "four", prompt: { text: "Skip graphify for this task" } });
  assert.doesNotThrow(() => broad("four"));
});

test("missing graph permits search, invalid query does not unlock, and classifier stays scoped", async () => {
  const root = mkdtempSync(join(tmpdir(), "argos-graph-"));
  const { hooks, ctx } = harness(root);
  await plugin.setup(ctx);
  assert.equal(graphUsable(root), false);
  assert.doesNotThrow(() => hooks.tool["execute.before"]({ sessionID: "missing", tool: "grep", input: { pattern: "foo" } }));
  assert.equal(isBroadSearch("grep", { path: "crates/argos-osint-core" }), false);
  assert.equal(isBroadSearch("bash", { command: "rg foo crates/argos-osint-core" }), false);
  assert.equal(isBroadSearch("bash", { command: 'cd /repo && cargo test --workspace 2>&1 | grep -E "FAILED|ok"' }), false);
  assert.equal(isBroadSearch("bash", { command: 'cargo test -- --nocapture | rg "FAILED"' }), false);
  assert.equal(isBroadSearch("bash", { command: 'rg "panic" target/debug/test.log' }), false);
  assert.equal(isBroadSearch("bash", { command: 'rg "Profile" assets/screenshots/' }), false);
  assert.equal(isBroadSearch("bash", { command: 'rg "binding" | grep Hunter' }), true);
  assert.equal(isBroadSearch("bash", { command: 'cd /repo && rg "binding" .' }), true);
  assert.equal(isGraphQuery('node gsd-tools.cjs graphify query "x"'), true);
  assert.equal(isGraphQuery('echo graphify query "x"'), false);
  mkdirSync(join(root, "graphify-out"));
  writeFileSync(join(root, "graphify-out", "graph.json"), "{bad json");
  assert.equal(graphUsable(root), false);
});

test("compaction stores bounded handoff and new session loads it once", async () => {
  const root = mkdtempSync(join(tmpdir(), "argos-graph-"));
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
  const root = mkdtempSync(join(tmpdir(), "argos-graph-"));
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
