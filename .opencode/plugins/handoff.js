const sessions = new Map();
const short = (value, limit = 500) => String(value ?? "").slice(0, limit);
function session(id) {
  if (!sessions.has(id)) sessions.set(id, { loaded: false, request: "", paths: new Set() });
  return sessions.get(id);
}

function resultText(value) {
  if (typeof value === "string") return value;
  if (typeof value?.content === "string") return value.content;
  if (Array.isArray(value?.content)) return value.content.filter((part) => part?.type === "text").map((part) => part.text).join("\n");
  return "";
}

export default {
  id: "argos.handoff",
  async setup(ctx) {
    const root = ctx.location.directory;
    const handoffKey = `handoff/${ctx.location.project?.id ?? root}`;
    await ctx.session.hook("prompt", (event) => {
      const s = session(event.sessionID);
      s.request = short(event.prompt?.text);
    });
    await ctx.tool.hook("execute.before", (event) => {
      const s = session(event.sessionID);
      const input = event.input ?? {};
      const path = input.filePath ?? input.path;
      if (typeof path === "string" && s.paths.size < 12 && !path.includes("/auth.json")) s.paths.add(short(path, 180));
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
