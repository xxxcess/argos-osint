# Fleetworks network clients

These terminal commands use the v1 HTTP API. Build them alongside Venture with
`cargo build --locked -p fleetworks-cli -p fleetworks-venture`.

| Executable | Commands |
| --- | --- |
| `fleetworks-atlas` | `start --request-id ID`, `days`, `articles DAY [--category CATEGORY] [--limit N] [--cursor CURSOR]` |
| `fleetworks-brain` | `actor ID`, `brief RUN ID`, `topology SEED` |
| `fleetworks-jobs` | `tui`, `list`, `detail ID`, `retry ID`, `cancel ID`, `logs [--job-id ID] [--limit N]` |
| `fleetworks-tools` | `list` |
| `fleetworks-models` | `defaults` |
| `fleetworks-profile` | `workspace`, `health`, `show` |

Use `--workspace PATH` to override `FLEETWORKS_WORKSPACE` and the default
`.fleetworks` directory under the current directory. `profile workspace`
reports the selected absolute path without creating a workspace or contacting
the API.

Atlas start returns a job ID and a run ID. Use Jobs detail to check progress
and reuse the same request ID after an uncertain submission. Starting cycles,
cancelling jobs, and retrying a failed Atlas job require the authenticated API.

The API origin resolves from `--api-origin`, `FLEETWORKS_API_ORIGIN` in the
effective workspace environment, then `http://127.0.0.1:8080`. Select a
credential file with `--api-token-file`. Otherwise the client resolves
`FLEETWORKS_API_TOKEN` or `FLEETWORKS_API_TOKEN_FILE` from process variables
first and workspace configuration second, then checks `secrets/api-token`.
Relative token paths start at the workspace root. Venture provisions a private
API token when it starts the stack.

One-shot commands write JSON to stdout. Use `--json` for compact output and
`--help` for command arguments. Open the line-oriented Jobs & Logs session
with `venture client jobs tui` or `fleetworks-jobs tui`. Use `list` to refresh
the jobs, `detail #1` to open the first job, and `logs #1` to read its recent
events. Retry and cancel require explicit confirmation in this session.
Enter `quit` to close the session.

Atlas article dates use `YYYY-MM-DD`. Pass
the returned `next_cursor` into the next request unchanged. Brain topology
accepts a canonical memory ID or a `brief:RUN/ARTICLE` seed. Tools, model
defaults, Profile show, and Jobs logs require their corresponding Fleetworks
API routes. The service rollout and terminal parity are tracked in the
[implementation plan](../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md).
