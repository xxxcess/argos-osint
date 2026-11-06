# Maintain the repo wiki

[Wiki](../index.md) · [Tags](tags.md) · [Planning format](code/planning-artifacts.md#working-format)

The wiki is curated Markdown checked against current source. Navigation is `AGENTS.md` → `docs/index.md` → concepts or code → reference/source. No generated code graph or external indexing service is required. Source, manifests and executable tests settle implementation facts; docs explain intent and contracts. Record disagreements instead of silently guessing.

The setup/refresh distinction takes inspiration only from [Atomic Claude's onboarding workflow](https://github.com/damusix/atomic-claude#the-workflow) and [repo wiki pipeline](https://github.com/damusix/atomic-claude/blob/main/docs/reference/repo-wiki.md): inventory facts, then write useful navigation. Argos uses canonical wiki pages, source tags, named plans and paired HTML/SVG diagrams, with no Atomic runtime dependency.

## Setup wiki

Run once for initial onboarding or when a route is missing. In OpenCode use `/setup-wiki`; elsewhere follow this checklist directly.

- [ ] Inspect root/scoped agent guidance, manifests, current source, tests and the existing doc/diagram catalogs. Record dirty paths; preserve unrelated work.
- [ ] Inventory tracked paths with `git ls-files` and relevant untracked paths with `rg --files`; exclude generated output, secrets and scratch from authored maps.
- [ ] Create only missing hierarchy: `docs/index.md`, concept/code route indexes and focused domain maps. Keep domain references under `wiki/concepts/` or `wiki/code/` and use stable relative source links. Move dated evidence to `docs/archive/`; keep only one canonical explanation per topic.
- [ ] Each domain map states responsibility, source entry points, contracts, verification entry points and a link back to its route. Pair concepts with implementation links.
- [ ] Register a few stable cross-cutting tags with matching source comments. Keep root agent guidance short and link detailed rules.
- [ ] Identify diagrams that explain the navigation or domain; follow [diagram conventions](../diagrams.md), register figures and preserve HTML/SVG parity.
- [ ] Run the refresh review below before calling setup complete.

## Refresh wiki

Use `/refresh-wiki <scope>` in OpenCode, or follow these steps in any host. Scope can be a domain, changed paths or an explicit revision range; default to the current task's changed paths. Audit the full wiki only when explicitly requested. Review affected docs once before the implementation freeze, then revisit only relevant changed deltas, following the [scope and finish line](code/agent-workflow.md#scope-and-finish-line).

- [ ] Read `git status --short`, `git diff --name-status` and `git diff --cached --name-status`; for a revision range also inspect its diff. Include task-owned new files. Do not infer ownership from status alone.
- [ ] Map changes through source links and tags to both routes, reference docs and figures. For renames/removals, search old paths, symbols and tags across docs and agent guidance.
- [ ] For persistence claims, verify table/field names against the active `storage/` schema declarations and follow callers through the shared runtime. Old `schema_*.sql`, numbered SQLite migrations and migration-era source comments are not current schema authority; see the [storage map](code/storage.md#schema-and-runtime-authority).
- [ ] Read final source and relevant tests. Update responsibilities, links, contracts and diagrams only where affected. Label historical material; merge duplicated explanations into their canonical page and link to it. Avoid parallel catalogs or volatile tree counts.
- [ ] Verify affected relative paths, heading anchors and registered tags against maintained pages/source comments. Walk one concept → code → test path per affected domain.
- [ ] Compare diagram labels/arrows with source and paired SVG/HTML; visually inspect new/changed figures. Run appropriate checks from [contributor workflow](code/contributing.md); docs/comment-only edits do not need a Rust build.
- [ ] Update phase checkboxes, evidence and Next Step as each item finishes. Report updated pages, actual checks and unresolved stale areas. A no-op refresh reports its scope and why no edits were needed.

Setup and refresh update files; neither implies commit/push authorization. They do not install hooks, rewrite unrelated docs or regenerate historical evidence. The task's plan owner owns shared wiki updates; assigned editors report progress, and reviewers flag stale links/contracts as incomplete work.

## Continuous documentation with Scribe

[Scribe](../../.opencode/agents/scribe.md) reviews affected code changes at the agreed checkpoint before implementation freeze. Coder supplies the baseline, task-owned changed/new paths and exact doc ownership; Scribe reads current code/tests and updates affected canonical wiki pages, README links and diagram pairs. No-op reviews must explain why the inspected changes do not affect docs.

Review affected behavior, contracts, ownership, navigation and diagrams once before implementation freeze; batch related phases into that pass. Later fixes invalidate only the relevant review evidence. Do not repeat unchanged reviews or expand into unrelated wiki cleanup. Scribe returns concise evidence and open decisions through its caller; the plan owner updates shared phase checkboxes. Source comments/tags and agent/config changes remain coder-owned. Use the active host's roles; this is not an installed file watcher or unattended service.

Select Scribe directly for documentation work, use `/docs-review <scope>` for a code-delta review, or `/setup-wiki` and `/refresh-wiki`. Its default model matches tester; delegated Scribe calls retain that model unless the user overrides it. See [role permissions and handoffs](code/opencode-v2.md).
