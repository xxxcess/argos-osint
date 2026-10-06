import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const home = join(homedir(), ".config", "opencode");
const hooks = join(home, "hooks");
const core = join(home, "gsd-core");
const names = { read: "Read", grep: "Grep", write: "Write", edit: "Edit", apply_patch: "MultiEdit", bash: "Bash", shell: "Bash", webfetch: "WebFetch", websearch: "WebSearch", subagent: "Task" };
const graphFirst = "For Argos code exploration, query graphify-out/graph.json with graphify query before broad search. Then inspect the returned paths. If the graph is unavailable, continue with scoped source search.";

function run(name, payload, strict = false) {
  const file = join(hooks, name);
  if (!existsSync(file)) { if (strict) console.warn(`[GSD V2] Missing ${file}`); return; }
  const result = spawnSync("node", [file], { input: JSON.stringify(payload), encoding: "utf8", timeout: 25000, cwd: payload.cwd });
  if (result.error) { if (strict) console.warn(`[GSD V2] ${name}: ${result.error.message}`); return; }
  let body;
  try { body = JSON.parse(result.stdout || "{}"); } catch { body = {}; }
  if (result.status === 2 || body.decision === "block") throw new Error(body.reason || `Blocked by ${name}`);
  const advice = body.hookSpecificOutput?.additionalContext;
  if (advice) console.warn(`[GSD V2] ${advice}`);
}

function payload(event, cwd, phase) {
  const input = event.input ?? {};
  return {
    hook_event_name: phase, cwd,
    tool_name: names[event.tool] ?? event.tool,
    tool_input: {
      file_path: input.filePath ?? input.path ?? input.file_path,
      command: input.command,
      content: input.content,
      new_string: input.newString ?? input.new_string,
      old_string: input.oldString ?? input.old_string,
      glob: input.glob ?? input.include,
      url: input.url,
    },
  };
}

export default {
  id: "argos.gsd-v2",
  async setup(ctx) {
    const cwd = ctx.location.directory;
    if (!existsSync(core)) { console.warn("[GSD V2] GSD core is not installed; bridge inactive."); return; }
    const updateAgents = (editor) => {
      for (const id of ["gsd-codebase-mapper", "gsd-code-fixer", "gsd-debugger", "gsd-planner", "gsd-phase-researcher", "gsd-code-reviewer"]) {
        editor.update(id, (agent) => { agent.system = `${graphFirst}\n\n${agent.system ?? ""}`; });
      }
    };
    await ctx.agent.transform(updateAgents);
    await ctx.session.hook("context", (event) => {
      if (event.agent?.startsWith("gsd-")) event.system.push({ type: "text", text: graphFirst });
    });
    await ctx.tool.hook("execute.before", (event) => {
      if (event.tool === "read") {
        const path = event.input?.filePath ?? event.input?.path;
        if (typeof path === "string" && path.startsWith("~/.claude/gsd-core/")) {
          const target = join(core, path.slice("~/.claude/gsd-core/".length));
          if (event.input.filePath) event.input.filePath = target;
          else event.input.path = target;
        }
      }
      const p = payload(event, cwd, "PreToolUse");
      const kind = p.tool_name;
      if (["Write", "Edit"].includes(kind)) {
        run("gsd-prompt-guard.js", p, true);
        run("gsd-read-guard.js", p);
      }
      if (["Write", "Edit", "MultiEdit"].includes(kind)) run("gsd-worktree-path-guard.js", p, true);
      if (kind === "Write") run("gsd-write-guard.js", p, true);
      if (["Write", "Edit", "MultiEdit", "Bash"].includes(kind)) run("gsd-workflow-guard.js", p, true);
      if (["Read", "Grep", "Bash"].includes(kind)) run("gsd-secret-read-guard.js", p, true);
    });
    await ctx.tool.hook("execute.after", (event) => {
      if (event.status !== "completed") return;
      const p = payload(event, cwd, "PostToolUse");
      if (["Read", "WebFetch", "WebSearch"].includes(p.tool_name)) {
        p.tool_response = event.result?.content ?? event.result;
        run("gsd-read-injection-scanner.js", p);
      }
      if (event.sessionID) run("gsd-context-monitor.js", { ...p, session_id: event.sessionID });
    });
    await ctx.session.hook("compaction", (event) => {
      run("gsd-context-monitor.js", { hook_event_name: "PreCompact", session_id: event.sessionID, cwd });
      event.system.push({ type: "text", text: "GSD phase and milestone state lives in .planning/. Preserve the current phase and plan after compaction." });
    });
    const controller = new AbortController();
    void (async () => {
      try {
        for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
          if (event.type === "session.created") {
            const id = event.properties?.info?.id;
            const dir = event.properties?.info?.directory ?? cwd;
            run("gsd-ensure-canonical-path.js", { hook_event_name: "SessionStart", session_id: id, cwd: dir });
            run("gsd-check-update.js", { hook_event_name: "SessionStart", session_id: id, cwd: dir });
          }
          if (event.type === "file.edited") {
            const file = event.properties?.file;
            if (file && resolve(file) === resolve(cwd, ".planning", "config.json")) run("gsd-config-reload.js", { hook_event_name: "FileChanged", file_path: file, event: "change", cwd });
          }
        }
      } catch (error) { if (!controller.signal.aborted) console.warn("[GSD V2]", error); }
    })();
    return () => controller.abort();
  },
};
