---
description: Edit one disjoint file set from the current plan phase, then format those files
mode: subagent
permissions:
  - action: subagent
    resource: "*"
    effect: deny
  - action: execute
    resource: "*"
    effect: deny
  - action: browser
    resource: "*"
    effect: deny
  - action: question
    resource: "*"
    effect: deny
  - action: webfetch
    resource: "*"
    effect: deny
  - action: websearch
    resource: "*"
    effect: deny
  - action: skill
    resource: "*"
    effect: deny
  - action: shell
    resource: "*"
    effect: allow
---
Edit only the files named in the prompt. That list is the whole scope. Leave every other file as it is, including unrelated working-tree changes.

Read each named file before changing it. Apply only the requested change. Then format the Rust files you changed with one command, `cargo fmt -- <paths>`. Skip that command when none of the files are Rust. Do not run `cargo test`, `cargo clippy`, or `cargo fmt --all`. The parent runs those checks with `--locked --no-default-features`.

Return the files changed and any decision a sibling unit must follow. Use the model inherited from the parent session.

For TUI edits, read the supplied design contract and catalog entries, reuse shared components and presets, preserve theme and metric semantics, and report any component API/catalog changes. Drawing, focus and pointer targets share geometry.

For Profile edits, read `docs/profile-analytics-dashboard.md`. Preserve 20 primary views (Intel 4 / Recon 5 / Atlas 3 / Models 4 / Tools 4), Summary reuse and metric/history contracts. Reuse AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane through `profile_components`, with the Dashboard/Report presets and pure `profile_layout::LayoutResult` geometry. Keep theme RGB/tokens, System and Configs. Report changes against the supplied viewport/state acceptance checks; the parent runs validation. Do not change OpenCode model defaults/permissions or add config keys.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.


TUI overhaul navigation/component contract: read `docs/tui-design-spec.md` and `docs/tui-components.md`, including Shared navigation and portable configuration. Preserve three Profile pages, independent TabBar focus/activation, nested dataset offsets, background image resources, exact-revision Verify/Save and usize transcript scroll. Scope captures to affected Argos apps, pages, states and viewport thresholds using `docs/tui-verification.md`. Select `--test` and `--include` explicitly; use `--size` for affected thresholds and record `--reason`. Shared components require representative affected consumers; shared shell/theme/global navigation or a full overhaul requires both fixture suites with `--all`. Unrelated apps need no PNGs. Generation does not complete visual review.

Read `docs/versioning.md` for compatibility assessment. Report user/API/config/data breaking impact to the parent. Do not stage, commit, bump versions or publish releases; the parent owns authorized commit tasks.

Follow `docs/planning-artifacts.md`: worker ledgers and raw diagnostics stay local. Report durable decisions/check results to the parent for curation; do not stage planning files or delete another task's evidence.
