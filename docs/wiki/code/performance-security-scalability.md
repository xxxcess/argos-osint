# Performance, Security, and Scalability Assessment

[Code route](index.md) · [CI workflows](ci.md) · [GSD Nightly: Performance & Security](../../../.agents/skills/gsd/SKILL.md#nightly-performance-security)

Baseline assessment of Fleetworks / Argos on `nightly` (`2d767d1`) against its three primary engineering tenets: **Performance**, **Security**, and **Scalability**. Reviewers and the [Nightly Performance & Security](../../../.github/workflows/nightly-performance-security.yml) workflow use this scorecard to track how PRs targeting `nightly` move the project relative to baseline, and to hand off concrete implementation specs to coding agents without blurring review and code-delivery boundaries.

## Baseline scorecard

Evaluated on `nightly` @ `2d767d1` (2026-11-25) across 15 weighted sub-axes (0–100 scale):

| Tenet | Baseline Score | Sub-Axis Breakdown | Summary |
| --- | :---: | --- | --- |
| **Performance** | **64 / 100** | Backend & Storage Latency **62** · Async & Concurrency **74** · Native & Web UI **66** · ML & Search **58** · Observability & Benchmarks **60** | Strong lock-free Tantivy snapshots, bounded RocksDB profiles, pooled IPC/HTTP, and strict React Compiler; held back by single-permit SQLite read/write serialization, cold-spawned decision worker, double-hop Web relay, `ForceCanvas` WebGL/DOM churn, and hot-path SurrealDB round trips. |
| **Security** | **81 / 100** | Auth & Credential Hygiene **84** · Isolation & AuthZ **86** · Input / Query / SSRF / Path **72** · Deployment & Containers **85** · CI & Supply Chain **78** | Mature defense-in-depth (purpose-scoped constant-time auth, HMAC pairing, Keychain/Secret Service isolation, parameterized SurrealQL/SQLite, unnamed IPC socket pair, Landlock+seccomp/Seatbelt QA sandboxes, hardened Compose); gaps in SSRF IPv6/Docker-range/redirect guards, Operator-to-Native grant inheritance + `profile/export` revalidation, and unpinned CI actions / shell interpolation. |
| **Scalability** | **58 / 100** | Bounded Queues & Backpressure **74** · Data / Storage / Pagination **52** · Concurrency & Elasticity **56** · Client Large-Dataset **62** · Durability & Retention **48** | Disciplined bounded semaphores/channels on private services and native desktop caps, but unscheduled disk retention/GC in `fleetworks-jobs` and SurrealDB, fake DB pagination in `intel_service::articles` + Web infinite-query refetch amplification, full-table scans into RAM, and local cursor/flock/bind-mount coupling block multi-replica and long-running scale. |
| **Composite** | **68 / 100** | Equal-weight average `(64 + 81 + 58) / 3` | Solid single-workstation security and defensive primitives; targeted, localized fixes across the High-impact specs below can lift Composite into the **80–85+** range without rewrites. |

## Scoring rubric and PR delta tracking

Every [Nightly Performance & Security](../../../.agents/skills/gsd/SKILL.md#nightly-performance-security) review on an open PR targeting `nightly` evaluates changes against this rubric and reports exact-head score deltas:

| Band | Score | Meaning |
| --- | :---: | --- |
| **Exemplary** | 90–100 | Bounded, measured, multi-replica ready, zero high-severity gaps, automated CI regression gates. |
| **Strong** | 75–89 | Sound architecture and guards; only localized medium/low gaps remain. |
| **Developing** | 60–74 | Core primitives exist, but concrete high-impact bottlenecks or missing retention/guards limit production scale. |
| **At Risk** | 40–59 | Hard scaling/latency walls or material security gaps under sustained or adversarial load. |
| **Critical** | 0–39 | Systemic defects, unbounded failure modes, or exploitable security flaws. |

### PR delta rules

1. **Directional delta per tenet**: Report `Δ Performance`, `Δ Security`, `Δ Scalability` (e.g., `+3`, `0`, `-2`) alongside the updated projected score and affected sub-axes.
2. **Evidence over speculation**: Measured KPIs require baseline/candidate SHAs, workload, platform, and comparable runs; contract/boundedness improvements cite the exact verifying test or `file:line` invariant; unmeasured expectations are explicitly labeled **unmeasured**.
3. **Baseline updates**: Only after a PR merges into `nightly` and its verified checks are green should the baseline table above be revised to the new commit SHA.

---

## Performance findings and specs

### Strengths already in place

- Lock-free Tantivy serving via `Arc<Snapshot>` under short `RwLock::read`, bounded admission (`REQUEST_CAPACITY = 8`, `WRITE_CAPACITY = 1`), and `2s` query timeout (`services/search-index/src/engine.rs:51-78,359-368,539-547`; `service.rs:34-37,304-329`).
- Bounded Embedded RocksDB profile (`16 MiB` block cache, `2 × 8 MiB` memtables), DISKANN KNN vector recall, and grouped job relation reads (`crates/argos-osint-core/src/storage/open.rs:27-37`, `recall.rs:35-43,173-236`, `jobs_view.rs:236-250`).
- Process-wide pooled HTTP clients, multiplexed unnamed-socket local IPC (`MAX_IPC_INFLIGHT = 64`, `256 KiB` windowed upload chunks), isolated `native_pool.rs` interactive vs. bulk lanes, and CAS-bounded QuickJS memory budgets (`512 MiB` cap) (`http_client.rs:27-101`, `ipc.rs:101-109`, `native_pool.rs:15-128`, `js_runtime_budget.rs:13,91-152`).
- Strict React Compiler (`apps/web-desktop/next.config.mjs:17-20`), keep-alive streaming relay (`apps/web-desktop/app/server-api.ts:181-185`), and virtualized native log table with `tx.live_only()` (`apps/fleetworks-desktop/src/backend_views.rs:2230-2306`).

### High-impact performance specs (agent-ready)

#### PERF-H1 — Split `fleetworks-jobs` SQLite reader/writer pools & materialize log columns
- **Potential file refs:** `crates/fleetworks-jobs/src/lib.rs:31-136`, `crates/fleetworks-jobs/src/scoped_logs.rs:23-24,239-388`, `crates/fleetworks-jobs/src/schema.sql:1-90`, `crates/fleetworks-jobs/src/operations.rs:247-253`.
- **Root cause:** Despite `journal_mode=WAL`, a single `Mutex<Connection>` behind `Semaphore::new(1)` serializes every read behind `F_FULLFSYNC` writes and 750ms 4-pass `json_extract` scoped log scans (`MAX(seq)`, rows, severity `GROUP BY`, distinct `app`). Missing FK indexes on `receipts(job_id)`, `jobs(parent_id)`, and `outbox(job_id)` slow cascading prunes.
- **Implementation spec:**
  1. Split `Store` into 1 writer `Connection` (`Semaphore(1)`) and a read-only WAL pool (`SQLITE_OPEN_READ_ONLY`, `Semaphore(4)`, `PRAGMA query_only=ON; PRAGMA busy_timeout=5000`); route `command.is_read()`, `scoped_log_page`, `log_list`, `log_get`, `log_counts`, and `log_apps` to the reader pool.
  2. Bump SQLite `user_version` to 2 in `schema.sql`: materialize `job_id`, `severity_rank`, `app`, `event_type`, and `message_summary` columns on `logs` with composite indexes `(job_id, seq DESC)` and `(job_id, severity_rank, seq DESC)`, plus FK indexes `receipts(job_id)`, `jobs(parent_id)`, and `outbox(job_id)`.
  3. In `scoped_logs.rs:266-277`, skip `SELECT MAX(seq)` when `read.upper_seq.is_some()`, and compute severity/app facets only on initial page (`boundary.is_none()`) or explicit facet flag.
- **Verification:** Concurrent read/write benchmark & unit tests in `crates/fleetworks-jobs` proving reads proceed while a write holds the writer permit and `EXPLAIN QUERY PLAN` uses the new composite indexes.

#### PERF-H2 — Warm idle-managed worker session in `qa_decision.rs`
- **Potential file refs:** `services/api-host/src/qa_decision.rs:33,66-136,216-286`.
- **Root cause:** Every `DecisionRuntime::score` call re-hashes the full model pack (`pack::verify`), cold-spawns `llama-server` with `--threads 1`, polls `/health` (75ms) and SurrealDB (200ms) inside a 3s deadline, sends one request, and immediately kills the worker.
- **Implementation spec:**
  1. Maintain a warm `WorkerSession` in `Supervisor` (mirroring `qa_worker.rs`), keyed by `(preference, generation, verified_pack_digest)`, unloaded after an idle TTL (e.g., 60–120s).
  2. Verify the pack on worker launch (or cache by `(path, mtime, ino, size)`) instead of hashing per turn.
  3. Replace the 200ms SurrealDB `replaced` poll loop with a `watch::Receiver<i64>` notified on `inbox_qa` settings mutation; honor `FLEETWORKS_QA_CPU_THREADS` (`1..=4`) instead of hardcoding `"1"`.
- **Verification:** Unit tests with fake decision binary asserting worker reuse across sequential turns, clean respawn on generation/settings change, and idle unload.

#### PERF-H3 — Eliminate double-hop Next.js Web relay & per-request token-file reads
- **Potential file refs:** `apps/web-desktop/app/server-api.ts:68-136`, `apps/web-desktop/connect-session.mjs:132-168,260-396`, `apps/web-desktop/server-origin.mjs:43-97`.
- **Root cause:** Every `/api/v1/[...path]` call runs `requireWebBoundary` twice, `rustUpstream()` twice (up to 4 open/fstat/read/close token-file syscalls), and an extra HTTP POST to `/api/v1/connect/sessions/resolve` before forwarding to Rust, where Rust revalidates the session again.
- **Implementation spec:**
  1. Remove the redundant `requireWebBoundary` call at `server-api.ts:69` and pass the resolved `upstream` handle once.
  2. Cache `readProtectedTokenFile` in `server-origin.mjs:43-82` keyed by `(path, dev, ino, mtimeMs, size)` with short stat revalidation while preserving symlink/mode checks.
  3. Forward session cookie token + expected revision headers directly to Rust `/api/v1/{endpoint}` so Rust's `context_boundary::gateway` validates authoritatively in one hop (reserving `/connect/sessions/resolve` for SSR page loads).
- **Verification:** `apps/web-desktop` unit tests verifying single upstream hop on API relay, preserved `401`/`409` fail-closed responses on revoked/stale sessions, and token rotation pickup.

#### PERF-H4 — Fix `ForceCanvas.tsx` WebGL allocations, GPU leaks, 60 FPS DOM thrash & per-render serialization
- **Potential file refs:** `packages/intel/src/topology/ForceCanvas.tsx:113-203,239-243,423-438`, `packages/intel/src/topology/Topology.tsx:153-163,209-265`.
- **Root cause:** `positionLink` allocates new `Vector3` + `BufferAttribute` GPU uploads per link every frame; node/link geometries & materials are never `.dispose()`d; `createLabelRenderer` runs `querySelectorAll` at 60 FPS writing `style.opacity` unconditionally; `ForceCanvas` runs `JSON.stringify`+`JSON.parse` every render and polls `cameraPosition()` every 250ms.
- **Implementation spec:**
  1. Share module-level `SphereGeometry` + `MeshBasicMaterial` across nodes; call `.dispose()` on per-link `BufferGeometry`/`LineDashedMaterial` when removed.
  2. Pre-allocate a 6-float `Float32Array` `BufferAttribute` per link and update via `pos.setXYZ(...)` + `pos.needsUpdate = true`; skip `computeLineDistances()` when endpoints move `< 1e-4`.
  3. Maintain a `Map<string, HTMLElement>` of active glyphs in `createLabelRenderer` (pruning `opacityCache` on removal) and skip style writes once settled.
  4. Replace `JSON.stringify({ nodes, links })` with memoized structural revision keys in `Topology.tsx` and replace the 250ms `setInterval` with a throttled `controls` `"change"` listener.
- **Verification:** Vitest + Playwright topology tests confirming zero DOM/GPU regressions and preserved selection/reduced-motion behavior.

#### PERF-H5 — Batch hot SurrealDB queries & remove per-request `INFO FOR TABLE` DDL probes
- **Potential file refs:** `crates/argos-osint-api/src/operational_logs.rs:188-308`, `crates/argos-osint-core/src/storage/operational_logs.rs:111-121`, `crates/argos-osint-core/src/storage/jobs_view.rs:436-441`, `crates/argos-osint-core/src/storage/topology.rs:318-336,2825-2975`, `crates/argos-osint-api/src/live.rs:91-180,220`.
- **Root cause:** `/api/v1/logs` runs `operational_logs::plan` 3× (~15 DB RTTs) with `INFO FOR TABLE` on every request; `topology_snapshot` runs sequential per-node `memory`, `article_in`, `payload`, and redirect lookups; `/api/v1/live` runs 7–8 sequential queries/sec per WebSocket client.
- **Implementation spec:**
  1. Cache activity-table schema verification once at `Repository` connect/bootstrap instead of per-request `INFO FOR TABLE`.
  2. Reuse the `CompleteLogCohort` computed in `initial()` inside `operational_logs::read()`.
  3. Batch-fetch admitted `Claim` memories, `Article` occurrences, and summary payloads in `topology.rs:2899-2975` using `SELECT ... FROM $records` and cache redirects per `Query`.
  4. Combine `live.rs:91-180` read queries into a single multi-statement SurrealQL batch and coalesce polling per `(run_id, article_id)` via a shared `watch` channel (mirroring `HostFeed`).
- **Verification:** `QUERY_ROUND_TRIPS` assertions in `storage_perf.rs` and `argos-osint-api` tests enforcing strict RTT ceilings per endpoint.

### Medium & low-impact performance backlog

| ID | Summary | Primary File Refs | Core Fix |
| --- | --- | --- | --- |
| **PERF-M1** | Fake `batch_size = 1` loop in local ONNX embedder | `crates/argos-osint-core/src/embed.rs:159-172` | Use `tokenizer.encode_batch` + length-bucketed `[N, max_len]` tensors in `self.plan.run`. |
| **PERF-M2** | QuickJS dual-mutex interrupt handler, unused SHA-256 & 4× JSON parsing | `crates/argos-osint-core/src/classifier/engine.rs:206-270,354-399,545-566,739-850` | Lock-free `AtomicBool`/`AtomicU64` deadline, `OnceLock` for `compiled_identity()`, skip SHA-256 without `replay_key`, pass `batch_size` directly. |
| **PERF-M3** | Single-threaded BERT/FLAN-T5 defaults & $O(P \times H)$ prompt re-tokenization | `services/api-host/src/qa_worker.rs:132-144`, `tools/assistant-bert/src/dialogue.rs:161-194`, `tools/atlas-bert/src/main.rs:135` | Default `2..=4` intra-op CPU threads; tokenize header/history/passages once upfront. |
| **PERF-M4** | Tantivy `Engine::apply` per-doc search/JSON parse + unconditional `delete_term` | `services/search-index/src/engine.rs:314-339,497-537,650-701` | Batch existing-doc lookups with one `BooleanQuery`, skip deletes on fresh shadow rebuild, cache `StatusSnapshot` lock-free. |
| **PERF-M5** | Unbounded `atlas::articles_for_intel` `SELECT *` + legacy `browse_assoc` | `crates/argos-osint-core/src/storage/atlas.rs:812-845`, `browse.rs:87-188` | Add `LIMIT`/cursor & column projection; drop redundant Rust sort. |
| **PERF-M6** | Native desktop per-frame `orbit::layers()` tessellation, 1–2s idle wake churn & eager Jobs list | `apps/fleetworks-desktop/src/orbit.rs:207-262`, `app.rs:1264-1388`, `backend_views.rs:1707-1760` | Memoize `orbit::layers()` in `OnceLock`, wake at exact `next_due`, move Jobs list to virtualized `rune_kit::Table`. |
| **PERF-M7** | Unvirtualized `BulletinView`, unmemoized `BackendContext`, heavy 3D landing `OrbitCanvas` | `packages/intel/src/views/bulletin/BulletinView.tsx:161-230`, `packages/api-client/src/index.tsx:517-538`, `OrbitCanvas.tsx:4,82-97` | Virtualize feed, memoize `BackendContext` value, replace landing `ForceGraph3D` with lightweight 2D/WebGL shader. |
| **PERF-L1** | Missing `gzip`/`br` HTTP compression on Rust API | `crates/argos-osint-api/src/lib.rs:480-492` | Add `tower_http::compression::CompressionLayer` (excluding WS/SSE). |
| **PERF-L2** | Missing exported HTTP/DB/ML latency histograms & strict CI perf gates | `crates/argos-osint-api/src/host_metrics.rs`, `crates/argos-osint-core/tests/storage_perf.rs:17-20` | Add Axum latency middleware + deterministic query-count ceilings in CI. |

---

## Security findings and specs

### Strengths already in place

- Purpose-scoped bearer credentials (`Operator`, `Native`, `WebRelay`, `InternalService`) with SHA-256 digest-only memory storage and `subtle::ConstantTimeEq` verification (`crates/argos-osint-api/src/network.rs:49-247,514-537`).
- HMAC-derived single-issuance native pairing, length-prefixed macOS Keychain & D-Bus Secret Service isolation, explicit fail-closed RAM-only fallback, and `0600` non-symlink credential file checks (`storage/native_pairing.rs:85-97,182-454`, `keychain.rs:63-93`, `linux_secret_service.rs:100-282`).
- Unnamed `UnixStream::pair()` Embedded IPC (`--ipc-fd`), `env_clear()` companion launch, and OS-level QA worker sandboxing (Linux Landlock ABI 3+ + seccomp-BPF, macOS Seatbelt `(deny default)`) (`ipc.rs:1576-1611`, `qa_sandbox.rs:90-397`, `qa_embedded_macos.sb:1-18`).
- Parameterized SurrealQL and SQLite queries, untrusted `x-fleetworks-*` header stripping, per-request CSP script nonces, `SameSite=Strict` cookies, hardened Docker Compose (`read_only`, `cap_drop: [ALL]`, `no-new-privileges`, loopback binds), and zero `pull_request_target` workflows.

### High-severity security specs (agent-ready)

#### SEC-H1 — Close SSRF bypasses & OOM buffering in article/OSINT fetchers and custom provider endpoints
- **Potential file refs:** `crates/argos-osint-core/src/intel_recon/body.rs:859-908`, `crates/argos-osint-core/src/osint.rs:710-736`, `crates/argos-osint-api/src/configuration.rs:488-504`, `crates/argos-osint-core/src/provider_attempt.rs:523-526`, `crates/argos-osint-core/src/provider.rs:2574-2579`.
- **Risk:** `body.rs::is_private_host` misses `172.16.0.0/12` (Docker bridge hosting internal services), `0.0.0.0/8`, `100.64.0.0/10`, and all IPv6 literals (because `url::Url::host_str()` wraps IPv6 in `[...]`); follows up to 5 unvalidated redirects without DNS pinning; buffers entire responses before enforcing the 2 MB cap. `osint.rs:721` fails to parse bracketed IPv6 literals (`[::1]`, `[::ffff:127.0.0.1]`). `configuration.rs:488-504` lets `custom`/`openai-compatible` accounts target `127.0.0.1`/private IPs with redirect-following HTTP clients.
- **Implementation spec:**
  1. Add `crates/argos-osint-core/src/ssrf.rs` with `is_public_ip(IpAddr)` (rejecting IPv4 loopback, RFC 1918 `10/8`, `172.16/12`, `192.168/16`, link-local `169.254/16`, `0/8`, CGNAT `100.64/10`, multicast/broadcast/doc; and IPv6 loopback, ULA `fc00::/7`, link-local `fe80::/10`, unspecified, multicast, and `to_ipv4_mapped()` recursively) plus `resolve_and_pin_public(url)` using `tokio::net::lookup_host` + `reqwest::ClientBuilder::resolve_to_addrs`.
  2. In `body.rs:859-908`: disable automatic redirects (`Policy::none()`), manually follow up to 5 hops revalidating every `Location` with `resolve_and_pin_public`, and stream bytes via `bytes_stream()` enforcing the 2,000,000-byte limit incrementally.
  3. In `osint.rs:710-736`: match `u.host()` directly and validate with `ssrf::is_public_ip`.
  4. In `configuration.rs:488-504`, `provider_attempt.rs:524`, `provider.rs:2576`: set `.no_redirect()`, restrict loopback `http://` exclusively to `kind == "local"`, and reject non-public IPs on non-`local` accounts unless explicitly allowlisted by operator config.
- **Verification:** Offline unit tests covering `[::1]`, `[::ffff:127.0.0.1]`, `172.16.0.1`, `100.64.0.1`, `169.254.169.254`, 302-to-loopback redirects, and >2 MB response streams.

#### SEC-H2 — Scope Native pairing permissions & revalidate `profile/export` under `ConfigLock`
- **Potential file refs:** `crates/argos-osint-core/src/storage/native_pairing.rs:238-255`, `crates/argos-osint-api/src/configuration.rs:722-825`, `crates/argos-osint-api/src/control.rs:47`, `crates/argos-osint-api/src/context_boundary.rs:72-74`.
- **Risk:** `native_pairing::grants` inherits `ManageConfiguration`, `MaintainBackend`, and `ManageInboxQa` from the approving Operator, letting any paired workstation export all backend API keys in plaintext via `POST /api/v1/profile/export`. Additionally, `configuration::export` never calls `context.revalidate(&state.repo)` under `ConfigLock`, allowing a TOCTOU race with credential revocation.
- **Implementation spec:**
  1. In `configuration.rs:801-825`: require `Extension<ResolvedRequest>`, restrict plaintext credential export to `CredentialPurpose::Operator` (or a dedicated `Permission::ExportSecrets`), acquire `ConfigLock`, and call `context.revalidate(&state.repo).await?` before reading `auth`/`settings`.
  2. In `native_pairing.rs:238-255`: default paired Native grants to a least-privilege workstation profile (`ReadBackend`, `RegisterWorkstation`, `RebindWorkstation`, `SubmitInteractive`, `ReadWorkstationActivity`, `RetryJob`, `CancelJob`) with explicit operator opt-in for administrative grants.
- **Verification:** Integration tests proving a revoked credential cannot export secrets and a default-paired Native workstation receives `403` on `/api/v1/profile/export`.

#### SEC-H3 — Pin all third-party GitHub Actions to 40-char SHAs & fix bash expression injection
- **Potential file refs:** `.github/workflows/{atlas-bert,embedded,fleetworks-inbox-qa-pack,fleetworks-native-linux,fleetworks-native,gsd-nightly-core,mutation,ui-visual-review}.yml` (unpinned `Swatinem/rust-cache@v2`, `dtolnay/rust-toolchain@1.95.0`, `rui314/setup-mold@v1`); `.github/workflows/fleetworks-native-linux.yml:88,211,379` and `.github/workflows/fleetworks-native.yml:193` (direct `${{ inputs.expected-head }}` / `${{ github.workflow }}` in `run:`).
- **Risk:** Mutable third-party action tags risk upstream supply-chain compromise on runners holding `depot-ci-token` and release build paths; direct `${{ ... }}` interpolation in `run:` enables shell injection.
- **Implementation spec:**
  1. Pin `Swatinem/rust-cache`, `dtolnay/rust-toolchain`, and `rui314/setup-mold` to verified 40-char commit SHAs across all 8 workflows (Code Architect scope).
  2. In `fleetworks-native-linux.yml:88,211,379` and `fleetworks-native.yml:193`: validate `[[ "$FLEETWORKS_HEAD" =~ ^[0-9a-f]{40}$ ]]` early and pass `inputs.expected-head` and `github.workflow` strictly via `env:` variables.
- **Verification:** Run `.github/scripts/nightly_perf_security.py` static workflow audit (0 findings) and `test_fleetworks_ci.py`.

### Medium & low-severity security backlog

| ID | Summary | Primary File Refs | Core Fix |
| --- | --- | --- | --- |
| **SEC-M1** | Pairing global rate-limit DoS, indefinite post-issuance `poll()` recovery, non-CT `!=` checks | `crates/argos-osint-core/src/storage/native_pairing.rs:137-208,335-391` | Bound post-issuance recovery (e.g., 5m or explicit authenticated `/pairing/ack`), add per-peer-IP rate limiting at gateway, use `subtle::ConstantTimeEq` on `proof_hash`/`approval_code`/`token_hash`. |
| **SEC-M2** | Unpinned Docker base images, flat Compose network, lax `read_secret` checks, raw Operator token mounted in `api` | `deploy/Dockerfile:3,7,59,86,99,131`, `deploy/Dockerfile.native-embedded:1`, `deploy/compose.native-review.yml:279`, `services/api-host/src/fleetworks.rs:390-407` | Pin all `FROM` tags by `@sha256:`, split Compose into `edge`/`internal`/`datastore` networks, enforce `O_NOFOLLOW`+`0600` in `read_secret`, support `token_sha256` in `gateway-credentials.json` so `api` never mounts raw Operator secrets. |
| **SEC-M3** | Unvalidated `node.imageUrl` sink, direct third-party OSINT image loads (OPSEC leak), missing HSTS/XFO/Permissions-Policy | `packages/intel/src/topology/nodeGlyph.ts:90-94`, `apps/web-desktop/proxy.ts:17,41-49` | Shared `safeImageUrl` helper (`https:` or proxied only), opt-in or server-proxied OSINT thumbnails + tighten `img-src`, add `X-Frame-Options: DENY`, `Permissions-Policy`, `COOP`, HSTS (HTTPS), and API CSP. |
| **SEC-L1** | Redaction gaps (`SettingsFile` keys, `x-fleetworks-*`, `amqps://`/`wss://`, pairing JSON), missing `zeroize`, `PATH` spawns (`tar`/`codex`/`nvidia-smi`), Linux `getpeereid` fallback | `crates/argos-osint-core/src/events.rs:91-140`, `provider_diag.rs:458-499`, `storage/install.rs:94`, `deploy/local_pairing_approval.py:79,202-206` | Expand `events::redact`/`provider_diag`, derive `ZeroizeOnDrop` on secret structs, replace `tar` subprocess with pure-Rust `tar`+`flate2`, pass `approval_code` via `argv` and check `SO_PEERCRED` on Linux. |

---

## Scalability findings and specs

### Strengths already in place

- Bounded group-commit SurrealDB memory writes (`QUEUE_CAP = 256`, `BATCH_CAP = 32`) with full-jitter optimistic transaction retries (`crates/argos-osint-core/src/storage/publisher.rs:32-34,146-228`, `tx.rs:69-85,159-224`).
- Bounded admission across private services (`jobs_service` 32, `execution_service` 8, `nlp_service` 4, `classifier_service` 1, `datasets` 2, `search-index` 8, `MAX_DETACHED_QUERIES = 32`), Tantivy RSS/disk/latency pressure hysteresis, distributed provider pacing, and transactional credit holds.
- SQLite VM interruption via `progress_handler` (`SCAN_BUDGET = 750ms`), bounded topology BFS (`4096..=32768`), chunked actor catalog seeding (`128` rows / `2 MiB`), disciplined Native Desktop row caps, and cgroup limits on core Compose services.

### High-impact scalability specs (agent-ready)

#### SCALE-H1 — Schedule bounded retention/GC for `fleetworks-jobs` (RocksDB + SQLite) & SurrealDB tables
- **Potential file refs:** `crates/fleetworks-jobs/src/queue.rs:81,141-146,220-231,377-385`, `crates/fleetworks-jobs/src/lib.rs:130-133`, `crates/fleetworks-jobs/src/operations.rs:247-253`, `crates/fleetworks-jobs/src/logs.rs:115-120`, `crates/argos-osint-core/src/storage/atlas_delete.rs:568`, `crates/argos-osint-core/src/storage/credits.rs:32-39,234-236`, `services/api-host/src/jobs_service.rs`, `crates/argos-osint-core/src/scheduler.rs`.
- **Root cause:** RocksDB queue keeps acknowledged `M` payloads (up to 16 KiB) and `C` receipts forever with `DBCompressionType::None`; SQLite `receipts` with `job_id = NULL` never cascade or expire; `Command::Prune`, `log_prune`, `prune_events`, `atlas_prune_expired`, and `credit_reservation` cleanup have zero scheduled callers.
- **Implementation spec:**
  1. In `queue.rs`: enable `Lz4`/`Zstd` compression, index settled `M` messages and `C` receipts by `settled_at`, and add bounded batch `QueueCommand::Prune { before_ms, limit }`.
  2. In `fleetworks-jobs/src/lib.rs`: skip inserting a receipt on empty `Claim -> Task(None)`; add `created_at` to `receipts` and prune expired rows in `Command::Prune`.
  3. Start a periodic bounded housekeeping loop (e.g., every 15 min) in `Role::Jobs` / Embedded `local-ipc` (calling `Prune`, `log_prune`, `QueueCommand::Prune`) and under the elected scheduler lease in `scheduler.rs` (calling `atlas_prune_expired`, `jobs::cleanup_batch`, and `credits::prune_terminal`).
- **Verification:** Storage/jobs unit tests proving settled payloads, `NULL`-job receipts, expired logs, and terminal credit reservations shrink within bounded batch limits.

#### SCALE-H2 — Push keyset pagination into `intel_service::articles` & cap Web `useInfiniteQuery` refetches
- **Potential file refs:** `crates/argos-osint-core/src/intel_service.rs:128-228`, `packages/api-client/src/index.tsx:750-788,874,1015`, `packages/intel/src/views/bulletin/BulletinView.tsx:161-230`.
- **Root cause:** `intel_service::articles` runs an unbounded `SELECT * FROM atlas_article` per page and slices in Rust; `useArticles` has `refetchInterval: 60_000` with no `maxPages`, refetching every loaded page every minute; `BulletinView` mounts all pages in a non-virtualized `ScrollView`.
- **Implementation spec:**
  1. Add keyset cursor predicates and `LIMIT $limit + 1` in SurrealQL inside `intel_service::articles` (backed by composite indexes or an admission-maintained `atlas_article_latest` projection).
  2. Set `maxPages: 10` on `useArticles`, `useArticleSearch`, and `useRelated` (or replace 60s full-chain polling with a lightweight head-revision check that refreshes page 0 only).
  3. Virtualize `BulletinView.tsx` (aligning with PERF-M7).
- **Verification:** Core pagination tests with >500 synthetic articles asserting bounded DB fetch size per page, plus React Query tests verifying bounded refetch page count.

#### SCALE-H3 — Replace unbounded full-table SurrealDB scans into process RAM
- **Potential file refs:** `crates/argos-osint-core/src/storage/actor_catalog.rs:120-137,179-191`, `storage/repo.rs:175-232`, `storage/vectors.rs:141-165`, `storage/browse.rs:87-188`, `storage/nlp_cache.rs:98,108-166`.
- **Root cause:** `vectors::reindex` loads every active memory into one `Vec` and deletes old embeddings in one transaction; `nlp_cache::put` runs full-table `math::sum` + unbounded `SELECT` on every write; `actor_catalog::load_matcher` duplicates the entire actor catalog in every container's RAM with process-local invalidation; `browse.rs` loads all claims/actors with $O(K^2)$ `Vec::contains`.
- **Implementation spec:**
  1. Stream `vectors::reindex` in keyset pages (`LIMIT 256`) and delete stale embeddings in `LIMIT 500` chunks.
  2. Track `nlp_cache` total bytes incrementally (or throttle eviction checks), bound eviction scans (`LIMIT 128`), and batch `gc_features()`.
  3. Stream `actor_catalog::load_matcher` in keyset pages, batch `backfill_curated_aliases`, and key `MatcherCache` on a persisted SurrealDB catalog revision token.
  4. Paginate `browse.rs` and replace `Vec::contains` with `HashSet`.
- **Verification:** Storage tests with synthetic multi-batch fixtures asserting bounded per-step memory/page counts and cross-process matcher cache invalidation.

#### SCALE-H4 — Stateless signed pagination cursors & distributed config fencing (unblock horizontal replicas)
- **Potential file refs:** `crates/argos-osint-api/src/operational_logs/cursors.rs:14-15,86-130`, `crates/argos-osint-api/src/activity_paging/cursors.rs:16-17,110-158`, `crates/argos-osint-api/src/admission.rs:112-120`, `deploy/compose.native-review.yml:5-10,40`.
- **Root cause:** Process-local `LogCursors` (128 cap) and `JobsCursors` (256 cap) break pagination across >1 `api`/`jobs` replica and evict active clients under load; `ConfigLock` relies on POSIX `flock` on a shared `/workspace` bind mount held across async DB transactions.
- **Implementation spec:**
  1. Replace process-local cursor `BTreeMap`s with AEAD/HMAC-authenticated opaque cursor tokens bound to `(scope_binding_hash, expires_at, keyset_anchor)` using a deployment-scoped signing key.
  2. Migrate `ConfigLock` serialization to SurrealDB optimistic `configuration_revision` fencing (`run_tx`) so API/control replicas need no shared POSIX filesystem lock.
- **Verification:** Multi-instance unit tests validating cursors minted by instance A decode and validate cleanly on instance B (and reject tampered/expired/mismatched-scope tokens).

#### SCALE-H5 — Add global gateway concurrency/load-shedding & WebSocket fan-out caps
- **Potential file refs:** `crates/argos-osint-api/src/lib.rs:326-492,568-584`, `crates/argos-osint-api/src/live.rs:56-75,220-311`.
- **Root cause:** Public `api` and `brain`/`tools`/`control` routers lack a global concurrency/load-shedding cap, `run_shielded` awaits indefinitely until the 45s timeout, and `/api/v1/live` allows unlimited WebSocket connections each polling SurrealDB every 1s.
- **Implementation spec:**
  1. Add a bounded `try_acquire_owned` admission semaphore returning `503`/`429` with `Retry-After` on public & private Axum routers, and bound `run_shielded` semaphore wait.
  2. Cap concurrent `/api/v1/live` WebSocket upgrades with an `Arc<Semaphore>` (e.g., 64–256) and coalesce brief-progress polling per brief via `watch` broadcast (shared with PERF-H5).
- **Verification:** `argos-osint-api` tests asserting fast `503` load-shedding under saturation and single-poll coalescing across multiple WebSocket subscribers.

### Medium & low-impact scalability backlog

| ID | Summary | Primary File Refs | Core Fix |
| --- | --- | --- | --- |
| **SCALE-M1** | 512-job workstation log cohort ceiling (`CohortPlan::TooLarge`) | `crates/argos-osint-core/src/storage/operational_logs.rs:22`, `crates/fleetworks-jobs/src/scoped_logs.rs:21` | Stamp scope/deployment/workstation tags on SQLite `logs` rows at append time with index `(deployment_id, workstation_id, seq DESC)`. |
| **SCALE-M2** | Tantivy `EmbeddedCatchUpMerge` stalls >500-doc merges; `control.lock()` 503s; non-waiting `retire_reserved` | `services/search-index/src/engine.rs:51-78,497-501,769-773`, `service.rs:470-472` | Switch back to bounded `LogMergePolicy` once `source_complete == true`; bounded timeout drain on reader retirement; lock-free status snapshot. |
| **SCALE-M3** | Missing Docker cgroup limits on `web`, `nlp`, `classifier` | `deploy/compose.yml:8-37`, `deploy/compose.atlas.yml:37-98` | Add explicit `cpus`, `mem_limit`, and `pids_limit` matching `compose.native-review.yml`. |
| **SCALE-M4** | `jobs_paging::page` repeats `COUNT(*)`/`GROUP BY` inside write `run_tx` | `crates/argos-osint-core/src/storage/jobs_paging.rs:155,198-243` | Run outside write transaction; compute facets only on `after.is_none()`. |
| **SCALE-M5** | Native Desktop log follow halts permanently at 500 rows; `native_worker` requeue churn | `apps/fleetworks-desktop/src/logs.rs:620-681`, `services/api-host/src/native_worker.rs:285-296` | Ring-buffer `VecDeque` + `HashSet` sliding window for follow mode; partition `tools` vs `bulk` queues so full bulk lanes don't claim/requeue every 500ms. |
| **SCALE-M6** | $O(N^2)$ DAG cycle check in `fleetworks-jobs::submit`; unchunked `atlas_delete` SQL | `crates/fleetworks-jobs/src/operations.rs:289,318-331`, `storage/atlas_delete.rs:78-133` | Replace cycle check with Kahn's algorithm ($O(V+E)$); chunk `atlas_delete` queries into batches of 100–200. |
| **SCALE-L1** | Unbounded `storage::runtime::spawn` & silent drop in `spawn_ordered` | `crates/argos-osint-core/src/storage/runtime.rs:165-234` | Add semaphore cap, remove per-item wrapper tasks, expose drop counter. |
| **SCALE-L2** | Fixed-delay `provider_pacer` retries, global RPM caps, unbounded `Command::Tasks`/`Attempts`, unjittered WS reconnect | `storage/provider_pacer.rs:70-86`, `fleetworks-jobs/src/read.rs:104-129`, `apps/web-desktop/app/live-transport.ts:97` | Exponential full-jitter backoff, per-account RPM keys, cursor/limit on `Tasks`/`Attempts`, ±30% reconnect jitter. |

---

## Complementary features and architectural shifts

1. **Event-driven push bus over `fleetworks-broker` / IPC / WebSocket:** Replace 1s DB polls (`live.rs`), 500ms worker polls (`native_worker.rs`/`atlas_worker.rs`), and 60s client polls (`useArticles`, `app.rs`) with push invalidations (`IpcEvent`, `/api/v1/live` `watch` fan-out, and optional RabbitMQ via `crates/fleetworks-broker`) to make idle and multi-client load $O(1)$.
2. **Centralized SSRF-enforcing egress service + `internal: true` Docker networks:** Isolate `api`, `surrealdb`, `brain`, `control`, `jobs`, and `search-index` onto internal networks with zero internet egress; route external OSINT/article/provider fetches through one egress-filtered fetcher with DNS pinning and byte caps.
3. **Pre-hashed gateway manifests + tamper-evident security audit log:** Eliminate raw bearer mounts in `api` via `token_sha256` entries and record append-only security audit events (pairing lifecycle, exports, provider URL changes, resets).
4. **Shared memory-mapped / service-backed actor matching & read-model projections:** Serve entity matching from `search-index` (or an `mmap` FST built per Tantivy generation) and materialize latest daily articles + scoped log facets incrementally on write.
5. **Automated CI Performance & Security regression gates:** Run [Nightly Performance & Security](../../../.github/workflows/nightly-performance-security.yml) on every PR targeting `nightly`, and follow up with `cargo-deny`/`cargo-audit`, `pnpm audit`, `zizmor`, and deterministic query-round-trip CI budgets.
