# OpenCode V2 workflow

GSD still owns phases and milestones in `.planning/`. Project plugins under `.opencode/plugins/` add graph-first navigation, a GSD hook bridge, and a short session handoff.

## Primary tools

OpenCode V2 ignores `experimental.primary_tools`. `build` and `plan` in `.opencode/opencode.json` deny Code Mode (`execute`), the browser catalog, GSD MCP tools, GSD subagents, and every skill except the list below. Call the provider schema for these tools. Do not invent `tools.<namespace>[...]()` calls.

| Tool | Use |
| --- | --- |
| `shell` | `graphify query "<question>"` before a repo-wide search |
| `read`, `grep`, `glob` | Paths the query returns, or a path-scoped search |
| `edit`, `write` | `build` only. `plan` cannot edit project files |
| `skill` | `{ "id": "<id>" }` |
| `webfetch`, `websearch`, `question` | One URL, one query, or one user choice |
| `subagent` | `agent`, `description`, `prompt`. `build` may launch `explore`, `ecc-planner`, `ecc-reviewer`, and `ecc-edit`. `plan` may launch the first three |

Skill ids: `graphify`, `argos-plan`, `argos-implement`, `ecc-plan`, `ecc-review`, `ecc-verify`, `ecc-checkpoint`, `ecc-learn`.

`argos-plan` writes `.planning/<YYYY-MM-DD-slug>/` and keeps that text out of the chat. Load `graphify` only when the shell command above is unclear. `/gsd-...` still runs GSD with its own tools.

## Phase edits

On `build`, load `argos-implement`. Split the in-progress phase into disjoint file sets and call `subagent` once per set. Use `ecc-edit`. Set `background` to true and send every call in the same turn when there are two or more sets. The prompt carries the phase, the file list, and the change. Do not set `model`, and do not poll.

`ecc-edit` changes only those files, then runs `cargo fmt -- <paths>` for the Rust files it changed. It cannot run the test suite.

When the children finish, the parent runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`, with shell `timeout` `600000` and `ARGOS_EMBED` unset. Failures go back to `ecc-edit`, using `sessionID` to continue the editor that owns the file. The parent re-runs the failed command, then the full trio, then `graphify update .`. `plan` does not launch `ecc-edit`.

## Graph first

[![Agent exploration](diagrams/agent-graphify.svg)](diagrams/agent-graphify.html)

Tracked graph: `graphify-out/graph.json`.

| Command | Use |
| --- | --- |
| `graphify query "<question>"` | Shell tool, before the first broad search |
| `graphify explain "<concept>"` | One symbol |
| `graphify path "<A>" "<B>"` | Relationship |
| `graphify update .` | After code edits (AST only) |

Track only `graph.json`, `manifest.json`, `GRAPH_REPORT.md`. GSD reads the same graph via `.planning/config.json`.

Skip graphify only if the graph is the thing being fixed, or the user says so.

## ECC commands

| Command | Purpose |
| --- | --- |
| `/ecc-plan` | Read-only plan from graph + source |
| `/ecc-review` | Read-only diff review |
| `/ecc-verify` | fmt, clippy, tests, graph refresh |
| `/ecc-checkpoint` | Short manual handoff |
| `/ecc-learn` | Reusable practice (needs approval) |

Planner and reviewer inherit the active model and cannot edit files. Automatic handoff: ≤500 characters each for latest request and outcome, plus up to 12 paths. Loads once in a later session for the same project. Does not replace GSD artifacts.

## Verify setup

With OpenCode V2 in this project:

- `/api/plugin` — `argos.graphify`, `argos.gsd-v2`
- `/api/command` — five `ecc-*` commands
- `/api/skill` — matching skills

Incompatible global GSD V1 was moved from `~/.config/opencode/plugins/gsd-core.js` to `~/.config/opencode/gsd-core.v1.js`. A later GSD update may restore it; move it out of `plugins/` again if startup errors return.

```sh
node --test .opencode/tests/*.test.mjs
```
