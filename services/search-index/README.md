# Private actor search index

`fleetworks-search-index` is a Linux backend service. It receives bounded,
authenticated projections from the durable ingestion coordinator and answers
bounded internal actor-discovery queries. SurrealDB remains authoritative. The
desktop and browser access discovery through the gateway; neither receives this
service's origin or credentials.

The existing NativeWorker owns durable ingestion and explicit rebuild Jobs. Source
transactions record projection intent, the gateway filters current canonical
eligibility, and deployment assets provide a separate private image and volume.
Executed evidence and remaining cutover gates are tracked in the
[cutover acceptance tracker](../../.planning/2026-11-11-rune-macos-cutover/acceptance.md).
Host fixtures do not establish Linux deployment performance or both architectures.

## Dependency and persistence contract

The selected engine is **Tantivy 0.27.0**, exactly pinned, with only `mmap` and
`lz4-compression`. Its [published package manifest](https://docs.rs/crate/tantivy/0.27.0/source/Cargo.toml)
declares Rust 1.88 and its [index format is **7**](https://docs.rs/tantivy/0.27.0/tantivy/constant.INDEX_FORMAT_VERSION.html);
this is source evidence, not an executed compatibility result. Schema and analyzer
versions are both **1**. See the versioned [Index API](https://docs.rs/tantivy/0.27.0/tantivy/index/struct.Index.html),
[prepared commit payload](https://docs.rs/tantivy/0.27.0/tantivy/indexer/struct.PreparedCommit.html)
and [reader reload contract](https://docs.rs/tantivy/0.27.0/tantivy/struct.IndexReader.html).

The [private protocol](../../crates/fleetworks-contracts/src/actor_index.rs) has
only DTOs, validation and SHA-256 helpers; it links no engine. Mutation checksums
cover the compact concrete JSON tuple `(actor_id, revision, projection)`. Batch
content checksums exclude the coordinator lease epoch so an unchanged retry can
reconcile under a newer lease. Field ordering belongs to protocol version 1.

Each canonical actor has one document. Ineligible actors and tombstones retain
stored revision/checksum fences but are excluded from discovery. A batch validates
every mutation before changing the writer. An older revision or conflicting equal
revision is rejected; a newer batch cannot resurrect a deleted actor using stale
source data. Aliases remain separate actors where identity is ambiguous.

Documents and bounded checkpoint/source/generation/lease/replay metadata share one
Tantivy commit through `PreparedCommit::set_payload`. Success is returned only
after disk commit, explicit manual reader reload and publication of the matching
immutable serving snapshot. A lost acknowledgement replays the same batch ID.
There is no transaction across Tantivy, SurrealDB and Jobs, and no exactly-once
claim. Source workers acknowledge their own outbox only after this visible receipt.

The most recent **64 batch identities** remain in commit metadata. A replay older
than that window is rejected as `checkpoint_gap_or_expired_receipt`; it must
reconcile against durable coordinator/source checkpoints. It never silently
advances unknown work. Tombstone fences are retained with actor records; safe
tombstone reclamation remains intentionally unavailable.

## Configuration and transport

External deployment tooling supplies these settings. The app never invokes this
executable or initializes its volume. Use an owned private backend volume and
matching runtime UID/GID; native workspace folders and provider secret stores must
not be mounted here.

| Variable | Purpose |
| --- | --- |
| `FLEETWORKS_SEARCH_BIND` | Listener, default `127.0.0.1:8087`; private-network bind is an explicit operator choice |
| `FLEETWORKS_SEARCH_ALLOWED_HOSTS` | Required exact comma-separated Host authorities; no wildcard, URL or userinfo |
| `FLEETWORKS_SEARCH_TOKEN_FILE` | Required protected read credential for gateway query/status |
| `FLEETWORKS_SEARCH_WRITE_TOKEN_FILE` | Required distinct protected write credential for the ingestion/maintenance owner |
| `FLEETWORKS_SEARCH_INDEX_DIR` | Required absolute owned volume directory; no request chooses a path |
| `FLEETWORKS_SEARCH_DEPLOYMENT_ID` | Required persisted target deployment/database identity |
| `FLEETWORKS_SEARCH_SOURCE_EPOCH` | Required source-database replacement fence; distinct from user configuration reset |
| `FLEETWORKS_SEARCH_LIMITS_FILE` | Optional protected, bounded JSON for the shared `ActorIndexLimits` policy; invalid hysteresis or amplified rate/concurrency is rejected |
| `FLEETWORKS_SEARCH_WRITER_MEMORY_BYTES` | Optional override, default 64 MiB; validated memory budget shared by the single active/shadow writer |
| `FLEETWORKS_SEARCH_QUERY_CAPACITY` | Default four queries, validated one through eight |

Credential files must be regular, bounded and private to their owner on Unix.
Tokens use the existing 32–256 byte bearer syntax; only digests are retained in
configuration. Host, Origin and purpose checks run before dispatch/body admission.
Browser Origin headers are rejected. Read credentials cannot mutate the index;
write credentials can inspect status for reconciliation. No CORS or public admin
surface is provided. Forwarded peer headers do not grant authority.

| Method / private route | Contract |
| --- | --- |
| `GET /internal/v1/actors/status` | Safe schema/format/generation/checkpoint state; no rebuild side effects |
| `POST /internal/v1/actors/query` | Plain text and result limit; no engine syntax, tenant selector, offset or index path |
| `POST /internal/v1/actors/batches` | Versioned bounded projection batch; write credential and current source/generation/lease fences |
| `POST /internal/v1/actors/rebuild/start` | Explicit expected identity/generation, source scan barrier and idempotent request ID |
| `POST /internal/v1/actors/rebuild/promote` | Explicit verified replacement/catch-up barrier; never clears the serving index first |
| `POST /internal/v1/actors/rebuild/repair` | Explicit current manifest checksum/source/epoch; preserves damaged bytes and resets to a baseline requiring a complete new scan |
| `POST /internal/v1/actors/generations/retire` | Explicit inactive generation, serving generation, current manifest checksum/source/epoch and replay identity; cannot remove active or pending generations |

All routes require a credential. Error payloads contain safe stable reason codes,
not input text, tokens or filesystem paths. Missing/corrupt/incompatible storage
starts the transport in an explicit unavailable state; it is never erased or
silently recreated. Only an explicit admitted rebuild can request repair from intact owned manifest
evidence. Malformed/foreign manifests require offline operator review. Runtime
active-generation failure can require service restart before explicit repair;
readiness polling never closes writers or creates replacement storage.

## Query and write budgets

Queries use typed term/conjunction tiers: exact canonical name, exact alias,
name/alias tokens, bounded prefixes, then description. Names use NFC and Unicode
lowercasing/alphanumeric analysis without stemming or English stopword deletion.
No raw query parser, regex, fuzzy expansion or identity-resolution decision is
exposed. Canonical actor ID breaks ties within each tier across segment changes.

Inputs are bounded at 256 query bytes, 16 tokens and 25 results. Name/alias tokens
are limited to 64 characters, 128 distinct name tokens and 1,024 generated prefix
terms per actor. Invalid expansion is unresolved source work, not truncated identity
data. Query results include the serving generation/checkpoint and explicit
ready/indexing/rebuilding/stale/degraded state. The gateway must perform bounded
canonical permission/lifecycle checks before exposing even labels or snippets.
Do not return the internal projection's authority metadata to clients.

The transport admits at most eight requests, four queries by default, and **one
write before buffering its body**. Batches are at most 500 mutations or 2 MiB;
queries have a separate 4 KiB body bound. One write starts no more frequently than
every 500 ms, with no catch-up burst. Blocking indexing, commit and query work runs
outside Tokio workers; permits remain with blocking tasks after an uncertain HTTP
timeout. Queries keep using the previous immutable searcher during writes.

The deployment-wide database lease owns at most two persisted batch bodies and
one request in flight. A rebuild can retain one old live body as recovery evidence
while preparing its shadow body. Source
reads use at most 128 actors per page to bound decoded alias provenance separately
from the serialized 500-document/2 MiB maximum. A partial live tail flushes after
the configured interval (default one second) or source scan end. Persisted start
times cover live, shadow and repair commands across replicas/restarts.

Linux RSS and filesystem-free-byte measurements gate service writes with high/low
hysteresis. Query/commit latency uses measured EWMA and a bounded cooldown probe.
Default memory pause/resume is 384/320 MiB, disk free pause/resume is 256/512 MiB,
and source backlog pause/resume is 500,000/250,000 events. Missing service
measurements remain explicit; stale observations pause bulk source work. Seed
pauses renew the existing source ownership lease and observe cancellation at
committed chunk boundaries. Interactive Tools have a separate bounded worker lane;
seed and Natural training share one process bulk slot. Database resource fences
remain authoritative across replicas.

NativeWorker also measures its own RSS and workspace free disk before starting
bulk setup. Unknown/high pressure waits with cooldown while Tools remains on its
independent lane. Native actor parsing bounds raw CSV records to 64 KiB, headers
to 64 fields, retained actors/IDs/metadata to at most 32 MiB (less with limited
headroom), and alias expansion to the source projection limits. Cancellation and
resource probes run during parsing. Input that exceeds a budget is dropped
before acquiring the source writer; a validated upload can therefore still fail
an explicit worker-memory budget. Later pressure cancels and awaits owned work,
preserving committed partial source and its index intent. Attributed seed source
transactions recheck an opaque actual task owner/epoch, live lease and cancellation
under the parent job guard as well as the existing seed-owner lease.

The coordinator persists Retry-After without shortening it to the local backoff
cap. Authentication/schema/identity/oversize conflicts retain intent for correction;
transient failures back off and enter cooldown. The service returns bounded retry
feedback and adds no competing retry loop. No production throughput or
idle-versus-loaded latency has been established by the policy fixtures.

## Rebuild, recovery and retirement

An authorized gateway Reindex request is a real attributed maintenance Job. Its
consumer persists a separate canonical keyset scan, exact pending scan/delta body,
visible checkpoint and catch-up barrier. Three bounded scan pages earn one delta
turn while scanning, so a hot tail cannot starve scan progress. Source events are
retained until the replacement becomes the serving reader. A lifecycle change
concurrent with scanning becomes a revisioned catch-up tombstone.

The active writer closes before the shadow writer opens, sharing the one
configured writer/merge budget. The old verified reader remains serving. Promotion
requires a visible complete replacement at or beyond the previous serving
checkpoint and a synced manifest rename. Only the matching promotion receipt
advances the source serving cursor, completes causal indexing children and allows
the maintenance Job to finish. Source checkpointing and index commit remain
separate transactions with durable replay, not an exactly-once claim.

An explicitly admitted rebuild can supersede a permanently rejected live body.
The exact old body and source events stay preserved until verified promotion.
If the old reader is ahead after an uncertain commit, its checkpoint must match
that exact saved body's through-checkpoint; unknown remote work is rejected.
Readiness does not create rebuild intent or clear blocked work.

A damaged shadow preserves the last healthy reader and failed directory. Explicit
repair is fenced by the physical manifest checksum, deployment/source identity and
writer epoch; it restarts a full canonical scan rather than guessing a checkpoint.
A rebuild allows at most three such repair restarts before correction is required.
Corrupt active storage can be recovered after a service restart only when intact
owned manifest evidence proves its identity. Original manifests and damaged
generations remain on disk. A fresh empty repair baseline reports not-built,
never a successful empty search.

Retention is bounded to 64 inactive generations plus one active and at most one
shadow. Cleanup requires the separate explicit retirement DTO/route. It refuses
active/pending generations, symlinks, nested/oversized directory layouts and stale
manifest/source/epoch evidence. A persisted retirement journal reconciles an
interrupted unlink; successful request identities make retries idempotent. HTTP
retirement waits for query slots. There is no automatic generation/tombstone GC,
blind corrupt-volume deletion, arbitrary path selection or configuration-reset
shortcut. A full retention bound or unowned/malformed manifest requires reviewed
operator action.

Real temporary-disk tests cover commit/reload/reopen, lost acknowledgement replay,
tombstones, revision/source/lease/cursor conflicts, Unicode/alias ranking,
shadow promotion, active/shadow corruption preservation, explicit repair,
retirement replay, writer locking and payload bounds. Router/policy fixtures cover
Host/Origin/purpose/body limits, pacing and controlled pressure hysteresis. Core
fixtures use actual isolated database registration/admission/dispatch and model the
service receipt boundary separately from the real Tantivy disk tests. Required
remaining system evidence includes source/outbox/index/gateway/dossier delivery,
abrupt process death/reload failure under load, measured Linux pressure/recovery,
concurrent queries and both Linux architectures; see the plan for actual results.

Use the shared matching debug cache and one integration/build owner. Scoped service
checks use `python3 scripts/agent_cargo.py test --locked -p fleetworks-search-index`.
Do not build the engine to verify a native-only view change.
