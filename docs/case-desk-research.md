# Case Desk, case evidence, and research

## Ask saved evidence, then investigate explicitly

The default general Desk scope is the saved collection. `/scope case <id>`,
`/scope reports <ids>`, or `/scope report` narrows retrieval; `/scope desk` retains
legacy unassigned-only retrieval. Ordinary questions search accepted observations
and indexed report passages, return attributed excerpts and stable citations, and
recommend source passages, entities, and cases. Unresolved parts stay unresolved.
No ordinary question makes provider or model calls. Exact domains/emails use exact
identifier boundaries. Observations without resolvable local sources are excluded.

A gap suggests `/new <question>`. This opens a compact scope screen with the question,
new/existing case (`c` cycles), seeds, included recommendations (`e`), and allowed
public/sensitive (`s`)/active (`a`) source scope. Global provider host/privacy limits
also apply. First-action checkboxes start empty. Space selects first sources; Enter
saves the case and starts only those actions. An empty selection saves a usable case
without a report, job, or graph. Search respects the selected Facts/Web/News/Social
sources; domain and identity first actions use exact seeds rather than a broad query.
Source scope is saved with the case; later focused actions cannot bypass it.

`/case` lists saved cases and `/case <id-or-title>` opens Inbox + Workbench. The Desk Queue
also opens cases, including cases without reports, across restarts. Esc returns
to the preserved Desk prompt, highlight, scope, and scroll position.

## Next-action Desk and case workspace

The Desk shows up to three **Next Work** cards, a case **Queue**, and typed **Open
Questions**. Cases without reports are included. `w`, `q`, and `g` focus those
lists; `j/k` moves and Enter opens the selected case and lead. `/scope case <id>` or
report scopes restrict the queue to matching cases. `\` toggles the retrieval
transcript in one key. When there is no next work, the transcript stays visible.
Ordinary questions remain retrieval-only; selecting a lead never collects.
`/dashboard` returns to this Desk with the saved prompt and scroll position.

| Mode | Behavior |
| --- | --- |
| `1` Desk | Next Work, Queue, Open Questions, and collapsible retrieval transcript. |
| `2` Inbox + Workbench | Persistent why-now lead list beside Plan, Intake, and So What. Pending/deferred review ranks first, followed by uncollected, conflicting, and candidate gaps. Accepted link degree is a tie-break; review/acceptance/degree counts remain separate. |
| `3` Graph | Immediate relationships and typed holes for the selected lead and expanded neighbors. Solid accepted links, dashed candidates, gold uncollected sockets, dim collected-absent sockets, red conflicts. `←/→` selects a link; `x` expands one neighbor; `z` collapses; `v` opens its intake; `o` opens its source. Limit: 16 immediate relationships. |
| `4` Product | Accepted observations for the selected lead; `c` switches to case-wide. Space selects IDs and Enter prefills `/draft final <IDs>` for editing/submission. Pending gaps remain visible and unresolved. Candidate identity observations are excluded. |

The inbox persists across centers. Tab cycles inbox, center, inspector, and
composer; Shift+Tab focuses the inbox. `j/k` operates the focused list. Narrow
terminals stack the inbox above the center; the header never wraps seven tabs.
`5` Review, `6` Jobs, and `7` Path are migration aliases. `s` cycles Intake,
Evidence, and chronological intake presentations. `/work [lead]`, `/graph`, and
`/gaps [lead|case]` open the relevant cached surface.

`e` focuses the selected lead's **Plan** (the case question is used when no lead
exists). Checkboxes start empty. Space checks an eligible action; `e` or Enter
queues the explicitly checked set as separate bounded jobs. Each remains independently
cancellable with `/cancel-jobs`; there is no implicit run-all or recursive collection.
Unready actions remain visible and disabled. Opening configuration never scans or
installs. `/investigate <provider> [exact input]` retains its single-action form and
stays on Workbench. Selected-lead jobs appear under Plan; `J` opens those jobs and
Enter filters intake to that job. `/jobs` opens the case-wide job list.

Graph `g` focuses holes. Enter or `e` on an uncollected hole opens Workbench with
that action checked, without collecting. Collected-absent holes open job/source
history and cannot silently replay the same action/input; they are **not a
real-world negative finding**. Conflict and candidate holes open the relevant
observations for review. Missing or uncollected evidence does not establish a
real-world gap. No gap is inferred solely from a missing edge.

`p` overlays Path on Graph, using the selected pair or existing `f/t` pins. `n/N`
chooses a path; `[ / ]` chooses a hop; `o` opens its source. Path uses only accepted,
noncandidate, source-supported links, up to four hops, and never starts research.
Pins and map selection survive center switches; Esc closes Path to the same map.
No recovered path is not proof of a real-world gap.

Gaps use an additive case-owned SQLite table. Projections backfill eligible never-run
actions, completed jobs with no extractable observations, existing candidate links,
and already categorized conflicts. A successful job closes its uncollected gap;
zero observations opens collected-absent. Partial/failed jobs never become absence
findings. Closed records retain history; `/correct`, `/merge`, and `/unmerge` follow
the case projection without rewriting original evidence. Gaps never start jobs.
Case clearing/deletion removes gap records alongside investigation data while
retaining detached reports and historical citations.

`/review <id> accept|retain|reject|defer <reason>` validates scope and appends a decision.
Acceptance enables source-backed observed links. Rejection/defer cannot promote a
link. Accepting a username match does not verify identity; accepting co-occurrence
does not establish ownership. Evidence count, acceptance, and link degree are shown
separately. No absent co-occurrence edge is proof of a real-world evidence gap.

Case `/correct <entity-id> <replacement_or_suppress> <reason>` changes only the
projection; underscores represent spaces. `/undo-correction <id>` reverses it.
`/merge <canonical-id> <other-ids> --reason <reason>` needs in-scope source evidence;
`/unmerge <decision-id> --reason <reason>` reverses the projection. Original entity
records, aliases, mentions, source text, and review history are retained. Historical
report workspaces keep byte-offset corrections and their existing identity commands.

## Providers → Research

All configuration stays on the existing Research page of Providers. Keys `1–5`
switch optional capability groups. Enter on Integration selects the next card;
`j/k` navigates settings, Enter edits/acts, and Save persists the selected integration.

| Phase | Cards / capability |
| --- | --- |
| Discovery | Search and public document sources; source operators are provider-dependent. |
| Infrastructure | DNS/RDAP/certificates/Wayback, InternetDB, Shodan, scoped Katana; SpiderFoot remains visibly unavailable. |
| Identity & contacts | GitHub identity, Published contacts, WhatsMyName, Maigret, Mosint. |
| Exposure | LeakCheck Public and XposedOrNot; sensitive lookup opt-in required. |
| Analysis & output | Conservative normalization, pending relationship review, explicit report output; default report kind and visible lead count. |

Cards expose supported inputs, enabled/readiness state, native HTTP/local/container
mode, credential reference or executable, supported/detected version, limits,
exact hosts, privacy controls, cache/retries, and offline Test Configuration.
Credentials are masked and stored by reference in owner-only `auth.json`. Model
accounts and Writer/Tools routing remain independent. Configured does not mean
successful collection or account entitlement. Shodan requires its account/key;
WhatsMyName requires a versioned local dataset and selected sites. Contacts require
active HTTP and an exact allowed host. Katana, Maigret, Mosint, and SpiderFoot
collection is unavailable even if an executable is configured or verified.

Opening Research never installs or scans. Managed install/update/remove first shows
a plan and destination; Apply submits a visible cancellable job. Katana installation
is pinned to 1.7.0 with official SHA-256 checks, platform/version/flag verification,
and previous-installation retention on failure. Other automatic installers are
unavailable. Removal accepts only an Argos-managed manifest/binary. No system Python,
container runtime, or browser installation is automatic.

## Jobs, review, and optional reports

API/executable work shares the existing bounded research queue, timeout, rate gate,
output limits, request cache, redirect/public-address checks, and provider scope.
`/cancel-jobs` cancels queued/running collection and managed tools; successful results
from independent jobs survive failures. Queued cancellation is persisted. Interrupted
jobs become partial on restart and are not automatically replayed. Explicit research
may reuse completed cached results. Paid requests require a fresh explicit selection
after restart or cache expiry.

Raw attributed provider hits/artifacts and normalized observations are stored separately
from analyst decisions. Persisting the same job result is idempotent; separate retrievals
retain their source histories and are categorized as repeated/changed for review.
Projection/index/source work runs on the existing bounded blocking workers; case projection and ingestion check cancellation cooperatively. Refreshes
read the affected case rather than rebuilding every report graph; stale generations
cannot replace a newly selected case. Report ingestion is version/pipeline keyed.

`/draft final|addendum|revision|followup <accepted observation IDs>` explicitly writes
a new report from that selected reviewed evidence, local citation targets, attribution,
event/retrieval dates, uncertainty, and pending review gaps. It never reassigns case
observations to the report. Addendum/revision/follow-up here are separate case outputs;
historical `/save-update addendum|revision|followup` retains its versioned report update
behavior. `/source <observation-id>` opens the original artifact. `/cite <number>` or
`/cite report-id@vN:Lline` opens current/historical report passages.

## Additive migration

The existing SQLite evidence tables remain the sole store. Entity memberships are
namespaced by case/report so identical canonical identifiers can belong to multiple
cases without overwriting membership. Original records are not deleted. Report text,
versioned passages, transcript history, decisions, credentials, and old report snapshots
remain readable. Indexing invalidates only the changed report and derived collection
cache, not unrelated report graphs.

Case ingestion uses existing report `case_id` values only. Explicitly included passages
are authorized source references in the saved investigation scope; the report's metadata
association remains unchanged. Unassigned reports remain accessible in the Desk/report
reader and are never assigned by guesswork. The deterministic report extractor proposes
mentions and same-passage co-occurrence as reviewable observations. It is one ingestion
path; provider observations create case links without any report. Ingestion markers
prevent unchanged report text from being re-extracted on each research result. Historical
jobs' source bindings are recovered from their observation/artifact IDs for projection,
without rewriting observation or decision history.

## Validation and current limits

Offline fixtures and local mocks cover reportless case restart, citation/scope isolation,
review-to-network updates, identity candidates versus observed links, correction/merge
reversals, additive report ingestion and historical citations, explicit drafting, queued
cancellation and partial results, provider readiness/privacy, and narrow terminal views.
The full workspace suite includes existing local mock HTTP tests; no routine live provider
lookup, paid call, scan, or installation is required.

Desk answers are deterministic attributed excerpts rather than model-synthesized prose.
Timeline is textual; semantic conflict detection still depends on explicit provider flags.
Advanced report-only Matrix coverage and Ribbon source extraction stay in the historical
report reader. CLI collection, live account quota discovery, automatic dataset refresh,
and automatic recursive pivots remain unavailable. Analysis exposes the supported
conservative parser/review policy rather than implying arbitrary extraction presets work.


The visible case header has Workbench, Graph, and Product centers, with Desk
available through `1` or Esc. Each center retains navigation and selected links.
Changing centers reads cached evidence and never starts collection.

## Clear or delete case data

Press `D` in a case workspace or use `/case-data`. `/clear-case [case ID or title]`
keeps an empty case; `/delete-case [case ID or title]` removes it. On the Cases &
reports list, `x` prepares a delete plan. Preparation does not remove data. The
review screen counts messages, case evidence/history, review decisions, research
jobs/cache entries, and retained reports. `/confirm-case <exact ID>` applies the
plan; `/cancel-case-data` or Esc in the workspace cancels it. Removal is irreversible.

Active case jobs must finish or be cancelled first with `/cancel-jobs`. The apply
worker rechecks the plan under an atomic transaction and refuses changed plans.
It removes chat/tool history, case-only entities, observations, artifacts,
relationships, reviews, investigation scope, correction/identity history, jobs/cache,
and the derived case network. Report-backed raw records and their review history
remain under their report; saved reports are detached from the case without changing
files, versions, or citation targets. Other cases, shared sources, memories, provider
configuration, and credentials are preserved. Durable reset markers reject old job
results after clear and prevent deleted cases from being recreated by stale workers.
New explicitly selected research can start in a cleared case. No provider request is
started by these controls.

## Official contract references

- [Katana 1.7.0 release](https://github.com/projectdiscovery/katana/releases/tag/v1.7.0) and [CLI documentation](https://docs.projectdiscovery.io/opensource/katana/running).
- [Shodan API](https://developer.shodan.io/api) and [host lookup entitlement](https://help.shodan.io/developer-fundamentals/looking-up-ip-info).
- [XposedOrNot endpoints, limits, and response examples](https://xposedornot.com/api_doc).
- [WhatsMyName dataset](https://github.com/WebBreacher/WhatsMyName).
- [Mosint](https://github.com/alpkeskin/mosint), [Maigret CLI](https://maigret.readthedocs.io/en/latest/command-line-options.html), and [SpiderFoot 4.0 CLI](https://github.com/smicallef/spiderfoot/blob/v4.0/sf.py).
- [IANA TLD list](https://data.iana.org/TLD/tlds-alpha-by-domain.txt), bundled for deterministic domain validation.
