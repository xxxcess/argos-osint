# Brief Performance and a Unified Brain Memory Graph — checklist

Spec: "Argos — Brief Performance and a Unified Brain Memory Graph" (2026-10-13, audited main `a12bc9b`, schema 36).
Branch `feat/brief-performance-unified-brain-graph` from origin/main `a12bc9b05ff314b15e89d0432489c130628bbecb`. Recheck at start: origin/main is identical to the audited SHA, so there is no drift. The §1 findings were reconfirmed in source: `BODY_WINDOW_BYTES=48_000`; `is_ambiguous` substring cues `share`/`related`; `insight_output_options` sends limits only for `JsonMode`; `MemoryKind` has `AtomicClaim` plus legacy kinds; schema 36.

Phases follow §6's order: (1) timing/request policy and canonical Brain types/ID migration, (2) shared sentences/cleanup and role routing, (3) Brain actor/relationship writer and cross-cycle backfill, (4) Atlas phase 6 and global/typed UI consumers, (5) oversized-body comparison, QA and docs.

Legend: **done** / **partial** / **deferred** / pending.

| § | Item | Status | Phase |
|---|------|--------|-------|
| 2 | Output-limit support decoupled from JSON mode; compatible fields for Google/Nvidia/OpenRouter; unknown transports diagnosed | pending | 1 |
| 2 | Capability-aware reasoning effort/budget in operation policy; unsupported options do not repeatedly fail a route | pending | 1 |
| 2 | Per-stage timing aggregation (local, admission, first meaningful output, generation, fidelity, backoff, persistence, total); active substep + role/model in Jobs/Logs; progress sinks wired | pending | 1 |
| 4 | `MemoryKind` actor/claim/inference/link/related + legacy kinds; typed payload/version/revision/lifecycle; one transactional writer; `EndpointRef` | pending | 1 |
| 4 | Migration: atomic_claim→Claim, insight classification→Claim/Inference, IDs preserved | pending | 1 |
| 2 | Structural HTML prose extraction; soft-wrap normalization with offsets | pending | 2 |
| 2 | Block-aware chrome cues (no substring `share`/`related`) | pending | 2 |
| 2 | Parse once, remap spans, `SentenceIndex` persisted and consumed by all stages + canonical publication | pending | 2 |
| 2 | Actor-focused n-gram retrieval → passage packets with omission manifests | pending | 2 |
| 2 | Compact requests; windows sized on the serialized request | pending | 2 |
| 2 | Interactive brief priority over background maintenance | pending | 2 |
| 3 | Dispatch declares (role, operation, format, input revision, policy); Stage-2 fidelity → Evidence Curator; Stage-3 support stays Claim Assessor | pending | 2 |
| 3 | `defaults.operation_overrides` + Models → Brief operations + import/export + per-stage dependency hashes | pending | 2 |
| 4 | Brain actor tables, `BrainActorService`, aliases/mentions/external refs/redirects, memory actor refs | pending | 3 |
| 4 | Legacy entity/derivation-link/lead migration to Actor/Link/Related | pending | 3 |
| 4 | Recall/graph on canonical Actor IDs; kind filters; vector metadata | pending | 3–4 |
| 5 | Atlas phase 6 `atlas-collection-v2`, `STAGE_ACTOR_ARTICLES=12`, receipts, resume | pending | 4 |
| 5 | `RelatedScope`, `related_articles_global`, `actor_articles`; AllRetained default in the UI | pending | 4 |
| 5 | Backfill + lifecycle (`atlas_delete_run` fix, redirects, TTL safety) | pending | 3–4 |
| 2 | Focused vs full-body path comparison on a fixed corpus (measured) | pending | 5 |
| 7 | All acceptance checks, test matrix, screenshots, docs/diagrams/graphify | pending | 5 |
