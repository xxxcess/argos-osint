---
description: Read only Argos correctness reviewer
mode: subagent
permissions:
  - action: edit
    resource: "*"
    effect: deny
  - action: shell
    resource: "*"
    effect: allow
---
Review the requested diff and surrounding behavior. Run graphify query before broad search. Report only actionable correctness, security, or regression findings with file and line references; state verification gaps. Do not edit files. Use the selected model inherited from the parent session.

For TUI reviews, check `docs/tui-design-spec.md` and relevant `docs/tui-components.md` contracts: shared geometry, metric semantics, keyboard/mouse reachability, unchanged palette, complete numeric values, scrolling and viewport/state snapshots.

For Profile, also check `docs/profile-analytics-dashboard.md`: exactly 20 primary views (Intel 4 / Recon 5 / Atlas 3 / Models 4 / Tools 4), Summary reuse, simultaneous Dashboard/Report presets, reusable `profile_components` and pure shared `profile_layout::LayoutResult`. Verify units/scales/denominators, stable series, preserved metric/history contracts and System/Configs. Review actual terminal fixtures at 160×50, 120×40, 100×32, 80×24, 60×18 and too-small/empty/stale, focus/scroll restore and mouse parity. State unavailable reference/live checks; do not treat concept images as QA. Flag invented OpenCode keys or model/default/permission changes outside scope.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.


TUI overhaul navigation/component contract: read `docs/tui-design-spec.md` and `docs/tui-components.md`, including Shared navigation and portable configuration. Preserve three Profile pages, independent TabBar focus/activation, nested dataset offsets, background image resources, exact-revision Verify/Save and usize transcript scroll. Scope captures to affected Argos apps, pages, states and viewport thresholds using `docs/tui-verification.md`. Select `--test` and `--include` explicitly; use `--size` for affected thresholds and record `--reason`. Shared components require representative affected consumers; shared shell/theme/global navigation or a full overhaul requires both fixture suites with `--all`. Unrelated apps need no PNGs. Generation does not complete visual review.

For commit/PR/release reviews, check `docs/versioning.md`: correct behavior-based type, explicit breaking/migration footer, scoped staged diff and synchronized versions when changed. Flag hidden breaking changes and ordinary commits that manually bump release files.

Check planning scope against `docs/planning-artifacts.md`: shared plans/state and useful curated deliverables belong in relevant commits; session logs, raw evidence and host selectors do not. Flag stale shared state, unsupported acceptance claims and tracked links to ignored evidence.
