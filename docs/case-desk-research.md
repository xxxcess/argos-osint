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

`/case` lists saved cases and `/case <id-or-title>` opens Leads. The Cases & reports
list also opens cases, including cases without reports, across restarts. Esc returns
to the preserved Desk prompt, highlight, scope, and scroll position.

## Case workspace

| Key/view | Behavior |
| --- | --- |
| `1` Leads | Small ranked list, selected question, recent changes, pending review, evidence gaps, jobs. Rank is accepted evidence count, not a strategic score. |
| `2` Focus map | Selected entity and immediate typed links. `j/k` selects entities; `←/→` selects a link; `x` expands one neighbor and `z` collapses. `v` opens its review; `p` opens Path for the pair. |
| `3` Evidence table | Observation and relationship rows show entity/type, source, retrieval/event dates, and review state. `/filter <text/source/state>` filters and `s` cycles entity, relationship, source, event/retrieval date, and review-state sorts. |
| `4` Timeline | Observation/provider and research lanes. Event date, publication, and retrieval stay separate; undated events stay undated. |
| `5` Review | New/repeated/changed/conflicting/candidate identity findings. `a/r/d/t` prepares accept/reject/defer/retain, with an editable reason. |
| `6` Jobs | Queued/running/partial/completed/failed/cancelled states, provider/input/progress/errors. Enter filters Review to the job's observations. |
| `7` Path | The only advanced case tool. After review, select a pair in Focus and press `p`; explore up to four hops through accepted supported links. `j/k` selects entities, `f/t` pins endpoints, `n/N` chooses paths, `[ / ]` chooses hops, and `o` opens the original source. Candidate links are excluded; no recovered path is not a real-world negative finding. |

`o` or Enter opens a selected source artifact or report passage. Focus, Evidence,
and Timeline resolve the same source targets. Narrow terminals stack the inspector
below the map rather than requiring a dense multi-column layout. Selection, expanded
neighbors, filter, and source scroll remain stable across background refreshes.

`e` offers eligible focused providers for the selected lead (or selected question
when there are no entities). This is a cached menu. Selecting a lead never collects.
Enter submits exactly one action. `/investigate <provider> [exact input]` provides
the same explicit action; open a case or report first. Newly extracted identifiers
are candidate leads with source mentions and do not trigger recursive enrichment.

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


All seven workspace views are separate clickable tabs whenever a case is open.
The selected tab is highlighted and the tab bar wraps at narrower terminal widths.
Number keys `1–7` select the same tabs; each retains its row and scroll position.
Changing tabs reads cached evidence and never starts collection.

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
