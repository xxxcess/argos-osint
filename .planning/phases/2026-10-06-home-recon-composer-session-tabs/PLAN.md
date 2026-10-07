# Phase Plan: Home Recon Composer and Investigation Tabs

## Goal

Implement the Home Recon Composer and persistent Investigation Tabs as specified in `ARGOS_HOME_RECON_COMPOSER_AND_SESSION_TABS_SPEC.md`. This feature builds on the merged OpenCode-inspired TUI redesign (phase `2026-10-06-argos-tui-spec-implementation`) and extends it with:

1. **Home Composer**: A new-investigation composer on the Home screen that creates and launches a Recon investigation in a single action
2. **Persistent Investigation Tabs**: A horizontal tab strip (Home + open investigations + `+`) on Home and Recon, inspired by OpenCode's session tabs but adapted to Argos's investigation data model

## Baseline

- Current active phase: `2026-10-06-argos-tui-spec-implementation` (completed TUI redesign)
- Key files from baseline:
  - `crates/argos-osint-bin/src/tui/app.rs` — main app state, module switching, draft persistence
  - `crates/argos-osint-bin/src/tui/ui.rs` — layout (chrome, header tabs, home rendering, composer)
  - `crates/argos-osint-bin/src/tui/theme.rs` — shared theme tokens
  - `crates/argos-osint-core/src/recon.rs` — Store, Thread, Run, Message, Call persistence
  - `crates/argos-osint-core/src/recon/orchestrate.rs` — investigation orchestration

## Scope from Spec (Sections 1–9)

### 1. Outcome
- User opens Argos, types investigation question on Home, submits once → creates/starts normal Recon investigation
- Opens investigation immediately in Recon, retains as named session tab
- Home stays useful as nine-app launcher + starting point for next investigation

### 2. Home Layout (Section 4)
- Vertical hierarchy: Header → Tab strip → Title/Subtitle → Applications → Gap → "New investigation" label → Composer → Guidance → Open space → Footer
- Title moves up 1–2 rows only; apps move up 3–5 rows by reducing gaps
- Composer 1–2 rows after apps, 5–7 rows total (3 editable + chrome + metadata)
- ≥4 blank rows below composer to footer at 32+ rows; ≥2 at 24–31 rows
- Responsive fallbacks for 24–31 rows, 16–23 rows

### 3. Home Composer (Section 5)
- Reuse existing Recon composer component with "launch mode"
- Heading: "New investigation", Placeholder: "What would you like to investigate?"
- Action: "Start investigation", shows Recon + Synthesis models
- One dedicated Home draft per Argos data/profile scope (independent of investigation follow-up drafts)
- Autosave via existing durable draft mechanism
- Attach context (@), report mode, optional model overrides, example prompts action
- Slash commands: help, models/config, context attachment, investigation switcher

### 4. Submission & Transition (Section 6)
- Single shared launch command (Home Send, Start investigation, palette)
- Capture draft revision → local validation → persist investigation + first message + pending run → resolve ID → open/deduplicate tab → navigate to Recon transcript
- Show initial message + Queued/Starting state immediately; run scheduler async
- Consume Home draft only after durable acceptance
- Duplicate prevention via submission token + atomic transaction/outbox
- Failure boundaries documented (validation fail, persistence partial, nav fail, provider fail, crash during acceptance)
- Focus during async acceptance: navigate if user hasn't moved; otherwise quiet notification

### 5. Persistent Investigation Tabs (Section 7)
- Horizontal strip below header: Home (permanent, first, not closable) → investigation tabs → `+` → overflow
- Tab = view into existing Recon history (keyed by durable investigation ID)
- Other apps hide strip; preserve open set
- Tab anatomy: short title, status indicator, unread/unsent markers, close action
- Status states: Running, Queued, Needs attention, Failed, Idle, Unread result, Unsent draft
- Open/switch/close/reopen semantics with draft preservation
- Overflow: horizontal window around active tab, searchable switcher, compact at <80 cols
- Persist open IDs, order, last active, recently closed (bounded 20) per Argos data/profile scope
- Startup: Home with draft focused; reconcile missing/deleted on restart

### 6. Keyboard & Mouse Contract (Section 8)
- Preserve Argos keybinding model; every op has command-registry entry
- Key mappings for composer, tab strip, app shortcuts, navigation
- Focus order: tab strip → applications → composer → aux controls → footer
- Ctrl+C on Home clears draft, doesn't cancel background investigation

### 7. Recovery & QoL (Section 9)
- Missing provider/model → config message near composer
- Missing attachment → title + Remove/Resolve
- Scheduler saturation → Queued state
- Consumed prompt doesn't reappear after restart
- Monochrome-friendly labels/glyphs
- Terminal-control sanitization for titles
- Report mode + model overrides visible in Recon after launch
- Deep link return restores tab + reading position
- Global "work needs attention" → Jobs, not composer

## Dependency-Ordered Task Graph (Spec Section 10.1)

| Task | Owner | Depends On | Deliverable / Gate |
|------|-------|------------|-------------------|
| **H0** — Resolve harness & current phase | Plan | None | Effective routing/permissions matrix; commit/dirty-state note; available GSD/Graphify; approval state |
| **H1** — Map feature seams | Explore | H0 | Source-backed map for composer, Home layout, create/send, draft persistence, navigation, tests; no edits |
| **H2** — Freeze contracts & packets | Plan / Pro; ECC if useful | H1 | Named interfaces, launch state transitions, storage reuse decision, file ownership, task checks; approved scope |
| **H3** — State & persistence foundation | Build | H2 | Home draft ownership, stable tab identity, launch acceptance contract, minimal additive persistence, test seams; state/crash tests pass |
| **H4** — Launch vertical slice | Build | H3 | Home submit creates normal investigation + opens Recon; mock delayed/failing acceptance & duplicate-submit tests pass |
| **H5** — Home composition & input | Build | H4 | Title/apps/composer layout, bottom reserve, input focus, paste handling, metadata; measured reference renders & input checks pass |
| **H6** — Complete tab working set | Build | H5 | Tab strip, switch/close/reopen, overflow, status/read/draft markers, per-tab restoration; identity/navigation tests pass |
| **H7** — Integration & recovery | Build | H6 | Cross-app links, restart reconciliation, model-setup return, no focus theft, deleted/stale records; integration tests pass |
| **H8** — Independent review & final verification | ECC reviewer from Plan/Pro; Build fixes | H7 | Actionable findings resolved/reported; required checks, graph update, acceptance matrix, handoff |

## Required Design Decisions Before Editing (Spec Section 10.2)

H2 must pin these contracts using actual source symbols:

1. **Home draft key**: one draft per Argos data/profile scope; no overlap with investigation follow-ups
2. **Launch state**: editable → accepting → accepted with durable investigation/run reference, or recoverable failure. Late acceptance tied to captured draft revision
3. **Persistence boundary**: reuse current transaction/checkpoint path if it can enforce duplicate prevention; document additional receipt/migration if needed
4. **Tab identity**: durable investigation ID; Close changes open set only
5. **Navigation intent**: delayed launch completion navigates only if user hasn't deliberately moved elsewhere
6. **State restoration**: define owners of cursor, draft, transcript anchor, context selection, unread watermark; avoid storing twice
7. **Input precedence**: completion/overlay → focused control → app action → eligible global command
8. **Layout measurements**: baseline title row, apps row, total launch-block bounds, footer row, minimum blank-row reserve at each reference size

## Verification Ownership & Order (Spec Section 10.5)

- Build owns terminal verification; Plan and ECC reviewer assess evidence
- For packet checks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, focused tests in repo order
- Before completion, run exact checked-in `/ecc-verify` gate from Build:
  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  graphify update .
  ```
- Keep `ARGOS_EMBED` unset for offline workspace tests
- Use deterministic fixtures for queued/running/failing sessions and crash-boundary cases
- Capture Home render evidence at 120×40, 100×32, 80×24, 60×18
- Graphify update required after code changes

## Validation Scenarios (Spec Section 11)

14 scenarios covering: visual positioning, single-action launch, duplicate activation, draft isolation, input safety, launch failures, background close, overflow, cross-app entry, no focus theft, responsive editing, configuration recovery, session restoration, keyboard parity

## Acceptance Criteria (Spec Section 12)

14 checkboxes covering: Home retains 9 apps, title/apps/composer positioning, focused composer, new-investigation targeting, single submit creates durable investigation, transition shows queued/running state, persistent tabs with switching/status/overflow/close/reopen, tab deduplication, Close ≠ Cancel ≠ Delete, draft survival, app-switch bindings intact, resize/error handling, no bypass of job limits/retries/evidence/memory/history

## Current Work Tree State

- Phase `2026-10-06-home-recon-composer-session-tabs` initialized
- Ready to proceed with H0/H1.
