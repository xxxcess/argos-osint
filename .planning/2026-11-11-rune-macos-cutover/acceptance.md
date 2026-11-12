# Acceptance tracker — Fleetworks native cutover

Source: [design.md](design.md); delivery: [handoff.md](handoff.md). Updated November 12, 2026. Implementation remains in progress. Passing checkpoints below are scoped evidence, not complete cutover acceptance. Local commits are now authorized; push, publication, actual user reset/layout transition and installed dual-Mac validation remain separate.

## Latest verified checkpoints

| Area | Actual result | Limits / next affected check |
| --- | --- | --- |
| Cache cleanup | COMPLETE;10.76GiB freed from57 proven obsolete agent incremental sessions | Preserved user target/debug, useful shared debug dependencies/fingerprints/binaries, active work and global caches |
| Native/contracts/workspaces | PASS: 78:26 contracts + 31 native + 11 workspace unit + 10 workspace integration on the appearance/lifecycle checkpoint | Includes queued cancel/failed Atlas retry and rebind/retirement/guarded Trash. Fixture invalidation failure corrected; affected app26+workspace21 pass with contract24 reused. Five added appearance/freshness tests pass; subsequent cfg0/missing precondition assertion passed its 2-case contract rerun. New activation UI, captures and newest ARM/installed-app evidence pending |
| Core persistence | PASS: 25 recorded cases: registry 5, setup 5, actions 3, activity 2, network configuration 1, reset barrier 1, outbox 4, rebuild 4 | Latest 10 cases passed using required pinned server harness; embedded reset/cache-owner fixtures and exact task-fenced seed increment remain pending |
| Backend/search compilation | PASS unified host+search server/nlp/classifier debug check;1m26s,0warnings | Includes schema 8, durable rebuild pump, canonical reindex route and reset startup/cache owners. New seed pressure/fencing and Natural activation are later increments |
| API unit behavior | PASS: 44 unique cases:40 unchanged initial passes + 4 affected Brain/dataset module passes | Initial run42/44; fixed real registry prerequisite and canonical temporary parent. Embedded gateway/worker and newest owner/reset fixtures pending |
| Search service | PASS: 13:9 real disk/recovery/generation cases + 2 HTTP/pacing cases + 2 pressure unit cases | Host Mac evidence; actual DB→outbox→Linux index→gateway, rebuild pump and measured Linux load remain pending |
| Web | Selected 8 Playwright journeys PASS;6 PNG states inspected; isolated production build/typecheck/compiler check PASS | Connect/HttpOnly/stale-tab/logout plus retained brief/topology/list/focus. Full acceptance matrix remains open |
| Python tooling | Platform 7, candidate 16, planner 15, native-review 2, reset 17, verifier 3, candidate-images 5 passed on recorded revisions | Real packaged download, Linux image build/start/inference and final changed-tool reruns remain pending |
| Native visual evidence |2 baseline Brain + 3 Tools/Atlas/Profile + 2 action PNG/tree pairs inspected | Cancel preview and wide retry receipt inspected; compact retry viewport does not show receipt. Corrected lifecycle fixtures and complete state/size/material/accessibility matrix pending |
| Docs | Native connection/workspace guides and main routes updated; Fleetworks HTML/SVG pair updated and inspected | Remaining affected guides/tags/figure pairs and retirement references are tracked in [phase4-design.md](phase4-design.md) |

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
| A10–A11 | Typed read-only readiness, current-format global configuration CRUD, raw bounded uploads, durable seed/Natural training without auto-activation, safe maintenance result metadata | Real upload/seed/train journey, complete training observations and separate exact-candidate activation |
| A12–A15 | Private Tantivy service, transactional source intent, canonical result filtering, leases/visible ACK, disk recovery and pressure policy | Durable rebuild/outbox 8 focused fixtures passed; exact claimed-task seed fencing, bounded local parser/RSS increment, real Linux recovery/load and both architectures remain pending |
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

1. Preserve the verified schema 8/reset-startup/rebuild checkpoint. Complete exact claimed-task seed fences, bounded parser/local resource probes and exact-candidate activation as separately verified increments; do not relabel these as already passed.
2. Preserve the passed 78-case appearance/lifecycle checkpoint; finish affected captures and integrate exact-candidate native activation only after its backend consumer checks. Native activation remains disabled until its real backend consumer is verified.
3. Run the required embedded gate and actual isolated Docker source-to-visible/recovery/pressure journeys. Finish affected docs/paired diagrams and tooling checks.
4. Commit coherent verified source checkpoints. Preserve the baseline scheduler commit. Do not stage scratch or unrelated changes.
5. Keep exact-SHA publication, downloaded dual-Mac/minimum-OS GUI evidence, terminal retirement and real reset/layout receipts explicit. No full cutover claim until every applicable gate above is supported.

Detailed command output is retained in ignored local verification logs. This curated tracker records results independently of those scratch files. Result vocabulary is PASS, FAIL, BLOCKED, SKIPPED or PENDING, with scope and limitations.

## Commit ledger

- `294a634` — committed curated source spec, endpoint/reset/docs inventories, acceptance/handoff/decisions and requested cache-first agent guidance. No push. Local scratch and working-memory files remain ignored.
- `062471c` — committed the latest backend/Core verification and remaining-gate refresh; no push.
- `0fb1a0a` — committed verified web Connect authority/session/context behavior and palette, with 61 frontend/fixture files. Scoped unit/Node/compiler/type/build checks,8 browser journeys and6 inspected PNGs are its recorded evidence; real public ingress/backend integration remains pending.
- Next source checkpoint: coherent manifests, native/contracts/workspaces, authority/context/configuration and backend/search integration after affected checks. Unreferenced or later unverified increments are excluded; current pending tests remain explicit.
