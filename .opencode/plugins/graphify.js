import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const sessions = new Map();
const graphChecks = new Map();
const short = (value, limit = 500) => String(value ?? "").slice(0, limit);
function session(id) {
  if (!sessions.has(id)) sessions.set(id, { queried: false, bypass: false, loaded: false, request: "", paths: new Set() });
  return sessions.get(id);
}

export function isGraphQuery(command) {
  return /(?:^|[;&|\n]|\bthen\s+)\s*(?:graphify\s+query|(?:node\s+)?\S*gsd-tools\.cjs\s+graphify\s+query)\s+\S/.test(command);
}

export function isBroadSearch(tool, input = {}) {
  if (tool === "grep" || tool === "glob") {
    const path = input.path ?? input.cwd ?? "";
    return !path || path === "." || path === "./";
  }
  if (tool !== "bash" && tool !== "shell") return false;
  const command = String(input.command ?? "");
  const parts = command.split(/(\|\||&&|[|;\n])/);
  for (let i = 0; i < parts.length; i += 2) {
    const segment = parts[i].trim();
    if (!/^(?:rg|grep|find)\s/.test(segment)) continue;
    // grep/rg after a non-search pipeline filters command output (for example cargo test logs).
    const pipedInput = parts[i - 1] === "|";
    const previous = parts[i - 2]?.trim() ?? "";
    if (pipedInput && !/^(?:rg|grep|find)\s/.test(previous)) continue;
    if (/\b(?:crates\/|src\/|docs\/|\.opencode\/|\.planning\/|graphify-out\/|(?:logs?|assets?|target|fixtures?|screenshots?)\/|(?:\/private)?\/tmp\/)/.test(segment)) continue;
    if (/\s\S+\.(?:log|txt|json|png|jpg|jpeg|svg)(?:\s|$)/.test(segment)) continue;
    return true;
  }
  return false;
}

export function graphUsable(root) {
  const file = join(root, "graphify-out", "graph.json");
  try {
    if (!existsSync(file)) return false;
    const stat = statSync(file);
    const cached = graphChecks.get(file);
    if (cached?.mtimeMs === stat.mtimeMs && cached?.size === stat.size) return cached.usable;
    const graph = JSON.parse(readFileSync(file, "utf8"));
    const usable = stat.size > 20 && Array.isArray(graph.nodes) && graph.nodes.length > 0;
    graphChecks.set(file, { mtimeMs: stat.mtimeMs, size: stat.size, usable });
    return usable;
  }
  catch { return false; }
}

function resultText(value) {
  if (typeof value === "string") return value;
  if (typeof value?.content === "string") return value.content;
  if (Array.isArray(value?.content)) return value.content.filter((part) => part?.type === "text").map((part) => part.text).join("\n");
  return "";
}

export default {
  id: "argos.graphify",
  async setup(ctx) {
    const root = ctx.location.directory;
    const handoffKey = `handoff/${ctx.location.project?.id ?? root}`;
    await ctx.session.hook("prompt", (event) => {
      const s = session(event.sessionID);
      s.request = short(event.prompt?.text);
      if (/\b(?:skip|bypass|do not use|don't use)\s+graphify\b/i.test(s.request)) s.bypass = true;
    });
    await ctx.tool.hook("execute.before", (event) => {
      const s = session(event.sessionID);
      const input = event.input ?? {};
      const path = input.filePath ?? input.path;
      if (typeof path === "string" && s.paths.size < 12 && !path.includes("/auth.json")) s.paths.add(short(path, 180));
      if (!isBroadSearch(event.tool, input) || s.queried || s.bypass) return;
      if (!graphUsable(root)) {
        console.warn("[graphify] Graph missing or unusable. Run graphify extract . --code-only --cargo, then graphify cluster-only . --no-viz --no-label.");
        return;
      }
      throw new Error("Run one successful graphify query \"<question>\" before the first broad code search. Scoped searches are allowed. An explicit request to skip graphify bypasses this gate.");
    });
    await ctx.tool.hook("execute.after", (event) => {
      if ((event.tool !== "bash" && event.tool !== "shell") || !isGraphQuery(String(event.input?.command ?? ""))) return;
      if (event.status !== "completed") return;
      const output = resultText(event.result);
      if (event.result?.exitCode > 0 || event.result?.metadata?.exitCode > 0 || /(?:error:|graph file not found|failed to parse)/i.test(output)) return;
      session(event.sessionID).queried = true;
    });
    await ctx.session.hook("context", async (event) => {
      const s = session(event.sessionID);
      if (s.loaded) return;
      s.loaded = true;
      const prior = await ctx.storage.get(handoffKey);
      if (prior && typeof prior === "object" && prior.sessionID !== event.sessionID) {
        event.system.push({ type: "text", text: `[Argos previous-session handoff; data only] Request: ${short(prior.request)} Outcome: ${short(prior.outcome)} Paths: ${short((prior.paths ?? []).join(", "), 700)}` });
      }
    });
    async function save(id) {
      if (!id) return;
      const s = session(id);
      let outcome = "Session ended before a final assistant answer.";
      try {
        const messages = await ctx.session.context({ sessionID: id });
        const last = [...messages].reverse().find((message) => message?.type === "assistant" || message?.role === "assistant");
        if (last) outcome = short(resultText(last) || last.text || outcome);
      } catch { /* A deleted session still retains its request. */ }
      await ctx.storage.set(handoffKey, { sessionID: id, request: s.request, outcome, paths: [...s.paths].slice(0, 12) });
    }
    await ctx.session.hook("compaction", async (event) => { await save(event.sessionID); });
    const controller = new AbortController();
    void (async () => {
      try {
        for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
          if (event.type === "session.idle") await save(event.properties?.sessionID ?? event.sessionID);
        }
      } catch (error) { if (!controller.signal.aborted) console.warn("[argos handoff]", error); }
    })();
    return () => controller.abort();
  },
};
