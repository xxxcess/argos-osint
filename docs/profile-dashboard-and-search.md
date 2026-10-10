# Profile dashboard and search

Profile (internal id `System`) is the observability and configuration module: an **Overview** dashboard of what Argos actually did, and a **System** tab that keeps the host, path and configuration surfaces. Keys and navigation in [usage.md](usage.md#profile). Module layout and the snapshot path in [architecture.md](architecture.md#profile-dashboard-and-config-transfer).

Nothing here invents a number. A metric the installation never collected renders **N/A, never `0`**, and an empty window is an empty section rather than a fabricated baseline. Rendering reads one immutable snapshot: no writes, no network calls, no model calls, and no prompts, article bodies, model outputs or keys.

## The two tabs

| Tab | Holds |
| --- | --- |
| **Overview** (default) | Filter strip, section navigator, 35 telemetry widgets |
| **System** | Host, Paths, **Refresh hardware**, and the **Configs** export/import popup |

`Tab` switches tabs. Only a bare `Tab` does: `Ctrl+Tab` still cycles modules and `Tab` inside a field still moves focus. The two tabs split the old single System pane — they do not add a pane beside it.

The System tab renders the Host pane, the Paths pane and the Refresh hardware button. **Logs** remains its own module next to Profile, as before; the module doc comment in `crates/argos-osint-bin/src/tui/profile.rs` mentions Logs among the System tab contents, but the rendered tab does not include it. The `?` help card for Profile also still describes only hardware and storage paths.

## The 35 widgets

Every widget is registered once in `WIDGETS` (`crates/argos-osint-bin/src/tui/profile.rs`); a test asserts the count, the absence of a duplicate id and the per-section totals, so a new widget cannot slip in unannounced. Inside a section the widgets render in presentation priority order — the most actionable first — and the section navigator moves between sections without duplicating widgets.

### Intel — 7 widgets

| Id | Widget | Measures |
| --- | --- | --- |
| `intel.volume` | Ingestion volume | Distinct canonical articles first seen per bucket, split by primary tag with untagged counted separately |
| `intel.confidence` | Initial vs current confidence | Paired initial claim confidence and current Brief rating across the five fixed 0–1 bands, with cohort mean, median and N |
| `intel.origins` | Country / origin mix | Distinct article volume by collection/source origin with share, plus `Other` and `Unknown` |
| `intel.enrichment` | Tag enrichment | Per tag: distinct articles, body / claim-extraction / report coverage, mean initial confidence and mean Brief rating with their Ns |
| `intel.reports` | Report outcomes | Latest terminal revision per (article, mode): completed / partial / failed, current waiting / blocked, mean and p95 wall, mean active, N |
| `intel.publishers` | Publishers | Top source domains by distinct articles with share and the top-3 concentration |
| `intel.freshness` | Article freshness | Published-to-first-ingestion delay in fixed buckets, with missing and future timestamps reported separately |

### Recon — 7 widgets

| Id | Widget | Measures |
| --- | --- | --- |
| `recon.outcomes` | Run outcomes | Mutually exclusive terminal outcomes per bucket and mode: completed with evidence, completed zero evidence, partial, failed, cancelled |
| `recon.stages` | Stage durations | Mean execution and mean wait per stage and mode, with N |
| `recon.recall` | Memory recall | Completed recall queries, queries with candidate hits, queries with accepted hits, candidate-to-accepted retention, accepted memories per run, dominant rejection reason |
| `recon.workload` | Workload per run | Runs, directives / calls / categories / accepted memories per run, median and p95 wall duration |
| `recon.diversity` | Tool diversity | Eligible scopes, scopes with ≥2 tools attempted, with ≥2 successful, independent source groups per scope and top shortfall reason per intelligence category (`recon_coverage`) |
| `recon.directives` | Directive resolution | Answered / partial / unresolved / blocked / unknown directive assessments per mode, with N |
| `recon.unresolved` | Unresolved directives | The unresolved and blocked directives with reason, evidence count, last progress and recorded next action |

### Atlas — 6 widgets

| Id | Widget | Measures |
| --- | --- | --- |
| `atlas.cycles` | Cycle outcomes | Completed / partial / failed / cancelled cycles per completion bucket |
| `atlas.hot_zones` | Hot zones | Origin table: articles, latest and mean temperature, latest tier, tier 1/2/3 shares, snapshot count |
| `atlas.temperature` | Temperature shifts | Largest absolute temperature movement per origin between the latest and the preceding comparable snapshot, with warming / cooling label |
| `atlas.cycle_time` | Cycle time | Mean and p95 completed-cycle wall duration per bucket, mean queue wait and N |
| `atlas.discovery` | Discovery mix | Fetched candidate occurrences split into retained-new, retained-existing, duplicate-in-cycle, rejected and pending |
| `atlas.backlog` | Backlog | Per stage: queued / running / waiting / blocked units from the latest transition per (stage, unit), oldest pending age and latest error category |

### Models — 8 widgets

| Id | Widget | Measures |
| --- | --- | --- |
| `models.capacity` | Provider capacity | Live sends in a rolling 60 s, effective RPM, active, queued and cooldown per quota scope. Live state: ignores the historical filters |
| `models.by_role` | Requests by role | Wire requests per bucket stacked by role |
| `models.latency` | Latency | p50 / p95 send-to-completion duration for finished attempts with N, plus p50 first-header and first-content timings |
| `models.queue` | Queue delay | p50 / p95 enqueue-to-send delay with N |
| `models.performance` | Provider performance | Hierarchical provider totals with expandable model rows: sends, completed attempts, attempt error %, 429 count, final operation failure %, mean and p95 duration |
| `models.fallback` | Fallback triggers | Triggered versus recovered operations per role and route with trigger reason, recovery %, and median / p95 time to the terminal outcome |
| `models.amplification` | Retry amplification | Sends per terminal operation per completion bucket, plus in-flight operations |
| `models.failures` | Model failure causes | Failed wire attempts per bucket split by failure category, with the share of all finished attempts |

`models.performance` and `models.fallback` currently render an empty state: their builders in `crates/argos-osint-core/src/profile_stats.rs` return no rows yet, because the schema-26 operation and attempt facts they group are not joined into those two shapes. The widgets are registered and asserted; the tables fill when that join lands. Live capacity and queue delay stay unavailable until the orchestration companion publishes (see [Provider orchestration companion](#provider-orchestration-companion)).

### Tools — 7 widgets

| Id | Widget | Measures |
| --- | --- | --- |
| `tools.usage` | Tool usage | Logical invocations per tool and category with the remote / local / cache split |
| `tools.attribution` | Attribution | Category totals split by trigger: Recon prompt, Intel brief, Atlas cycle, manual, scheduled, repair |
| `tools.outcomes` | Tool outcomes | Completed-nonempty, verified-zero, partial, failed and blocked invocations per bucket |
| `tools.reliability` | Reliability | Per tool: invocations, wire requests, cache-hit %, verified-zero %, remote error %, mean and p95 duration, dominant trigger and mode |
| `tools.search_health` | Search health | Google, Yandex and Mojeek: fetches, valid SERPs, verified-zero, challenge, parser mismatch, transport failure, usable results per fetch, cache hits, last success, parser version |
| `tools.evidence` | Evidence contribution | Successful nonempty invocations, invocations yielding ≥1 accepted evidence item, acceptance %, distinct evidence items, citations by completed reports |
| `tools.failure_causes` | Failure causes | Ranked terminal failure causes with count and share of finished invocations |

## Filter strip

One line above the section navigator.

| Control | Action |
| --- | --- |
| `period` | `1h` / `24h` / `7d` / `30d`. The default is the last 24 h |
| `app`, `provider`, `role`, `mode`, `tool`, `category` | Bounded dimension filters. An empty value reads `any` |
| `[c] clear` | Clears every dimension, keeping the selected period |
| `[f] filters` | Shown instead of the strip on a narrow viewport (below 72 columns). `n` cycles the open dimension |

The **period is a view choice**, not a filter: it changes the window and the bucket width (1 h → 5-minute buckets, 24 h → hourly, 7 d → 6-hour, 30 d → daily), so it never makes `clear` necessary. The six **dimensions are filters**; each one narrows every widget that carries that dimension.

Filter popups only offer values the installation actually recorded (`profile_stats::filter_options`), so the strip never proposes a value that selects nothing. A changed filter drops the cached snapshot and the `see more` pages.

## Metric dictionary

Aggregation rules that hold for every row:

- **Weighted means are total sum / total count.** A pre-aggregated average is never averaged again.
- **Quantiles come from raw samples** while a cohort is still raw, and otherwise from the mergeable duration bins (`telemetry::DURATION_BINS_MS`) — never from averaged percentiles.
- **`N = 0` is N/A.** Every `Option<f64>` returns `None`, never `0.0`.
- **Below 20 observations** p95 is suppressed while p50, the mean and `n` stay visible; the UI labels the small sample.
- **Trend points are never interpolated.** A bucket without data keeps zero counts and `None` durations.
- `wait_ms` is stage capacity/retry wait. It is **not** queue residence, so it never feeds the queue-delay widget.

| Metric | Denominator, exclusions and shape |
| --- | --- |
| Intel volume | Distinct canonical article ids first seen in the window. Articles are counted once; tag membership is counted per tag, so **tag totals may exceed distinct articles** |
| Initial confidence | Mean raw pre-adjustment claim confidence at first extraction, over articles that carry one. Never reconstructed from the adjusted score. Missing history is N/A |
| Brief rating | Per article, mean current claim confidence; the dashboard mean weights each article once. No-claim briefs are excluded and reported as coverage, never scored zero |
| Confidence distribution | Paired cohort: only articles with both observations. Bands are `[0,.2)`, `[.2,.4)`, `[.4,.6)`, `[.6,.8)`, `[.8,1]`; a score of exactly 1 lands in the last band |
| Origin / publisher share | Distinct articles by collection origin or source domain. `Other` is the remainder past the top 10; `Unknown` stays a visible category. Top-3 concentration is identical on every row |
| Article freshness | First ingestion minus published time, not time since the dashboard opened. Missing and future timestamps are reported separately, never bucketed |
| Intel report outcomes | The **latest terminal revision per `(article_id, report_mode)`** is the default view; attempts stay a separate view. Waiting and blocked are current counts, not completions. Cancellations stay out of every column and out of `n`. Durations use terminal revisions only |
| Recon run outcomes | Mutually exclusive terminal states. Completed with evidence is a different outcome from completed zero evidence |
| Stage durations | Mean execution and mean wait per (stage, mode). A stage retry is a separate attempt, not an extra run; `n` counts duration observations |
| Recall usefulness | Candidate-to-accepted retention = queries with ≥1 accepted hit / queries with ≥1 candidate hit. Accepted per run = accepted memories / completed-with-evidence runs |
| Workload per run | Per-run averages over runs in the window. Categories are categorical: they are counted and listed, never averaged as names |
| Tool diversity | Rates over scopes with ≥2 eligible tools; scopes below that are marked ineligible rather than scored zero |
| Directive resolution | Recorded terminal assessments among terminal runs. `unknown` is a category; an answer is never inferred from run completion |
| Temperature shifts | Latest eligible snapshot versus the preceding comparable one under the same scoring version and eligible collection scope. New, missing or method-incomparable origins are labelled and shown separately, never given an invented zero baseline |
| Tier shares | Shares across captured origin-cycle snapshots. Tier is ordinal, so the mean is descriptive only. Temperature is Argos's own score |
| Discovery mix | One mutually exclusive disposition per candidate occurrence; pending and unknown are shown separately |
| Pipeline backlog | The latest transition per (stage, unit) is that unit's current state, so the counts describe the end of the window. Each row carries the unit label its writer recorded, and units from different stages are never summed. The completed-in-period column is registered but not populated yet |
| Model sends / requests by role | Actual wire attempts. A logical operation may hold several, so sends are not jobs |
| Model latency | Send-to-contract-completion duration of successful finished attempts. First header and first content are separate drill-down fields; queue delay stays in its own widget |
| Queue delay | Measured enqueue-to-send delay only, with N |
| Attempt error % | **Failed finished attempts / all finished attempts.** An unfinished send is never a failure |
| Final operation failure % | Terminal operations counted once, attributed to the primary route cohort; the effective route is a separate field so a fallback does not double-count |
| Fallback triggers and recovery | A **trigger is not a recovery**: they are separate columns, and recovery % = recovered / triggered |
| Retry amplification | Sends / terminal operations per completion bucket, including every route of each operation even when a send predates the window; in-flight operations stay separate |
| Failure categories | Each failed attempt is classified once. The percentage is of all finished attempts |
| Tool usage | Logical invocations with the remote / local / cache split. **A cache hit is not a remote request** |
| Tool reliability | Cache-hit % and verified-zero % are over logical invocations; the error % denominator is **remote invocations only**, excluding cache and local-only executions |
| Named search health | Usable per fetch = valid SERPs / fetches. Engine identity is separate from the transport provider that fetched the SERP |
| Evidence contribution | Acceptance % = invocations yielding ≥1 accepted evidence item / successful nonempty invocations. One evidence item may credit several tools, so the rows are nonadditive |
| Tool failure causes | One terminal cause per invocation, ranked, with share of finished invocations. Attempt-level errors are drill-down, not a second count |
| Provider capacity | Live sends in a rolling 60 s, effective pace, active, queued and cooldown. Always live: it does not pretend to honour a historical filter |

A **failed transport is not zero results**. `verified_zero` means the engine or provider explicitly reported zero; every other non-success is a distinct failure cause.

## Retention and coverage

| Window | Value | Source |
| --- | --- | --- |
| Raw operational events | 90 days | `telemetry::RAW_RETENTION_DAYS` |
| Hourly and daily rollups | 365 days | `telemetry::ROLLUP_RETENTION_DAYS` |
| Prune sweep interval | 15 minutes | `telemetry::PRUNE_INTERVAL_SECS` |

Counts and histograms merge `telemetry_events` with `telemetry_hourly` / `telemetry_daily`. A rollup bucket is read only when the whole bucket predates the raw retention boundary, so nothing is counted twice; at most one bucket of history is lost at that seam.

`observed_since` is the earliest moment this installation recorded anything still retained — the oldest raw event and the oldest rollup bucket, whichever came first. A fresh installation reports an empty string rather than an invented epoch. **A metric before `observed_since` is N/A, not zero**, and the dashboard marks coverage wherever historical facts are missing: legacy records lacking a link or an assessment show unavailable coverage, not zero.

## The portable configuration contract

Profile > System > **Configs** exports and imports the schema-v1 document. The full JSON Schema is `docs/schemas/profile-config-v1.schema.json`; the DTOs are `crates/argos-osint-core/src/config_transfer.rs`. `SettingsFile` is never exposed wholesale: only the documented fields travel, so unrelated settings, recon limits and local state cannot be overwritten by a foreign document.

Root fields: `schema_version` (integer `1`), `providers`, `tool_credentials`, `model_roles`, `rate_limits`. `exported_at` is an optional informational RFC3339 timestamp and is never used to merge.

| Field | Shape |
| --- | --- |
| `providers[]` | `{id, kind, base_url, default_model, credential, quota_group_id}`. Every configured model account, including keyless local routes |
| `tool_credentials[]` | `{provider, primary, fallback}` with credential objects or explicit null |
| `model_roles` | The ten role keys plus optional `decision_fallback` |
| `rate_limits[]` | `{quota_group_id, scope, verified_rpm, verified_tpm, verified_rpd, local_rpm, concurrency, source, verified_at}` |

**Credential** is a tagged object in exactly one of three shapes:

| `source` | Carries | Meaning |
| --- | --- | --- |
| `inline` | `api_key` | A saved key copied in the document |
| `env` | `name` | An environment variable resolved at runtime |
| `none` | — | A keyless route |

Inline saved keys round-trip. An environment credential exports a **reference by default**, preserving the precedence rule without copying environment secrets. Precedence on import: a saved key overrides the environment, so an inline or resolvable `env` credential clears the local saved key, and `none` means no account is configured. A blank `api_key` is rejected — it is not a masked placeholder.

**Keyed tool providers** (nine, in `provider::KEY_ENV` order): `firecrawl`, `hunter`, `sociavault`, `newsapi`, `courtlistener`, `gnews`, `newsdata`, `currents`, `whoxy`. **Holehe is keyless and is never listed** — it must not gain a fabricated credential field. The list is registry-backed, so a future tool provider cannot be silently omitted.

**Roles**: `recon`, `synthesis`, `tool_picker`, `classifier`, `summarization`, `evidence_curator`, `entity_resolver`, `claim_assessor`, `investigation_controller`, `decision_model`, plus the optional nullable `decision_fallback`. Each assignment is `{primary: route|null, fallbacks: [route…]}` with route `{provider_id, model}`. A null primary preserves documented inheritance (Tool picker for the decision roles, Recon for evidence curator and investigation controller, Classifier for entity resolver, Synthesis for summarization and claim assessment). Fallback order is priority; duplicates are rejected.

Never exported: ephemeral counters, cooldowns, queues, databases and host paths. An exported subscription route is marked `subscription` and carries no key material — it indicates reauthorization is required on the importing machine.

## Import semantics

Import is a **merge by stable id** — provider, tool, role and quota-group id are the join keys.

| What the document does | Effect |
| --- | --- |
| Mentions a field with a value | Replaces the local value |
| Mentions a field with explicit `null` | Clears it back to documented inheritance or removal |
| Omits the field entirely | Preserves the local value |
| Omits a `primary` / `fallback` slot on a tool credential | Preserves the local account for that slot |

Merge scopes: quota settings merge by `(quota_group_id, scope)`; groups absent from the document are preserved untouched. `decision_fallback` absent or null means none configured.

An unresolved `env` reference **keeps the local account**: importing it would silently install an empty key, so the route stays as it was and the import reports a warning that names the variable. Model ids absent from a stale remote catalog are allowed, and validation never issues network calls.

The Import screen shows a redacted change summary before the final action — which areas, ids and fields move, never a credential value. Editing the buffer disarms the Import button, so a document that no longer matches what was validated can never commit.

## Validation

The whole document is validated before anything is written, twice: once by walking the parsed JSON (so every error carries a JSON pointer) and once against the typed DTOs, so a hand-built document cannot skip the JSON checks. A duplicate object key anywhere is rejected, because a `serde_json::Map` silently keeps the last of a repeated key. A document over 1 MiB (`config_transfer::MAX_DOCUMENT_BYTES`) is refused before parsing.

Rejected:

- a root that is not a JSON object; unknown or missing root fields
- a `schema_version` that is not the integer `1`; a malformed `exported_at`
- duplicate object keys; any unknown field at any level
- empty ids, kinds, models, routes or provider names; duplicate provider, tool-credential, or `(quota_group_id, scope)` entries
- a provider `kind` outside `openrouter`, `google`, `nvidia`, `grok`, `openai`, `local`, `subscription`
- a `base_url` that is not an absolute URL with a host, or is not `https` unless the kind is a documented local route
- a subscription route carrying key material; an `inline` credential with a blank key or a stray env name; an `env` credential that is not a valid variable name or that also carries a key; a `none` credential carrying any value
- a `quota_group_id` that references a group `rate_limits` does not define
- `verified_rpm` / `verified_tpm` / `verified_rpd` that are not positive integers, `local_rpm` that is not a non-negative integer (an explicit `0` is the documented disabled quota), `concurrency` outside 1–64, a `source` outside `provider_docs` / `probe` / `desk_assumption` / `unset`, a malformed `verified_at`
- a tool provider outside the nine keyed providers; a `primary` / `fallback` that is neither a credential object nor null
- a missing or unknown role key; a role assignment that is not an object; duplicate fallback routes; a route whose `provider_id` or `model` is empty
- role incompatibility: a decisions model cannot serve a general synthesis role, and a decision role needs a decisions model
- a route referencing an unknown provider that is neither in the document nor a known preset

Warnings, which do not block: a keyless non-local, non-subscription route; an `env` credential whose variable is not set here. Error messages name a field path and an expected shape, never a value from the document, and the editor bounds each message to 120 characters.

## The commit

`config.toml`, `auth.json` and `quota.json` are one configuration. A caller that changes a provider either moves all three or none of them, so ordinary sequential writes are not enough.

`config_transfer::commit_profile_config` takes `crate::config_commit::ConfigLock` and lands the three files as **one all-or-nothing revision**:

- The lock is an app-wide `create_new` lockfile (`.argos-config.lock`) with a heartbeat and a stale takeover, so it serialises writers across processes without a new dependency.
- A sibling journal (`.argos-config-journal.json`) records paths and state only — never a body and never key material.
- Each file is written through `config_commit::write_secure`: a unique sibling temporary file created **owner-only (0600) at creation**, written, flushed, fsynced and atomically renamed. A temporary file never survives an error path.
- Any single failure rolls the whole batch back, restores the previous files and leaves a recoverable error state. **A failed import writes nothing.**
- A successful commit advances a process-wide generation id and the reload epoch (`config_commit::subscribe_reload`), which is the channel a running app watches to reload. No TUI surface subscribes to it yet.
- A process that dies mid-commit leaves `state: "pending"` behind. `config_commit::recover_at_startup()` runs at TUI boot and on CLI start and finishes the rollback before anything else opens the store.

Export uses the same secure primitive: the document is serialised deterministically, then written to a unique owner-only temporary file and atomically renamed into place. Destination errors name the path, never a credential.

## Named search engines

`firecrawl_google_search`, `firecrawl_yandex_search` and `firecrawl_mojeek_search` all send one Firecrawl `/v2/scrape` request for an engine SERP and then run local HTML selectors. `crates/argos-osint-core/src/osint/search_engines.rs` owns the truthful outcome contract.

`SerpOutcome` has eight variants. Only `Valid` and `VerifiedZero` are successes.

| Variant | Meaning |
| --- | --- |
| `Valid` | At least one accepted organic card |
| `VerifiedZero` | A recognized engine status region with an explicit supported no-results phrase and zero accepted cards |
| `Challenge` | A real challenge interstitial (captcha / sorry form or widget) |
| `Consent` | A real consent interstitial |
| `RateLimited` | Target status 429, or an explicit rate-limit phrase in a status region |
| `ParserMismatch` | Page present but nothing recognized: unknown DOM, links-only, or missing HTML |
| `UpstreamFailure` | Provider or envelope failure: `success == false`, a non-2xx/3xx target status, HTTP 404, or an explicit provider error |
| `ResponseTooLarge` | Input bytes exceeded 8 MiB, or the envelope says truncated |

**Only `VerifiedZero` becomes `no_results`** (`osint::no_results`). An **unknown layout is a parser mismatch, not a zero**: the old path returned success with an empty item list for unrecognised HTML, which the executor then displayed as no results. `ParserMismatch`, `UpstreamFailure` and `ResponseTooLarge` map to a failed status with a typed reason, `Challenge` and `Consent` map to blocked, and `RateLimited` maps to `rate_limited`. Empty arrays alone never prove zero.

Interstitial detection is **structural**, never a whole-document substring scan:

- a form whose `action` points at a captcha or `/sorry` wall is a challenge; one that points at a consent wall is a consent wall
- an iframe or anchor whose host is a consent or captcha host
- a recaptcha widget element (`.g-recaptcha`, `#recaptcha`, and class-based variants)
- a `meta http-equiv="refresh"` whose target is a consent host, **including one wrapped in `<noscript>`** — the European-Google wall hides the tag there, and `html5ever` parses `<noscript>` as raw text, so reading the text node directly is the only way to see it. It is still structural evidence, so it stays a consent wall rather than a parser problem

No-results phrases are matched only inside a recognized engine status region, with word boundaries, so `0 results` matches a real zero but not `10 results` or `1,000 results`. Recovery is bounded: only a `ParserMismatch` with a DOM actually present allows one fresh same-engine fetch, never a challenge.

**Engine identity is separate from the transport provider.** Firecrawl is the transport that fetches the SERP; `google` / `yandex` / `mojeek` are the engines. `search_engines::engine_family` groups the three named tools under `named_serp`, while `firecrawl_search` keeps its own family so its hits are never relabelled as a named engine. Cache identity carries the parser version, the fetch contract version, the canonical query, locale and limit, so a parser upgrade invalidates legacy v1 entries without deleting unrelated caches.

## Provider orchestration companion

Limits, shared admission, fair queuing and retry/fallback policy are owned by `ARGOS_PROVIDER_REQUEST_ORCHESTRATION_SPEC.md`. Argos is a **read-only consumer**: `crates/argos-osint-core/src/provider_metrics.rs` publishes the snapshot DTOs the Models widgets render and the quota-setting DTOs the portable configuration shares, and it contains no scheduler, no admission policy and no retry policy.

Until the companion publishes a snapshot, `provider_metrics::capacity_snapshot` returns `available == false`. Live capacity and queue delay then render **unavailable, never zero** — `models.capacity` says the companion has not published a snapshot, and the status strip shows the queued count as N/A. A missing companion metric is never read as a measured zero.
