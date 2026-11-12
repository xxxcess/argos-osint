# Acceptance tracker — Fleetworks native cutover

Source: [design.md](design.md); delivery: [handoff.md](handoff.md). Updated November 12, 2026. Implementation remains in progress. Passing checkpoints below are scoped evidence, not complete cutover acceptance. Local commits are now authorized; push, publication, actual user reset/layout transition and installed dual-Mac validation remain separate.

## Current implementation and verification — 2026-11-12 15:59 UTC

| Area | Actual status | Remaining / limitation |
| --- | --- | --- |
| Source checkpoints | COMMITTED: main native/activity source `f8c216c`; workspace verification repairs `d7636c0` | Full cutover acceptance is still open; no push/publication |
| Cache cleanup | COMPLETE: 10.76 GiB freed from 57 proven obsolete agent sessions | User `target/debug`, useful shared caches, active work and global caches preserved |
| Native/contracts/workspaces | PASS: 119 unique scoped cases (32 contracts + 61 desktop + 16 workspace unit + 10 integration); six affected paging cases reran after the equivalent guard fix | Physical GUI, minimum OS and installed-platform journeys remain pending |
| Native lint / ARM source | PASS: native all-target Clippy 11.15s; latest ARM source check 2m01s | Cross-compilation is not physical ARM acceptance |
| Backend scoped behavior | PASS: six focused Logs cases, six new Jobs paging cases and ten focused required-embedded Logs/reset cases | Evidence overlaps; do not add these into a new total. Full runtime/Docker verification remains pending |
| Full canonical matrix | FORMAT PASS (9s), WORKSPACE LINT PASS (3.28s); workspace TEST BUILD RUNNING on `d7636c0` | Current build reached Core after SurrealDB; test execution, storage CLI smoke and required storage phase are not complete |
| Native debug artifact | PASS: clean Intel debug app/archive from `f8c216c`; independent safe extraction, bundle/hash/permissions/Mach-O/library checks passed | Unsigned review artifact; GUI launch, second Mac artifact and physical minimum-OS checks not performed |
| Search / worker / persistence baseline | Prior scoped checkpoints record 13 host search, six worker, 33 Core and 45 API cases | Current search/fixture lint repairs pass workspace lint; runtime rerun is part of the running matrix. Actual Linux recovery/load/inference remains pending |
| Web | PASS: eight selected Playwright journeys, six inspected PNGs and isolated production build/type/compiler checks on unchanged source `0fb1a0a` | Real public ingress/backend/platform acceptance remains separate |
| Tooling / guidance | PASS: 99 Python cases across nine suites, seven guidance/policy cases; Rune skill/alias validated | Hosted artifact execution/download and real Linux inference remain pending |
| Native visual evidence | Final run: 40 generated, 11 selected inspected across ten scenes; prior profile/log run: 52 generated, 14 selected inspected | Generated variants are not all visually approved; real OS input/accessibility remains pending |
| Docs / diagrams | 20 affected HTML/SVG pairs checked, rendered and inspected; 1,688 relative links pass | Consumer retirement stays gated on replacement/platform parity |
| Isolated Docker | Preflight/recipe/static checks prepared; actual build/start/journeys PENDING | Fresh owned project/workspace/ports required; user's stack/images are protected |
| Actual cutover | PENDING: hosted CI, matching physical Macs, retirement, actual reset/layout and trusted publication | Production reset support remains sealed; no real user installation or credentials reset |

The active task checklist has been reconciled with these results. Completed source items and scoped checks are separate from unexecuted runtime/platform gates. Counts exclude zero-case targets and duplicate reruns. The earlier scoped checkpoints remain evidence for their recorded revisions; they do not replace the running full matrix.

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
| A01–A03 | Pinned Rune/kit, API-only eight-page shell, one runtime/client, global drafts/context fences and saved connection profiles; latest host checks and ARM source check pass | Installed matching hardware/minimum-OS journeys, complete external endpoint parity and actual app packaging |
| A04–A07 | Protected OS-home/default/direct-child manager, durable registration-attempt journal, canonical IDs, real registry/session/rebind/retirement backend, central web gate | 71-case native lifecycle suite passes; actual platform journeys and real OS effects remain pending |
| A08–A09 | Durable creator/request associations; scope before counts/limits/facets; bounded private log/export/follow cohorts; real Jobs filters/keyset paging; shared private payload closed; confirmed supported cancel/retry | Six new paging backend cases and six native paging cases pass; scoped Logs/public private-owner fixtures pass. Expanded isolated Linux API/worker journeys remain pending |
| A10–A11 | Typed read-only readiness, current-format global configuration CRUD, raw bounded uploads, durable seed/Natural training without auto-activation, safe maintenance result metadata | Genuine disposable training→leased activation fixture and6native activation tests pass; real public upload/seed/train/activate journey and full training observations pending |
| A12–A15 | Private Tantivy service, transactional source intent, canonical result filtering, leases/visible ACK, disk recovery and pressure policy | Scoped outbox/rebuild, claimed-task seed and bounded parser/local policy evidence passes; real Linux recovery/load and both architectures remain pending |
| A16 | Native theme/shared production fixtures; web color-only palette and retained canvas/list interactions | System preference cache/matte material/freshness tests pass; affected material/state captures, real OS preferences and physical accessibility remain pending |
| A17–A20 | Platform/planner/packaging tools, backend-only review recipe, exact-SHA CI source | Actual packages/downloads/installed dual-Mac launch, minimum OS, real Linux artifacts; signing/publication separate |
| A21–A23 | Reset inventory/tool, sealed coordinator, purpose/fingerprint/epoch/write barriers and real owner adapters; terminal/import compatibility retained | Four positive required-embedded Core reset cases, actual API owner recovery and 29 synthetic Python cases pass. Actual owner deployment, external parity before retirement and separately authorized real reset remain pending |
| A24–A25 | Named plans/inventories, cache-first and Rune agent guidance, affected canonical docs and 20 rendered/inspected diagram pairs | Consumer retirement remains gated; finish source checkpoint, packaging and required runtime/platform verification before full acceptance |

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

1. Preserve the committed source/tooling/docs/reset checkpoints and their revision-bound evidence. Native profiles, Logs and Jobs paging now pass 119 scoped cases, strict native Clippy and the latest ARM source check; finish the repaired host strict lint gate and source checkpoint.
2. Preserve the inspected material/lifecycle/profile/Logs evidence. The final Jobs renderer generated 40 captures, with 11 selected captures covering all 10 chosen scenes inspected. Physical GUI/accessibility and complete training/activation journeys remain separate.
3. Build and verify the actual debug app archive, then run the required full embedded gate and actual isolated Docker source-to-visible/recovery/pressure journeys. Reuse unaffected web and diagram evidence.
4. Commit coherent verified source checkpoints. Preserve the baseline scheduler commit. Do not stage scratch or unrelated changes.
5. Keep exact-SHA publication, downloaded dual-Mac/minimum-OS GUI evidence, terminal retirement and real reset/layout receipts explicit. No full cutover claim until every applicable gate above is supported.

Detailed command output is retained in ignored local verification logs. This curated tracker records results independently of those scratch files. Result vocabulary is PASS, FAIL, BLOCKED, SKIPPED or PENDING, with scope and limitations.

## Commit ledger

- `f8c216c` — committed saved connections, scoped Jobs/Logs, real paging and affected contracts/fixtures/plans (88 files). Scoped native119, strict native/backend Clippy, latest ARM source check and selected final captures pass. The clean Intel debug app/archive built and passed independent bundle verification; full embedded and Docker/platform gates remain separate.

- `f311350` — committed saved-profile reset CAS/recovery and fresh configuration revision-zero verification (5 files), with 29 synthetic reset and 4 deployment verification cases passing. Production reset remains sealed and unexecuted.

- `1272a0b` — committed affected canonical guides, 20 exact HTML/SVG pairs and curated local evidence (53 files). All changed pairs passed XML/accessibility/parity checks and were rendered/inspected; 1,688 relative documentation links pass. This is documentation evidence, not a runtime/platform gate.

- `e3e84c0` — committed the84-native/33-Core/45-API/6-worker/15-Venture/13-search/65-Python scoped source checkpoint, plans/diagrams/deployment/tooling/CI source. Required embedded/Linux/installed-artifact acceptance remains pending. Future A09 files and saved-profile parity were excluded.

- `294a634` — committed curated source spec, endpoint/reset/docs inventories, acceptance/handoff/decisions and requested cache-first agent guidance. No push. Local scratch and working-memory files remain ignored.
- `062471c` — committed the latest backend/Core verification and remaining-gate refresh; no push.
- `0fb1a0a` — committed verified web Connect authority/session/context behavior and palette, with 61 frontend/fixture files. Scoped unit/Node/compiler/type/build checks,8 browser journeys and6 inspected PNGs are its recorded evidence; real public ingress/backend integration remains pending.
- Next source checkpoint: coherent manifests, native/contracts/workspaces, authority/context/configuration and backend/search integration after affected checks. Unreferenced or later unverified increments are excluded; current pending tests remain explicit.

## Next source checkpoint

Native profiles, Logs and Jobs paging pass 119 scoped cases: 32 contracts, 61 desktop, 16 workspace unit and 10 integration. All six new paging cases executed. The warm build took 44.07s and desktop tests 7.04s. After an equivalent stale-ticket lint repair, those six affected cases passed again (7.16s build / 0.29s runtime); they are not additional unique coverage. Strict native Clippy passes in 11.15s, canonical formatting passes, and the latest ARM source check passes in 2m01s. Cross-compilation does not establish physical ARM or minimum-OS behavior.

The log backend passes six focused cases: three SQLite persistence/deadline/projection, two opaque cursor and one real required-server cohort case, including the 513-job narrowing boundary. Jobs paging passes six new backend cases: two cursor-cache units, two required-embedded Core and two public API fixtures. Its initial affected build took 8m31s; the integration build reused that graph in 15.33s, with API runtime 16.23s and Core 19.81s. Strict all-target Core/API/host/bin Clippy now passes; the final warm retry took 50.33s. Reported lint repairs preserve assertions and protocol behavior, without suppressions.

The focused required-embedded logs/reset check passes ten executed cases: five API owner/reset/cursor, one actual public-gateway/private-Jobs-SQLite page/export/follow and four Core fence/drain/reprovision/replay cases. These overlap the narrower evidence above and are not summed into a new total. The actual cache-header failure was repaired to `no-store, private`; the affected rerun passed. Python tooling passes 99 cases across nine suites, including 29 synthetic reset cases. Seven guidance/policy checks and 1,688 relative documentation references pass. Production reset remains sealed and unexecuted.

The profile/log renderer generated 52 captures, with 14 selected captures covering all 13 scene names inspected. The final Jobs/preserved-view renderer generated 40 captures, with 11 selected captures covering all ten chosen scenes inspected. Current representative saved-profile and loaded-Logs PNGs match their prior bytes exactly. The 450-row Table has 268 layout nodes in the recorded debug diagnostic; single contended frames do not establish production responsiveness. Generated but uninspected variants and real GUI behavior remain separate. All 20 affected HTML/SVG pairs were checked, rendered and inspected.

The actual clean-source debug app archive now passes build and independent verification; the full native embedded matrix stopped before tests on search-service lint blockers. Minimal search and fixture repairs now pass strict all-target workspace Clippy (3m42s); the required runtime phases will resume. The isolated Docker journey remains pending. Physical-Mac, hosted CI/download, real Linux inference artifacts, trusted publication, terminal retirement and actual reset/layout obligations remain open. Source completion and scoped synthetic checks do not imply full cutover acceptance.

## Committed tooling checkpoint

The native renderer verifier passed its 10 offline cases and actually generated 52 captures from the fresh compiler-reported executable, using the implicit shared host debug graph. At that checkpoint, inspection found Jobs/Logs tab and visible-label gaps, and the large-list debug capture showed 532.81ms/1,482 layout nodes. Those findings were repaired and verified in the subsequent source increment described above. The app/workspace/contract checkpoint passed 112 tests and scoped Clippy before these latest UI repairs.

The tool-only source has 64 passing Python cases: platform 7, candidate 16, planner 16, image candidate 5, native verifier 10, Linux worker verifier 10. Seven guidance/policy checks pass. The default host build helper now omits Cargo's explicit target argument, and unknown build inputs conservatively select checks/images. Three changed workflows pass the Code Architect's static actionlint check; real hosted Linux inference and Mac artifact jobs remain pending. This checkpoint excludes the still-changing UI/backend/reset/deployment smoke increments.

- `fa52b0c` — committed the independent native/Linux verification, planner and agent-guidance checkpoint: 64 tool Python cases, 7 policy cases and actual 52-capture generation. The new UI layout, reset adapter, operational backend and deployment driver remain in the next source increment. No push or publication.

## Local debug app artifact

Source `f8c216c2b3cdfcfa6b4f22098fdf5f18f75f312f`, clean tree, version0.2.1, debug, `x86_64-apple-darwin`, repository toolchain1.95.0 and exact Rune pin. The build used the implicit shared host debug graph. The archive is `Fleetworks-0.2.1-f8c216c2b3cd-local-1794495750731160000.1-x86_64-apple-darwin.zip` (34,616,869 bytes). SHA256: `57dabfec3889346b18301bc1900af224cfb31acb285c877b21a626d0daf84594`.

The separate verify-native invocation checked archive/source/profile/Rune identity, safe extraction, bundle file hashes/permissions, Info.plist, Mach-O architecture and linked libraries. All passed. Declared minimum macOS is13.0; actual minimum-OS behavior is unverified. This is an unsigned local review app. GUI launch, signing/notarization, hosted download checks and the second physical Mac remain pending.

The full workspace lint retry passes after minimal search-service control-flow/error-layout/portable-probe repairs and three equivalent Jobs fixture initializers. HTTP response contents, auth/pacing semantics, promotion behavior and assertions are unchanged. The original matrix failure remains recorded; runtime acceptance still requires the remaining canonical matrix stages.
