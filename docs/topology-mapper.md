# Seeded topology mapper

The approved Graph first design opens a full relationship mapper from an exact
Brain or Intel record. Its default neighborhood contains two semantic hops of
persisted data. The canvas, inspector, relationship records, receipts and counts
consume one immutable snapshot. Production loading, empty and error states never
display a sample graph.

## Entry identity and return

| Entry | Seed |
| --- | --- |
| Brain claim-path graph | The open canonical Claim ID. Activate the graph pane with Enter or a click. |
| Brain actor-group graph | Exact group name and retained member IDs; a display group does not merge canonical identities. |
| Intel Focus Brief claim | Exact article, run, revision, publication job and snapshot-local claim identity. Resolve through its matching publication/derivation. |
| Intel Focus Brief inference | Exact article, run, revision, publication job and snapshot-local inference identity; resolves to its current canonical Inference independently of Links completion. |
| Intel Focus Brief actor | Exact article metadata revision and scoped actor reference. Metadata-only actors remain actionable without extracted claims. |

Intel entries preserve typed records through formatting and measure the whole
visible wrapped record for focus and pointer targets. Display labels are never
parsed as IDs. Missing canonical publication retains an exact source projection
and reports its unresolved state, preserving Claim versus Inference type; opening topology creates no memories and runs
no models. Relational data remains usable while vector indexing is pending.

Selection changes the inspector. Recenter, including an edge endpoint action,
changes the seed and pushes topology history. Back first restores that history,
then the originating app, record, revision, page, scroll and focus. History retains
stable node/edge identity, filters, panel tabs, viewport transform and independent
inspector/relationship offsets. A changed or deleted origin gives a notice and
returns to a safe owner view. Esc dismisses an overlay before navigating back.

## Graph first components

Use `GraphCanvas`, `TopologyInspector`, `ListGeometry` / `ScrollPane`, `TabBar`
and measured action controls. `topology_layout::TopologyLayoutResult` owns pure
panel geometry; graph placement supplies the same clipped node/edge rectangles
to drawing, keyboard focus and pointer targets. Draw performs no database,
network or model work. Preserve the shared theme colors and semantic kind styles.

Dimensions below refer to the available body after the shared shell:

| Available body | Layout |
| --- | --- |
| At least 132 columns and 32 rows | Canvas left 65%, inspector/relationships right 35%. |
| 100–131 columns and at least 32 rows | Canvas left 70%, inspector/relationships right 30%, compact actions. |
| At least 60 columns and 18 rows, below the split threshold | Graph / Details / Controls pages with retained state. |
| Smaller | Reachable Back and a minimum-size notice. |

Back, title, origin, seed, depth 1/2/3, retained/current-cycle scope and snapshot
counts sit above the body. The graph uses deterministic hop columns, a seed
marker independent of selection, cell-measured labels, directed connectors and a
semantic legend. Crowded neighborhoods have a pannable extent. Zoom, pan, center
and Reset view change actual geometry. No decorative minimap is shown.

The inspector exposes full selected text, kind, hop, revision, provenance and
owner/recenter actions. Edge inspection exposes true source → predicate → target,
rationale, epistemic state, exact receipts and both endpoint actions. Related
similarity remains dashed and labelled potential context. Relationship records
retain the repository's normal blank-row separation and independent scrolling.
Keyboard and pointer navigation share visibility/filter rules; Tab focus remains
independent of activation.

## Snapshot and provenance contract

`Store::topology_snapshot` in `brain_topology.rs` delegates bounded query work to
`brain_topology_query.rs`. One SQLite read transaction supplies the snapshot and
authoritative `Store::memories_changed_seq`.

| Presentation | Recorded basis |
| --- | --- |
| Actor | Exact canonical actor and assessed redirects; no same-name union. |
| ActorGroup | Explicit members and recorded relationship binding receipts; grouping does not assess identity. |
| Claim | Canonical Claim or exact retained publication projection. Extraction support does not establish world truth. |
| Inference | Canonical Inference with retained derivation/premise receipts, reasoning and limitations. |
| Topic | Derived `insight_claims.topic` and Atlas category attributes, with normalized key and original receipts. There is no canonical Topic memory kind. |
| Article / Source | Exact occurrence/revision and provenance or evidence receipts. A publisher does not establish independent corroboration. |
| Link | A selectable directed edge retaining its memory identity, relation, rationale and receipts. |
| Related | Tentative adjacency with retained basis/revisions and verification gap; it is never required for direct mention/provenance edges. |

Discover neighbors in both endpoint directions while retaining each relationship's
stable direction. Traverse typed actor references, mentions, Links/Related,
inference premises, derivations and evidence. Canonical publication persists actor bindings on Claims and premise-derived actor associations on Inferences, retaining their kind and provenance. Each stage enqueues changed canonical records through the existing index outbox; new canonical premise writes use indexed relational evidence references for topology adjacency, which remains available while vector acknowledgments are pending. Historical rows without canonical relational receipts retain typed-JSON/local-ID fallback, resolved against the original carried snapshot and source revision. Resolve snapshot-local IDs only
inside their exact publication. Validate current artifact/URL/time identity
before attributing historical content to a current article. Group membership
retains member attribution and does not assert identity. Cycle scope follows
recorded origin joins.

Ranking is deterministic: traversal orders direct provenance/premise edges before
tentative leads. Final node admission protects the root and explanatory paths,
then orders by hop, predecessor, kind label and stable typed key. This bounded
selection does not guarantee every Source wins admission over other kinds.
Bound adjacency reads and candidate exploration as well as output. Every
edge endpoint and every retained predecessor path must exist in the snapshot.
Report visible/discovered counts, omitted nodes/edges, unresolved endpoints and
whether exploration is exhausted. Bounded discovered counts are not database
totals. Continuation is deferred: reject input tokens and emit no cursor.

One background read at a time coalesces newer requests. Generation and typed
seed/options fences reject obsolete completion, including same-seed option
changes. Accept snapshots atomically and preserve stable selection/focus on
reorder. Initial/new-query loading displays no old graph. A matching prior
snapshot can remain visible during refresh or after a read error with explicit
stale/error status. An isolated valid root has one node and zero edges; a missing
record is a separate state. Unknown coverage is unavailable rather than zero.

The [neighborhood summary](#neighborhood-summary) reads this snapshot but never writes canonical data. Scope analysis remains an explicit separate action. Verified canonical Claims
supply inference premises; canonical Actor references are revalidated. Links may
use canonical Claim/Actor/Inference or exact Article/Source endpoints. Retained
Claim/Inference/Actor projections are browse data, not canonical memory endpoints. This change does not modify Atlas
classification-only collection, independent Intel/Recon publication, provider
policy or schema.

## Neighborhood summary

Each seed shows a deterministic **neighborhood brief** immediately, then
automatically generates a better explanation with the configured Summarization
role and atomically replaces the fallback only after the complete answer
validates. Any failure keeps the fallback. No schema migration, canonical memory,
relationship, extraction or index write accompanies it; summary cache, task and
log rows are the only writes.

### Flow

1. **Fallback and frozen input.** Inside the topology read transaction
   (`brain_neighborhood_brief.rs`), bounded typed payloads (at most
   `MAX_PAYLOAD_READS` = 160) for the admitted nodes and edges yield an immutable
   `NeighborhoodBrief` and a typed `NeighborhoodInput` packet. The snapshot owns
   both as defaulted optional fields. Semantics come from typed payloads and edge
   kinds, never from parsing labels or IDs. The transaction is released before any
   cache write, enqueue or network work.
2. **Queued generation.** `brain_neighborhood_brief_exec.rs` publishes the
   fallback through `summarization::publish_deterministic_and_enqueue`
   (`SummarizationMode::TopologyNeighborhood`, operation `topology_neighborhood`,
   `POOL_SUMMARY`). The request is deduplicated on its identity (task
   `sum-task-<cache key>`), so reopen and refresh do not duplicate work. The worker
   holds a renewed lease and runs the Summarization role chain through
   `model_exec::execute_chat` with `ResponseFormat::GeneratedJson` and the
   Summarization attempt cap and timeout profile. The chain owns provider
   retries and fallbacks; the queue never replays an exhausted chain.
   Decisions-only and unconfigured routes are skipped without dispatch.
3. **Validate, then replace.** Output is buffered until complete, then checked for
   schema, meaningful content, known typed references, reference-kind and
   epistemic compatibility, relation direction, preserved qualifications and
   unsupported assertions. Host code owns seed, keys, counts, coverage and required
   caveats. Only a valid result is cached (with the actual provider/model route)
   and delivered. Empty, malformed, truncated, rejected, blocked, cancelled or
   storage-failed outcomes complete the task with a typed failure
   (`topology_failed:<kind>`), so the fallback stays and the indicator stops.
4. **Delivery.** A background watcher polls cache/task state every 750 ms with one
   read in flight; it never runs a model request and never runs during drawing.
   `WorkEvent::TopologySummaryLoaded` carries the summary generation, typed seed,
   options and `basis_key`. The app drops it unless all still match the open view,
   assigns the result in one step (text and typed references together) and never
   moves focus. Obsolete watchers are detached on new seed, Back, a cleared
   snapshot and every new begin.

### Immutable input ownership

The snapshot read owns the brief and packet; the UI and executor only borrow
clones. Selection, display filters, pan and zoom never change them. Recenter,
depth, cycle scope, changed data or a changed route/model configuration produce a
new `basis_key` and request. The generated brief is accepted only for the same
`basis_key` as the fallback on screen.

### Cache identity

`SummaryRequest` identity is `mode`, source `topology:<seed key>` with the
`basis_key` as revision and a hash of the packet JSON, focus, budget
(`PACKET_BYTES_MAX` = 36 000), prompt and output versions, and
`provider = "role-chain"`, `model = route fingerprint`. The fingerprint hashes the
ordered Summarization routes (provider, account, model, endpoint host, skip
state) and contains no secrets. `basis_key` hashes seed, options, admitted
content, hydrated revisions and content, coverage and `FORMAT_VERSION`. Changing
any of these resolves a fresh request; an exact valid generated hit displays
immediately. Results live in `argos_derived_summaries`, separate from the
deterministic local identity and from `memory_graph_summaries`. Reopening an
exhausted request keeps its fallback until explicit **Retry summary**, which
resets that same request in place and never blanks content.

### Content and mapping

Target 120–180 words, fewer when sparse. Blocks: Overview, Connections (at most
two), Interpretation (at most one existing inference with its limitations or a
tentative lead with its gap) and Limits; unsupported blocks are omitted entirely.
Header `<seed> · Neighborhood summary`; footer
`Within <depth> hops · <cycle scope> · <coverage>`. Argos has no confidence
rating, origin groups, source reliability, valid time or likelihood, so none is
shown. See the [plan](topology-scope-summary-plan.md) for the concept-to-record
mapping.

### Summary and Record tabs

The inspector has **Summary** and **Record** content tabs. Summary is the default
for a new seed and renders the brief as markdown with typed basis references.
Record keeps selection details and actions; the Relationships panel keeps its
separate All/Articles tabs. Activating a node, edge, related row or basis
reference (click or Enter) opens it in Record without recentering or changing
filters. Focus movement and arrow navigation alone never switch tabs. Summary
scrolls independently by wheel, PgUp/PgDn, arrows and Home/End with an
`▲▼ rows a–b of n` indicator. History restores the tab and scroll. Compact
Details keeps both tabs; below the minimum, Back remains.

State label: `Summary · generating…`, `generated · <route>`, `blocked`
(no usable model; Retry summary, View logs, Models), `rejected`/`failed`
(Retry summary, View logs), `stale · refresh failed` or
`stale · loading the new scope`, `limited coverage`, `isolated`,
`seed not found`, `recorded brief`. A new query shows loading, then its fallback;
a same-query refresh or error keeps graph and brief together with stale labeling.

### Summary sizing

The details panel takes 75% of the inspector height (minimum 8 rows), always
leaving the Relationships panel at least 5 rows; Record keeps 55% (minimum 7).
Inside it: a 1-row tab bar, a 1–2 row wrapped state label, the scroll content
(registered as the pointer and focus target), a 1-row indicator only on overflow
and a 1-row button row only on failure. Measured panel heights including borders:
160×50 → 32, 120×40 → 24, 100×32 → 18, 80×24 → 12, 60×20 → 8. At 60×20 only two
content rows remain; it is reachable and scrollable but cramped. 60×18 and
40×20 are below the mapper minimum and show the size notice with Back.

## Requested list density

Keep `LIST_ITEM_GAP_ROWS = 1` and existing `ListGeometry` constructors' defaults.
Explicit `with_gap` / `fixed_with_gap` / `capacity_with_gap` calls use zero gap
only for:

- Home application items;
- Ctrl+K commands, with nonselectable section headings preserved;
- Atlas headline feeds, cycle lists and retained article lists;
- Intel default category headlines, preserving all six category tabs.

Drawing, extent, capacity, reveal, offsets and pointer mapping use the same dense
geometry. Wrapped content remains contiguous. Brain, topology relationships,
Focus Brief Related, Recon, Tools, Models, Jobs, Logs and Profile retain their
existing spacing. The Profile Dashboard/Report layouts, 20 primary IDs, metric
semantics/history, Configs/System and portable configuration are unchanged.

## Rust API compatibility

Topology's public node/snapshot structs have new fields, and its seed/node-kind
enums have new variants. Rust callers using struct literals must initialize the
new fields; exhaustive matches must handle the new variants. Older serialized
snapshot fields remain readable through serde defaults, but readers must accept
the new presentation kinds.

Actor entries now preserve exact canonical identity; use an explicit ActorGroup
seed when aggregation is intended. Article keys include revision and ActorGroup
keys include membership: rebuild cached topology keys rather than parsing or
reusing old strings. Default depth is two; set `depth: 1` explicitly for the prior
scope. Continuation is unsupported and supplied tokens now return an error;
remove them and inspect bounded coverage fields instead. No database migration
is required.

## Verification and ownership

Core owns typed resolution/traversal and scope eligibility. The TUI app owns
request generations, typed entries, origin/history and actions. Layout owns pure
geometry; renderers consume immutable snapshots. Shared density helpers preserve
spaced defaults. See the [architecture figure](diagrams/brain-topology.html).

The offline gate is fmt → strict Clippy → workspace tests through the
[agent Cargo wrapper](../AGENTS.md#agent-cargo-isolation), with locked dependencies,
no default features and `ARGOS_EMBED` unset. Core fixtures cover disconnected
seeds, depth boundaries, local-ID collisions, revisions, namesakes, direct
provenance, inference/topic relations, source identity, budgets and path closure.
Interaction fixtures cover generation races, selection/recenter, stable focus,
origin restoration, filters, compact pages and keyboard/pointer parity.

Use fresh actual TestBackend captures per [TUI verification](tui-verification.md):
`dump_brain_topology_screens` / `topology-*` and `topology-summary-*` (committed in `screens/topology-summary/`),
`dump_topology_density_and_entries_screens` / `density-*` and `intel-topology-*`,
and ignored `dump_brain_kind_screens` / `brain-actor-graph-*` and
`brain-claim-split-*`. Review 160×50, 120×40, 100×32, 80×24, 60×20, 60×18 and 40×20,
plus 132/131-column and 100×34/100×33 height split boundaries. Include selected edges, controls, crowded,
isolated, loading, stale and error states. Record image observations separately
from interaction and data results; generation alone does not complete review.
