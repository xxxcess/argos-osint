# Fleetworks network clients

These terminal commands use the v1 HTTP API. Build them alongside Venture with
`cargo build --locked -p fleetworks-cli -p fleetworks-venture`.

| Executable | Commands |
| --- | --- |
| `fleetworks-atlas` | `tui`, `start --request-id ID`, `days`, `articles DAY [--category CATEGORY] [--limit N] [--cursor CURSOR]` |
| `fleetworks-brain` | `tui`, `search QUERY [--alias] [--limit N]`, `actor ID`, `brief RUN ID`, `topology SEED` |
| `fleetworks-jobs` | `tui`, `list`, `detail ID`, `retry ID`, `cancel ID`, `logs [--job-id ID] [--limit N]` |
| `fleetworks-tools` | `tui`, `list`, `run TOOL_ID --request-id ID --input JSON`, `result CALL_ID` |
| `fleetworks-models` | `tui`, `defaults`, `set ROLE PROVIDER MODEL [--account ID] [--fallbacks JSON]`, `clear ROLE` |
| `fleetworks-profile` | `tui`, `workspace`, `health`, `show`, `export NAME` |

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

Open Atlas with `venture client atlas tui`. Use `days` to refresh dates and
categories, `articles` to show the newest retained day, or
`articles DAY CATEGORY` to filter. Use `next` for the next page and `open #1`
to read the first article on the current page. `start REQUEST_ID` prompts
before submitting a collection cycle.

Open Brain with `venture client brain tui`. Use `find NAME` for an exact actor
name or `aliases PREFIX` for indexed aliases, then `actor #1` to open a
selected result. Shared names and aliases are marked ambiguous. Discovery
requires a seeded actor catalog and includes its revision. Use `actor ID`
for a known actor, `brief RUN ID` for an article's retained brief, and
`topology SEED` for its recorded nodes and edges. The actor dossier prints
brief commands for associated article occurrences.

Use `venture client tools run dork_generate --request-id example-1 --input '{"objective":"Find public annual reports"}'`
to submit a local tool call. Use `venture client tools result fleet-tool-example-1`
to read its durable result. Reuse the same request ID and input after an
uncertain response. A call interrupted by a service restart is never
automatically repeated because a provider may already have executed it.
Provider credentials come from the service workspace. Atlas collection
endpoints remain behind Atlas provider admission.

Open Tools with `venture client tools tui`. Use `list FILTER` to filter the
catalog and `detail #1` to inspect required inputs and restrictions.
`run TOOL_ID REQUEST_ID` prompts for one JSON object and confirmation.
`result CALL_ID` shows the status and a bounded observations preview.

Open Models with `venture client models tui` to read saved primary and fallback
routes. Use `defaults ROLE` to select a role. Open Profile with
`venture client profile tui` to inspect the local `workspace`, service
`health`, and service profile with `show`.

Models `set` changes a canonical role's primary route and preserves the current
fallbacks unless `--fallbacks` supplies a replacement array. `clear` removes the
primary and fallback routes for the selected role. The interactive forms
prompt for confirmation.

Profile `export NAME` writes `exports/NAME.json` inside the service workspace.
The file contains saved credentials and has owner-only permissions. The API
returns only the relative path and schema version. Choose a new name for
another export. The interactive form prompts for confirmation.

The interactive sessions use ordinary terminal input and output and close on
`quit` or end of input. Use the one-shot commands for complete JSON responses.

Atlas article dates use `YYYY-MM-DD`. Pass
the returned `next_cursor` into the next request unchanged. Brain topology
accepts a canonical memory ID or a `brief:RUN/ARTICLE` seed. Tools, model
defaults, Profile show, and Jobs logs require their corresponding Fleetworks
API routes. The service rollout and terminal parity are tracked in the
[implementation plan](../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md).
