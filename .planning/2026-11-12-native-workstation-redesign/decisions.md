# Native workstation redesign — intake decisions

[Execution handoff](handoff.md) · [Source package](../../docs/design/native-workstation-handoff/README.md)

- Preserve all archive contents unchanged in `docs/design/native-workstation-handoff/` so the README's relative specification/mockup links remain valid. Keep proposed behavior distinct from currently implemented wiki contracts.
- Use actual intake date November 12, 2026 for this named plan. The source proposes a November 13 path and audit date; those are supplied metadata, not locally established facts. HEAD matches its pinned revision.
- Explicit user decisions take precedence, then the current handoff for execution/API migration and current design guide for UI. The original structural reference is historical input where it conflicts on workspace identity, Tauri/Linux clients, gloss, example hardware/data, Rust-BERT training or unrestricted logs.
- Keep shared execution/status and acceptance in curated `handoff.md`/`acceptance.md`; `task_plan.md`, `findings.md`, `progress.md` remain ignored local working memory. Do not force-add ignored files or alter other plans/global selectors.
- Plan contracts/migration before dependent clients and composition; retain one integration/build owner. Mark implementation and verification independently. No mockup counts as runtime evidence.
- This intake changes documentation only. Canonical behavior docs and diagrams are updated with implementation so they continue describing the actual product meanwhile. No product tests/builds or deployment were requested or performed.
