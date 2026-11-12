# Acceptance tracker — Fleetworks native cutover

Source: [design.md](design.md); delivery: [handoff.md](handoff.md). Updated November 12, 2026. Implementation remains in progress. Passing checkpoints below are scoped evidence, not complete cutover acceptance. Local commits are now authorized; push, publication, actual user reset/layout transition and installed dual-Mac validation remain separate.

## Latest verified checkpoints

| Area | Actual result | Limits / next affected check |
| --- | --- | --- |
| Cache cleanup | COMPLETE;10.76GiB freed from57 proven obsolete agent incremental sessions | Preserved user target/debug, useful shared debug dependencies/fingerprints/binaries, active work and global caches |
| Native/contracts/workspaces | PASS:84:26 contracts +37native +11workspace unit +10workspace integration on the activation/lifecycle checkpoint | Includes queued cancel/failed Atlas retry and rebind/retirement/guarded Trash. Fixture invalidation failure corrected; affected app26+workspace21 pass with contract24 reused. Five added appearance/freshness tests pass; cfg0/missing assertion passed its 2-case contract rerun. Native/matte appearance PNGs inspected; 6newactivationUItests pass; their captures, full matrix and newest ARM/installed-app evidence pending |
| Core persistence | PASS: 29 persistence + 4 bounded-parser/activation unit cases on recorded checkpoints | Latest18-case required-server rerun passed, including genuine Natural training/activation, ambiguity retirement fence, config-change exact replay and claimed-task cancellation/takeover. Embedded reset/cache-owner and API journeys pending |
| Backend/search compilation | PASS unified host+search server/nlp/classifier debug check;1m41s,0warnings | Includes schema 8, durable pump/reset startup plus bounded parser/local pressure/exact seed task fences and explicit activation/auxiliary drains. Latest18 persistence cases pass; parser/domain4, worker6 and affectedAPI5 cases pass; embedded/public/broker/Linux journeys pending |
| API unit behavior | PASS:45 unique cases on recorded checkpoints; latest5 affected discovery/readiness and1 real capture/save/acceptance guard regression pass | Initial run42/44; fixed real registry prerequisite and canonical temporary parent. Embedded gateway/worker and newest owner/reset fixtures pending |
| Native worker | PASS: 6 focused lane, parser/hash, pressure/cooldown/async-wait and safe-failure cases;5m cached compile | Actual broker/Linux delivery and measured loaded resource behavior remain pending |
| Search service | PASS: 13:9 real disk/recovery/generation cases + 2 HTTP/pacing cases + 2 pressure unit cases | Host Mac evidence; actual DB→outbox→Linux index→gateway, rebuild pump and measured Linux load remain pending |
| Web | Selected 8 Playwright journeys PASS;6 PNG states inspected; isolated production build/typecheck/compiler check PASS | Connect/HttpOnly/stale-tab/logout plus retained brief/topology/list/focus. Full acceptance matrix remains open |
| Python tooling | PASS current65: platform7, candidate16, planner15, native-review2, reset17, verifier3, candidate-images5 | Real packaged download, Linux image build/start/inference and final changed-tool reruns remain pending |
| Native visual evidence |2 baseline Brain + 3 Tools/Atlas/Profile + 2 action + 2 appearance PNG/tree pairs inspected | Cancel preview and wide retry receipt inspected; compact retry viewport does not show receipt. Native/matte toolbar comparison inspected; remaining lifecycle states and full size/material/accessibility matrix pending |
| Docs | Native connection/workspace guides and main routes updated; Fleetworks HTML/SVG pair updated and inspected | Remaining affected guides/tags/figure pairs and retirement references are tracked in [phase4-design.md](phase4-design.md) |

Native scoped Clippy --all-targets -Dwarnings passes after9reported fixes; the final84-case affected rerun passes without compiler warnings. Current tooling rerun executes all65cases across7suites.

Counts exclude zero-test targets, ignored/filtered companion targets and duplicate reruns. New source invalidates only affected evidence. Reset preparation is deliberately unsupported in production while full legacy retirement is incomplete; no real installation, credential or running stack has been reset.

## Contract and phase gates

| ID | Spec | Finite acceptance | Required evidence |
| --- | --- | --- | --- |
| A01 | 1, 2.2 | Both Rune crates pinned together; one coordinated compiler; Mac graphs exclude Venture/Docker/terminal/webview/DB/search/inference engines; native opens offline | Exact pins/features/lock/toolchain, dependency graph and both-Mac build/offline shell evidence |
| A02 | 1.3, 2.1 | Every old command disposition and required page action maps to a real endpoint or justified external operator task | One endpoint matrix with route/payload/owner/permission/origin/revision/idempotency/UI/test and existing/extend/create/blocked status |
| A03 | 1.1, 2.1 | Eight pages in two groups; same shell/client/runtime; global settings shared across children and isolated by deployment/access | Runtime/transport fixtures plus real global configuration operations and scope assertions |
| A04 | 3.1 | Home container/default protected; only verified immediate children; safe create/attach/remove/delete/recovery | Temporary-home tests for traversal, external/nested paths, ownership, symlink/case/NFC/reserved/ancestor/mount/substitution races; siblings/data/history intact |
| A05 | 3.1 | Offline creation keeps stable pending identity; all-known-backend retirement precedes deletion; active/shared jobs protected | Crash/ambiguous-call/journal/revocation/filesystem-failure and recreation-new-ID fixtures |
| A06 | 3.2–3.4 | Canonical shared parser, explicit native interface, authenticated ensure/reuse, collision/rebind, default protection | Rust/TS corpus, IPv4/IPv6/NFC bounds, concurrent database uniqueness and second-deployment tests; opaque IDs not raw Connect records |
| A07 | 4.1–4.3 | Validated advertised URL, central server gate, remote authenticated HTTPS ingress; no data before authority | Direct pages/RSC/prefetch/REST/actions/exports/data/live denial, revoked/expired/forged/unknown/wrong-backend/outage cases, CSRF/Origin and stale-tab `409` |
| A08 | 4.4 | New native/web jobs have immutable origin/config refs at acceptance; reuse adds association and preserves creator | Brief ensure/reload + Atlas/Tools + seed/train/reindex admission, uncertain response, retry/children/queue/restart/dedup/legacy tests |
| A09 | 4.5 | Native Jobs/Logs shows only eligible registered-node work; child filter narrows; no global mode | Two nodes/two children/second deployment; exact IDs/counts; guessed detail/stage/result/log/export/live/action denials; mixed-origin events and shared-job cancel restrictions |
| A10 | 5.1 | Partial typed readiness is read-only and truthful; unknown/stale/offline distinct; health remains lightweight | GET side-effect checks, freshness/reason/permission states, optional-service failure, hidden polling/backoff/latest-context tests |
| A11 | 5.2–5.3 | Bounded dataset purpose/hash/bytes; durable seed/Natural train and separate explicit activation | Public API/worker persistence, upload limits/expiry, lease/busy/idempotency/cancel/restart, seed partial outcomes, prior model retained on failure; no Mac/server arbitrary path |
| A12 | 6.1 | Private Linux search, constrained bounded query, one actor/document and stable authorized result IDs | Real query/Unicode/alias/ambiguity/exact-prefix/filter tests; no raw query syntax/DocAddress/direct service exposure |
| A13 | 6.2 | Canonical source changes and index intent commit together; visible checkpoint before ack; safe rebuild | Real disk restart, rollback/commit/crash/ambiguous ack/duplicate/out-of-order/alias-only/delete/redirect/tombstone/reader-reload/backfill/writer-lock/corrupt-generation tests |
| A14 | 6.3 | Brain debounces/supersedes; gateway hit → selected SurrealDB dossier; empty/indexing/unavailable distinct | Production handler keyboard/click/stale reply fixtures and real public search/detail journey; web outage does not disable it; no native graph/eager graph fetch |
| A15 | 6.4 | Shared batch/count/byte/rate/concurrency/queue budgets; bounded retries/fairness/pressure | Controlled clocks/barriers, Retry-After/cooldown/tail/full-queue/slow commit/disk cases; measured idle/loaded query latency, lag, memory/index size on both Linux architectures |
| A16 | 2.3–2.4 | Existing logo untouched; accessible native matte/material states; web colors only; topology/list retained | Token/composite contrast; real-view all-page state/size/scale/fallback/reduced-motion evidence; browser before/after color + unchanged geometry/type/motion/behavior |
| A17 | 8 | Native Cargo/helper builds produce complete exact app independently; platform mismatch/failure safe; backend build/start separate | Both physical-Mac lanes, Rosetta/SDK/tools/path-spaces/output/failed-build cases; dry-run and actual `.app`; tested backend-only Bake/start with matching assets/ports/secrets/identities |
| A18 | 8.3 | One planner selects actual checks/build/recreate/HMR without unrelated work; unknown inputs conservative | Table/mixed/deleted/renamed/untracked/unknown/docs-only/override tests; forced native means both Macs |
| A19 | 9 | Exact-SHA dual-Mac ZIPs, both Linux packages and required aggregate; privileges separated | Real workflow/run/artifact IDs/URLs, downloaded ZIP hash/arch/libraries/permissions/manifest; missing/failed selected lane fails |
| A20 | 9.3, 10 | Both downloaded apps really launch and operate on matching supported hardware | Launch Services/Finder/input/accessibility/dialog/clipboard; authenticated read/mutation/local/remote/setup/web handoff; minimum OS and signing/notarization statuses |
| A21 | 7.1 | Synthetic reset narrow/idempotent/resumable; new keys survive rerun/upgrade; no legacy reload | Exact file/secure-store/backend fixtures, frozen inventory/epoch/receipts, retained root/default/data/infrastructure; ordinary build/start never wipes |
| A22 | 7.1, 3.1 | Actual targeted user-config reset and old-layout transition separately recorded | Explicit authorized targets, independent local/backend receipts, drained work/session revocation/new access; inaccessible installations/remaining provider revocation stated |
| A23 | 7.2, 10.4 | Terminal/client/import/capture products removed after native parity and caller migration | Consumer/command dispositions, migrated data/behavior checks, dependency/image/CLI archive/active-reference audit; justified operators retained |
| A24 | 11 | Actual docs/agent skills/aliases/routes and paired diagrams match implemented behavior | Path/pair disposition inventory, command/link/catalog/tag checks, rendered/inspected modified HTML/SVG, no stale active terminal routing |
| A25 | 12 | Final scoped freeze and honest handoff | Exact source/Rune/Tantivy/schema/index/pace, changed/removed paths, commands/results/URLs/checksums, remaining blockers and separate statuses |


## Current implementation against the gates

| Gates | Implemented source | Still required |
| --- | --- | --- |
| A01–A03 | Pinned Rune/kit, API-only eight-page shell, one runtime/client, global drafts and context fences; initial Intel+ARM compilation | Complete endpoint disposition and page parity; newest ARM build and installed matching hardware |
| A04–A07 | Protected OS-home/default/direct-child manager, durable registration-attempt journal, canonical IDs, real registry/session/rebind/retirement backend, central web gate | 71-case native lifecycle suite passes; actual platform journeys and real OS effects remain pending |
| A08–A09 | Durable creator/request associations at real admission; scoped membership before counts/limits/facets; shared private payload closed; supported queued cancel/failed Atlas retry plus native confirmations | Whole-node/descendant log/export/live completeness and expanded real API/worker journeys |
| A10–A11 | Typed read-only readiness, current-format global configuration CRUD, raw bounded uploads, durable seed/Natural training without auto-activation, safe maintenance result metadata | Genuine disposable training→leased activation fixture and6native activation tests pass; real public upload/seed/train/activate journey and full training observations pending |
| A12–A15 | Private Tantivy service, transactional source intent, canonical result filtering, leases/visible ACK, disk recovery and pressure policy | Durable outbox/rebuild8 and exact claimed-task seed3 fixtures passed; parser/local pressure units, real Linux recovery/load and both architectures remain pending |
| A16 | Native theme/shared production fixtures; web color-only palette and retained canvas/list interactions | System preference cache/matte material/freshness tests pass; affected material/state captures, real OS preferences and physical accessibility remain pending |
| A17–A20 | Platform/planner/packaging tools, backend-only review recipe, exact-SHA CI source | Actual packages/downloads/installed dual-Mac launch, minimum OS, real Linux artifacts; signing/publication separate |
| A21–A23 | Reset inventory/tool, sealed coordinator, purpose/fingerprint/epoch/write barriers and real owner adapters; terminal/import compatibility retained | Startup integration compiles and server barrier fixture passes. Positive embedded reset/recovery/cache-owner tests, external parity before retirement and separately authorized real reset remain pending |
| A24–A25 | Named plans, source/doc inventories, cache-first agent guidance, growing canonical docs and paired figures | Complete affected docs/diagram/agent routing, final source freeze and required verification gates |

## Actual commands and verification policy

User authorization covers scoped local builds/tests, native fixtures, browser checks and isolated Docker integration. Do not ask again within that scope. One integration owner runs Cargo through `python3 scripts/agent_cargo.py … --locked`, reusing the matching shared debug graph; workers edit/format only.

The latest native checkpoint used:

```sh
CARGO_BUILD_JOBS=2 python3 scripts/agent_cargo.py test --locked \
  -p fleetworks-desktop -p fleetworks-workspace -p fleetworks-contracts \
  -- --test-threads=1
```

The recorded backend compilation used:

```sh
CARGO_BUILD_JOBS=2 python3 scripts/agent_cargo.py check --locked \
  -p argos-api-host -p fleetworks-search-index \
  --no-default-features --features server,nlp,classifier
```

Focused Core persistence uses the pinned isolated Surreal3.3.4 harness with required mode. Server fixtures do not replace the required native embedded gate. API embedded-only targets executing zero cases in the server graph are PENDING, not PASS. Hosted server/all-platform/mutation matrices remain separate; local mutation execution is deliberately skipped.

Browser work uses repository Playwright with `ARGOS_WEB_TEST_DEV=1`; user frontend output/locks remain intact. Native Runtime/PNG/tree scenes use the production views/handlers with synthetic effects. A fixture is not an installed-app or real provider journey. No real homes, secrets, paid APIs, production writes or user stack are test inputs.

## Active work and finish line

1. Preserve source checkpoint e3e84c0 and its84native/33Core/45API/6worker/15Venture/13search/65Python evidence. Verify next scoped operational-log integration, saved local connection profiles and the build-once native renderer verifier as separate increments.
2. Preserve the passed 78-case appearance/lifecycle checkpoint; finish affected captures and integrate exact-candidate native activation only after its backend consumer checks. Native activation remains disabled until its real backend consumer is verified.
3. Run the required embedded gate and actual isolated Docker source-to-visible/recovery/pressure journeys. Finish affected docs/paired diagrams and tooling checks.
4. Commit coherent verified source checkpoints. Preserve the baseline scheduler commit. Do not stage scratch or unrelated changes.
5. Keep exact-SHA publication, downloaded dual-Mac/minimum-OS GUI evidence, terminal retirement and real reset/layout receipts explicit. No full cutover claim until every applicable gate above is supported.

Detailed command output is retained in ignored local verification logs. This curated tracker records results independently of those scratch files. Result vocabulary is PASS, FAIL, BLOCKED, SKIPPED or PENDING, with scope and limitations.

## Commit ledger

- `e3e84c0` — committed the84-native/33-Core/45-API/6-worker/15-Venture/13-search/65-Python scoped source checkpoint, plans/diagrams/deployment/tooling/CI source. Required embedded/Linux/installed-artifact acceptance remains pending. Future A09 files and saved-profile parity were excluded.

- `294a634` — committed curated source spec, endpoint/reset/docs inventories, acceptance/handoff/decisions and requested cache-first agent guidance. No push. Local scratch and working-memory files remain ignored.
- `062471c` — committed the latest backend/Core verification and remaining-gate refresh; no push.
- `0fb1a0a` — committed verified web Connect authority/session/context behavior and palette, with 61 frontend/fixture files. Scoped unit/Node/compiler/type/build checks,8 browser journeys and6 inspected PNGs are its recorded evidence; real public ingress/backend integration remains pending.
- Next source checkpoint: coherent manifests, native/contracts/workspaces, authority/context/configuration and backend/search integration after affected checks. Unreferenced or later unverified increments are excluded; current pending tests remain explicit.

## Next source checkpoint

The scoped log backend passes 6 new executed cases: 3 SQLite persistence/deadline/projection cases, 2 API cursor cases, and 1 real required-server membership/cohort fixture. The 513-job cap and selected child/descendant behavior ran; zero-test companion targets are excluded. Unified host/search compilation passed without warnings. The first affected test build took 10m21s; the exact Core fixture rerun reused that graph in 2.13s and passed in 49.90s.

The latest app/contracts/workspace checkpoint passed all 113 cases (32 contracts, 55 app, 16 workspace unit and 10 integration) after the Jobs/Logs tabs, visible labels and virtualized Table repairs. The matching scoped Clippy check passed. Its warmed debug test build took 1m26s; app tests took 6.16s. The initial focus API mismatch, clipped-pointer fixtures and queued focus-event assertion were corrected without weakening their contracts. Updated captures are being generated; the additive Jobs filters/paging modules remain unverified and outside this 113-case checkpoint.

The focused required-embedded logs/reset check passed 10 executed cases: 5 API owner/reset/cursor cases, 1 real public-gateway/private-Jobs-SQLite log page/export/follow fixture and 4 Core fence/drain/reprovision/replay fixtures. Its initial fixture exposed an outer middleware cache-header overwrite; the canonical policy was repaired to `no-store, private`, preserving the assertion. The affected warm rerun passed. This does not replace the pending full native embedded matrix or authorize a production reset.

Current Python tooling passes 99 cases across 9 suites. This includes the 64 tool cases detailed below, 6 deployment cases and 29 reset cases. Seven guidance/policy cases and all 1,689 relative documentation references pass. The Rune skill and alias validate. The first real renderer invocation generated 52 captures; inspection findings remain explicit. The public Docker journey and actual Linux worker artifact execution are pending.

These increments remain separate from source checkpoint `e3e84c0`. Source completion, a clean build and synthetic fixtures do not imply full cutover acceptance. Production reset is sealed; physical-Mac, CI/download, retirement and actual reset obligations remain open.

## Committed tooling checkpoint

The native renderer verifier passed its 10 offline cases and actually generated 52 captures from the fresh compiler-reported executable, using the implicit shared host debug graph. This records generation; inspection found Jobs/Logs tab and visible-label gaps, and the large-list debug capture showed 532.81ms/1,482 layout nodes, so affected UI work remains open. The app/workspace/contract checkpoint passed 112 tests and scoped Clippy before these latest UI repairs.

The tool-only source has 64 passing Python cases: platform 7, candidate 16, planner 16, image candidate 5, native verifier 10, Linux worker verifier 10. Seven guidance/policy checks pass. The default host build helper now omits Cargo's explicit target argument, and unknown build inputs conservatively select checks/images. Three changed workflows pass the Code Architect's static actionlint check; real hosted Linux inference and Mac artifact jobs remain pending. This checkpoint excludes the still-changing UI/backend/reset/deployment smoke increments.

- `fa52b0c` — committed the independent native/Linux verification, planner and agent-guidance checkpoint: 64 tool Python cases, 7 policy cases and actual 52-capture generation. The new UI layout, reset adapter, operational backend and deployment driver remain in the next source increment. No push or publication.
