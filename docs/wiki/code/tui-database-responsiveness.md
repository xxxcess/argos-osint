# TUI database and index responsiveness

[Code route](index.md) · [Wiki](../../index.md)

The terminal event loop renders cached state and handles navigation while bulk
database operations run on workers. These changes address an observed 84,334 ms
Brain browse load over 434 display rows. That live database changed before a
second measurement, so it is not a valid before/after benchmark. The regression
cohort fixes 150 article occurrences and 250 Actor-name groups instead.

| Audited path | Execution and contention handling |
| --- | --- |
| Brain list entry, change watcher, Atlas/Recon/Intel completion | One read-only snapshot in flight; one newest invalidation is retained. Hidden Brain only marks the cache dirty. Find/kind filtering uses cached rows. |
| Brain Actor totals, graph and Related tabs | `storage/browse.rs` and `storage/browse_assoc.rs` load Claims and retained associations through SurrealDB repository queries. Detail uses one association snapshot for graph, totals and tabs. |
| Brain detail, Back, list anchors | Background readers; destination/request guards drop late navigation and preserve Back state. Errors retain the current view. Deleted Back destinations are skipped. |
| Brain topology snapshot and scope worker | Background snapshot load via `Store::open_light`; `storage/brain_scope.rs` drains the `POOL_BRAIN_SCOPE` pool, persists typed inference/link memories through the shared publisher, and defers vector work to the index outbox. |
| Intel bulletin, brief polling, Related scope/expansion | Background snapshots keyed by article occurrence, scope, job and requested page size. Polling preserves Related selection and expanded pages. |
| Atlas history, retention and cycle deletion | Background history/pruning and deletion; headline navigation fetches one article instead of decoding the retained corpus. Duplicate delete requests are rejected while pending. Completion updates caches after commit. |
| Recon lists, transcript/tool/run history, recalled-memory popup and investigation deletion | Background reads/deletion with selection and pending-refresh guards; recalled-memory summaries arrive in the transcript snapshot, and historical tool-result log formatting and writes stay on the reader worker. |
| Tools manual history | Background read, retaining a selected historical call. |
| Jobs and Logs | Background bounded pages/counts/detail and direct job-source navigation, keyed by filters and selection. Old-job cleanup and durable event clearing run on workers; duplicate cleanup requests are rejected and failed clearing retains the current view. Fold, follow and scroll stay local. |
| Profile analytics | Existing background snapshot reader retained. |
| Rendering and list arrows | Cached presentation; no vector search or graph lookup during Brain list movement. |
| Interactive point lookups and small edits | `Store::open_ui` returns a handle to the shared SurrealDB runtime. Reported writes preserve their error path; failed draft flushes remain dirty for retry. |
| Startup | Schema initialization remains required; bulk Brain/history loads and backfill are deferred. The UI Store shares the process SurrealDB runtime. |
| Index/background maintenance | Durable outbox and elected workers retain indexing ownership. Index work uses the shared SurrealDB repository. Lightweight Store handles do not create separate database connections; schema bootstrap belongs to runtime initialization. |

Background readers deliver immutable UI snapshots. Topology repository reads are not one database snapshot transaction; delivery fences prevent stale UI replacement, not mixed database revisions. A pump handles at most 32 work events and yields after its 8 ms processing budget, so background bursts leave time for keyboard/mouse handling.
There is at most one read per surface, and an invalidation during a read schedules
a follow-up instead of creating one thread per event. Small control writes remain
atomic SurrealDB operations; they do not run embeddings inline. No provider defaults,
credentials, user database reset or schema version change is part of this fix.

Google native schema requests use
`generationConfig.responseFormat.text.mimeType = "APPLICATION_JSON"`, the enum
specified by the [GenerateContent reference](https://ai.google.dev/api/generate-content#TextResponseFormat).
The legacy `responseMimeType` JSON-mode string stays `application/json`. This
corrects the observed HTTP 400 fallback failure; external server errors and token
limits still follow the existing bounded route policy.

Calendar-month false positives are excluded from Actors and the two-entity Related
threshold while proper names are retained. Optional metadata failures remain
warnings, with bounded parser flags persisted for diagnosis. The original failed
article cohort was removed externally before replay; its precise parser cause
cannot be reconstructed from the older status-only receipts.

Validation and visual review are recorded in the
[verification ledger](../../archive/tui-verification-responsiveness-2026-10-14.md).

Jobs filter changes clear the displayed old cohort and pending owner lookup before requesting a new asynchronous snapshot. An older in-flight completion is fenced by module/filter/selection key and queues the latest scope; it cannot restore prior rows or filters. Same-filter refresh keeps retained data.
