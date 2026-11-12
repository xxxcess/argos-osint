# Decisions and risk gates — Fleetworks native cutover

The full [source contract](design.md) controls; the [handoff](handoff.md) owns delivery order. Decisions below define the implementation contract; actual results and open work are in acceptance.md.

| Decision | Rationale and consequence |
| --- | --- |
| Preserve the supplied spec verbatim in `design.md` | Durable handoff must not rely on a personal Downloads path; its SHA-256 is recorded in the handoff |
| Use the spec's `2026-11-11-rune-macos-cutover` directory | Consolidated revision is November 12; date difference is intentional, not a second task |
| Preserve current scheduler fix | Actual HEAD is one in-scope baseline reconciliation step beyond the spec; no rollback or rewrite |
| Implement one thin Rune app, independent browser and Linux backend | No native lifecycle ownership, backend engines, embedded web/terminal adapter or native topology |
| Keep five Backend pages global and activity workstation-scoped | Context/provenance does not partition shared configuration/data or grant permission |
| Reuse persisted network identity for stable deployment identity | Process-specific gateway instance identity cannot key cross-restart registration or caches |
| Establish purpose permissions before registry/web exposure | Reviewed network policy only supports loopback/bearer; no existing per-purpose authority can be assumed |
| Use a protected home container and immediate children | Operator CWD `.fleetworks` semantics are different; never register/delete root or protected default |
| Preserve canonical IDs and immutable snapshots | IP/folder changes do not rewrite job history; markers/IP/Connect ID are not credentials |
| Join origin/request association to actual durable acceptance | Queued work cannot execute unattributed; dedup preserves creator and adds requester association |
| Enforce membership before all activity surfaces | UI filtering cannot protect totals/deep links/exports/live; association alone does not grant cancellation |
| Centralize authoritative web guards | Layout/proxy redirect alone misses RSC/REST/export/upgrade boundaries; old tabs need revision checks |
| Keep Natural training optional and activation explicit | Existing worker does not retrain Rust-BERT; candidate completion is not inference/activation success |
| Project canonical actors transactionally into private Tantivy | SurrealDB remains truth; search service is isolated from desktop graph; commit/reload visibility precedes ack |
| Treat pacing values as testable starting policy | Resource-wide lease/budgets, pressure handling and real measurements are required; no inferred speedup |
| Reuse planner/candidate tools, not new frameworks | One path/action source, physical host detector and shared packager for local and CI |
| Keep implementation, verification, reset, Mac launch and publication separate | Fixture/CI success cannot establish installed hardware behavior or user-installation cleanup |
| Retire clients only after replacement gates | Six clients, second Ratatui consumer and capture pipeline need caller/behavior migration first |
| Keep shared plans/history and Jobs persistence independent | Superseded authority requires reviewed disposition; no unrelated cleanup or migration completion claim |

## Risks to settle early

| Risk | Required evidence / owner | Blocks |
| --- | --- | --- |
| Rune/kit revision and repository compiler differ | Inspect pinned source/manifests/features; build/test selected graph on Intel and ARM; integration owns any coordinated toolchain change | Broad native implementation/working quick-start claim |
| Headless renderer/GUI unavailable on runners | Validate actual adapter/fonts/rendered PNG/tree; matching physical-Mac Launch Services/input/accessibility check stays separate | Visual/installed-app acceptance |
| Current shared bearer cannot express required purposes | Explicit scoped native/web/maintenance/config/activity policy and trust-scope documentation | Registry/session exposure |
| Global versus origin scope accidentally conflated | Two children share roles/keys/corpus; two nodes/deployments isolate activity; expected revision tests | Functional parity |
| Native path race or unsafe old-layout adoption | Object-identity/ownership/default/ancestor/symlink fixtures, journal and multi-backend retirement recovery | Workspace mutation |
| Job metadata lost across reuse/queues | Test durable admission + request association + retry/child/restart/read-model boundaries | Activity and attribution |
| Index is committed but not serving current data | Explicit reader-visible checkpoints, replay/tombstone/generation/catchup tests | Search-ready status |
| Shared resource limits multiply across clients/replicas | One lease, bounded coordinator and controlled-clock policy tests plus real load evidence | Safe ingestion |
| Desktop workspace member breaks Linux Docker manifest resolution | Clean-context backend build with narrow source inputs; no Rune/inference rebuild from native view-only edit | Backend delivery |
| Required Linux classifier libraries unavailable on one architecture | Real matching-checkpoint inference and reproducible architecture-specific artifact packaging; no silent emulation/drop | Dual-platform backend |
| Reset removes infrastructure or silently runs twice | Exact frozen allowlist, synthetic interrupted/epoch-repeat tests, independent receipts/admin recovery; actual run separately authorized | Real reset |
| Terminal consumers force obsolete clients to remain | Command/consumer inventory, migrate automation and preserve data assertions before deletion | Final retirement |

## Authorization and finish line

The original cleanup/planning pass is complete. The user subsequently authorized implementation and tracking against these docs on November 12, then confirmed scoped local builds/tests, native fixtures, browser checks and isolated Docker integration. The user also explicitly authorized local commits and requested progress updates in the planning docs. Root owns exact-path staging and coherent checkpoint commits; no implicit push/publication/messages or mutation of a running user stack; a real reset requires concrete frozen targets and applicable destructive-cutover authorization. Reuse authorization actually present and do not add inferred approval gates.

Freeze each coherent implementation increment before its final affected checks. Reopen for a demonstrated acceptance blocker or user direction; defer unrelated discoveries. Never convert a missing selected lane, zero-match test, uninspected PNG or fixture-only integration into PASS.

## Current implementation decisions

- Inspect and reuse matching target/profile/features/storage mode before compiling; default scoped local checks/tests to shared debug output. Preserve target/debug and useful agent/global caches. Release/cross-target builds serve specifically required artifact/platform evidence.
- Keep native and backend edit/build windows disjoint; reuse passing evidence when a later change does not affect it. Zero-case companion targets never increase pass counts.
- Production reset remains sealed until full current-only legacy retirement. Loaded token fingerprints and real process cache ACKs cannot be replaced by rereading mounted files or caller JSON. New nonservice work and causal children share the reset guard; actual draining/terminal proof precedes deletion.
- Reindex enablement follows the actual durable consumer/promotion pump. Source checkpoints and retained reader visibility remain distinct; no cursor is advanced merely because a request was sent.
- Commit plans/cache guidance now, then coherent implementation checkpoints after affected checks. Do not push, publish, perform a real reset or remove compatibility products merely to close a checkbox.

- Record source implementation, executed tests and external cutover separately in every planning refresh. As of the latest update, local plan/cache commit `294a634` exists; linked schema 8/reset/rebuild compiles and 10 focused server fixtures pass, while embedded reset/owner and real Linux/physical-Mac acceptance remain pending.
- Missing maintenance configuration preconditions must fail with428; exact already accepted request replay must preserve its original receipt across later configuration revisions. Enforce this in the shared admission/storage transaction, with an affected replay regression.
