# Unified Investigation Harness

Shared evidence-driven pipeline for:

- **Recon Chat** — interactive multi-turn investigations
- **Home Composer** — launcher that allocates a session immediately
- **Intel Briefing & Jobs** — multi-section background reports

Surfaces keep their own UI, permissions, and stored ids. Admission, picker, executor, evidence curation, and synthesis are shared. Recon turn details: [architecture.md](architecture.md).

[![Investigation harness](diagrams/investigation-harness.svg)](diagrams/investigation-harness.html)

## Model roles

Defaults live under **Models → Defaults**. Empty derived roles inherit. Explicit config wins. Cycles are rejected. In-flight tasks keep snapshotted assignments.

| Logical Role | Purpose | Default inheritance |
|--------------|---------|---------------------|
| `recon` | Directives, task dependencies | Root default |
| `tool_picker` | Next eligible tool | OpenRouter Jev (`typesafe/jev-1.13`) |
| `synthesis` | Assessments and answers | Root default |
| `classifier` | Taxonomy and tags | OpenRouter Jev (`typesafe/jev-1.13`) |
| `summarization` | Compress evidence | Inherits `synthesis` |
| `evidence_curator` | Source-linked passages | Inherits `recon` |
| `entity_resolver` | Identity conflicts | Inherits `classifier` |
| `claim_assessor` | Verdicts and citations | Inherits `synthesis` |
| `investigation_controller` | Checkpoints and dispatch | Inherits `recon` |

## Catalog projection

59 tools in 14 intelligence categories. `hunter_tech_lookup` aliases `hunter_technologies`.

1. Web Discovery
2. News & Events
3. Publisher Context
4. Legal & Litigation
5. Corporate & Organizations
6. Professional Identity
7. Social Content
8. Public Account Corroboration
9. Domain & Infrastructure
10. Historical Web
11. Software & Code
12. Geography & Places
13. Bitcoin & Blockchain
14. Vulnerabilities & Cyber

| Level | Audience | Contents |
| --- | --- | --- |
| 1 Compact | Planner / Controller | Cost, prerequisites, limits |
| 2 Candidates | Picker | Binding status, expected contribution |
| 3 Contract | Executor | JSON schema, route, retry guidance |

## Validation gates

1. **Task admission** — surface eligibility, no duplicate tasks, valid dependencies
2. **Tool preflight**
   - Atlas-only news tools (`atlas_gnews`, `atlas_newsdata`, `atlas_currents`, `atlas_newsapi`) forbidden in Recon and Intel
   - `sociavault_google_search` needs **both** weak Firecrawl **and** an unmet evidence need
   - Platform-native tools need an unmet platform-native need
3. **Evidence admission** — URL/domain, retrieval timestamp, quote/facts
4. **Claim assessment** — claim-specific isolation; contradictions kept
5. **Publication** — every assertion grounded in admitted passages

## Reasoning channels

- Reasoning is never promoted to answer text or evidence. Reasoning-only completion = empty, bounded repair.
- Stream answer (`on_delta`) and reasoning (`on_reasoning`) separately.
- TUI thinking rows: `▶ Thinking… · {role} · {model} · {timestamp}`. Expand with Enter/click.
- `/thinking` toggles disclosure. `/details` toggles directive/tool payloads.

## Persistence

SQLite (current schema version 34; harness tables from v20+; v33 adds `insight_refresh_execution.activity`/`activity_at`, v34 adds `insight_supersessions`):

| Table | Contents |
| --- | --- |
| `investigation_tasks` | `pending`, `executing`, `completed`, `failed`, `blocked`, `cancelled` |
| `investigation_task_dependencies` | Prerequisite edges |
| `investigation_evidence_passages` | Deduped quotes, entity bindings, stance |
| `investigation_claim_assessments` | `verified`, `refuted`, `inconclusive`, `insufficient_evidence` |
| `investigation_events` | Gate/tool/transition trace, secrets redacted |
| `investigation_stream_parts` | Durable streams for resume |

## Shared claims and derivations

`investigation::insights` owns candidate packets, qualified source claims, evidence readiness, Synthesis inferences and assessed typed links. Preparation remains separate from assessment. A source allegation, denial or speculation remains an attributed claim. Kind, fidelity, evidence relation and lifecycle are independent. Mentions are candidates, never support.

The `insights-v3` assessment lane uses at most eight items per compiled-budget batch and a deduplicated original-evidence dictionary. Claims are canonicalized and capped before assessment; inferences and links share final batches. One-window report extraction with eight claims, five inferences and eight links targets five successful logical operations: extraction, claim batch, Synthesis, and two final batches. Actual wire attempts include bounded repairs, failures and declared fallback routes; fixtures measure those separately from successful operations.

Insight operations allow at most two wire attempts per route, including one output-contract repair with precise validation errors. A second invalid output advances to the next compatible declared route. The shared executor owns all retry/admission, records exact stage/category/attempts/skips, and links operations to the refresh task (and Recon run where applicable). Ordinary callers keep their 4-primary/3-fallback policy and terminal `InvalidResult`. Refusal, cancellation, invalid local input and persistence failure remain terminal. JSON output/completion limits are requested only for capability-mapped JSON-mode transports; unknown endpoints use strict local answer validation. Private model reasoning is never accepted as answer JSON.

Durable refreshes yield after two uncached logical operations, renewing their lease while a request runs. Continuations reuse successful checkpoints and return to the fair owner queue. Optional unavailable assessments settle as withheld with gaps; faithful attributed claims remain eligible without invented corroboration. Mandatory extraction/Synthesis failures, stale evidence or lease, cancellation and all-withheld model failures preserve the active snapshot. A legitimate assessed empty result is distinct. Reduced snapshots and later repaired snapshots have separate publication identities, with Brain/provenance/head/index outbox committed together.

Schema 32 adds `insight_refresh_execution` for explicit retry epochs and progress; it does not reset old checkpoints. **Retry extraction** admits one bounded epoch for recoverable unfinished work and clears only that owner's failed/cancelled dispatch barriers. Reopening a brief and restart never admit new retry epochs. Compatible completed work remains reusable; relevant role changes are compared against the prior task's captured `role_snapshot`, not recomputed with today's routes. Deleted, superseded and history-retired work cannot be retried. Recovery logs an individual owner's error and continues with later owners. Index retries remain a separate local worker action and make no inference requests.

Recon stages insights from retained tools before the analytical answer. Its new `insights-v1` extraction receipt prevents answer mining and citation retrofit; legacy queued jobs retain `legacy-v1` dispatch. Article reports delegate to one durable `insight_refresh` task after asynchronous inventory, collection and assessment. Section and BLUF prose consume its readiness result; insufficient evidence produces observations and gaps.

See [NLP coverage](nlp-preprocessing.md) and the [shared harness](diagrams/investigation-harness.html). Schema 29 stores generation, lease-fenced activation, immutable original evidence and stage receipts alongside canonical Brain records.

Atlas uses [classification-only collection and Related articles](atlas-classification-and-related-articles.md). Its local shared mentions are tentative browsing leads, separate from investigative claims, assessed Links and evidence confidence. Current Atlas stages count article metadata; historical memory units retain legacy labels.

## Progress, supersession and publication

Intel and Jobs show one status sequence: Queued → Waiting for output (or Waiting for decision) → Receiving output → Validating → Published → Indexed, with explicit Stalled, Retry wait and failure states; header receipt is never shown as model activity and no fake percentages or token counts are shown. Notable activity transitions are logged as `insight.activity` events (never per delta). When a source body, completed report, captured role route or preparation contract changes, the owner's generation advances: older queued tasks become `superseded` immediately and running tasks are retired at admission, pre-send, the 30s heartbeat (dropping the in-flight request) or the publication fence. Supersession is intentional retirement — no error category, refunded attempt, no owner error — with reason and replacement link in `insight_supersessions` and an `insight.superseded` event; Jobs detail reads `Superseded · replaced by <job> · <reason>`. Visits, polling, heartbeats and progress never create generations. Index-only repair after publication drains the canonical outbox without new model calls. Figures: [intel-report](diagrams/intel-report.html), [persistence](diagrams/persistence.html).
