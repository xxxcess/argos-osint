# H3 Task Packet: State and Persistence Foundation

## Task ID and Goal:
**H3 — State and persistence foundation**
Implement Home draft ownership, stable tab identity, launch acceptance contract, and minimal additive persistence for submission tokens with test seams.

## Prerequisites and Approved Decisions:
- H0-H2 completed (harness resolved, feature seams mapped, contracts frozen)
- Home draft: Separate storage per Argos data/profile scope (independent of investigation drafts)
- Launch state: Editable → Accepting → Accepted (with investigation/run reference) OR RecoverableFailure
- Persistence boundary: Reuse existing draft persistence mechanism + submission token for duplicate prevention
- Tab identity: Durable investigation ID (Thread.id) as tab key
- Navigation intent: Track last user navigation to prevent focus theft during async acceptance
- State restoration: Define per-tab vs global state ownership (avoid duplication)
- Input precedence: Completion/overlay → Focused control → App action → Global command
- Layout measurements: Baseline references for title/apps/composer positioning with responsive fallbacks

## Effective Agent / Model / Variant:
Build / google/gemini-3.1-pro-preview-custom-tools / high variant

## Spec Sections and Invariant IDs:
- Sections 3-9 (Product decisions through Recovery and quality-of-life details)
- Section 10.1 Task H3
- Section 10.2 Required design decisions (items 1-8)
- Section 10.5 Verification ownership (Build owns terminal verification)
- Section 11 Validation scenarios (all 14 scenarios relevant to state/persistence)
- Section 12 Acceptance criteria (items 1, 4, 6, 7, 10, 12, 13)

## Source Paths and Symbols Already Inspected:
- `crates/argos-osint-bin/src/tui/app.rs`: 
  - `App` struct (L582), `draft_dirty` flag (L643), `flush_draft()` (L6269)
  - `go_home()` (L1209), `set_focus()` (L1615), `enter_investigation()` (L1711)
  - `send_recon()` (L1762), `new_thread()` wrapper (L1754)
  - `ModuleId` enum (L37), `Target` enum (L455)
  - Test functions: `home_order_renames_and_nine_routes_agree()` (L7940)
- `crates/argos-osint-bin/src/tui/ui.rs`:
  - `home_rows()` (L467-518), `draw_home()` (L4149-4216)
  - `composer_height()` (L57-63), `composer_parts()` (L146-150)
  - `chrome()` (L75-96), `draw()` (L3829-3934)
- `crates/argos-osint-core/src/recon.rs`:
  - `Store::new_thread()` (L540-556), `save_draft()` (L592-594)
  - `Thread` struct (L198), `Run` struct, `Call` struct
  - `Store` persistence methods (save/get thread, run, call, message)
- `crates/argos-osint-core/src/recon/orchestrate.rs`:
  - Investigation orchestration entry points

## Files This Worker Owns:
1. `crates/argos-osint-bin/src/tui/app.rs` - Primary modifications for:
   - Adding HomeDraft storage (separate from investigation drafts)
   - Adding tab strip state (Vec<Thread.id>, order, last active, closed history)
   - Implementing launch state machine (enum/App state field)
   - Adding submission token mechanism for duplicate prevention
   - Implementing tab switch/open/close/reopen logic
   - Navigation intent tracking (last user action during acceptance)
2. `crates/argos-osint-bin/src/tui/ui.rs` - Secondary modifications for:
   - Home tab strip rendering (below header)
   - Tab anatomy (title, status indicators, close action)
   - Overflow/history switcher UI
   - Responsive layout adjustments for tab strip
3. `crates/argos-osint-core/src/recon.rs` - Minimal modifications for:
   - Potential helper methods for tab state persistence (if needed)
   - Ensuring Thread.id stability for tab keys
4. Test files (to be determined) - State/seam test additions

## Existing APIs / Data Models to Reuse:
- Existing durable draft mechanism (`save_draft`, `draft_dirty`)
- Existing investigation persistence (`Store::new_thread`, `save_draft` for Thread)
- Existing run/message/call persistence (`Store::new_run`, `save_draft` for Run/Call)
- Existing command registry and input handling
- Existing theme and layout primitives (`chrome()`, `Constraint`, etc.)
- Existing focus system (`Target`, `set_focus()`)
- Existing overlay system (`Overlay` enum)
- Existing keyboard handling (`handle_key`)

## Required Behavior Including Failure Transitions:
### Home Draft Management:
- On App init: Load Home draft from persistent storage (if exists)
- On editor change: Set `home_draft_dirty = true`, debounce save
- On navigation away from Home: Flush Home draft (save if dirty)
- On returning to Home: Load Home draft (restore cursor, context)
- On successful launch: Consume Home draft exactly once (clear after persistence acceptance)
- On launch failure: Preserve Home draft (do not clear)
- On draft persistence failure: Show "Draft not saved" (do not silently promise recovery)

### Launch State Machine:
```
[Editable] --(submit attempt)--> [Accepting]
[Accepting] --(validation fail)--> [RecoverableFailure] 
[Accepting] --(persistence success)--> [Accepted]
[Accepting] --(persistence fail)--> [RecoverableFailure]
[RecoverableFailure] --(user fix)--> [Editable]
[Accepted] --(launch accepted)--> [Editable] (draft consumed)
```

### Tab Management:
- Home tab: Permanent, first, not closable (ID: reserved/sentinel value)
- Investigation tabs: Keyed by Thread.id, ordered by last active
- Open tab: Visual indication (selected/focused state)
- Close tab: Removes from open set, preserves investigation (jobs continue)
- Reopen tab: Restores most recently closed valid investigation (bounded history: 20)
- Duplicate prevention: Opening existing investigation reuses existing tab
- Overflow: Horizontal window around active tab when not all fit; searchable switcher

### Submission and Duplicate Prevention:
- On submit attempt: Capture draft revision + create submission token
- Local validation: If fails, stay in Accepting state, show error, preserve draft
- Persistence attempt: Atomic operation creating:
  1. New Thread (investigation)
  2. First Message (user prompt)
  3. Pending Run Request
  4. Submission token record
- On persistence success: Resolve investigation ID, open/deduplicate tab, navigate to Recon
- On persistence failure: Stay in Accepting state, preserve draft, show error
- Duplicate prevention: Disable resubmission for same draft revision while acceptance pending
- Crash recovery: On restart, reconcile submission tokens with persisted investigations

### Navigation Intent:
- Track last explicit user navigation action during Accepting state
- If user navigates away during acceptance: Do not auto-navigate on completion
- Instead: Show quiet notification "Investigation started — Open" with action to open
- If user stays: Navigate immediately after durable acceptance

### State Restoration:
- Per-tab owned state (stored with tab):
  - Draft content and cursor position
  - Transcript anchor (chat scroll position)
  - Context-pane expansion state
  - Unread watermark position
- Globally owned state (not duplicated per tab):
  - Application module (Intel, Recon, etc.)
  - Global UI settings (theme, layout preferences)
  - System-wide flags (error counts, etc.)

### Input Precedence:
When processing input:
1. If completion popup/overlay active: Route to completion handler
2. Else if focused control (composer, input field): Route to focused handler
3. Else if app action (numeric shortcut 1-9): Route to app switcher
4. Else if eligible global command (/, :, etc.): Route to command processor
5. Else: Ignore or show help

## Acceptance Checks and Exact Existing Test Targets:
### State/Crash Tests to Pass:
- Draft isolation: Home → Inv A → Inv B → Home restart → each draft correct
- Launch failure recovery: Validation/persistence failure preserves draft
- Crash during acceptance: Restart reconciles token, connects draft or restores unconsumed
- Tab persistence: Open/close/reopen preserves per-tab state (draft, scroll, context)
- Duplicate prevention: Rapid double-submit creates one investigation; later identical submit creates another
- Navigation intent: Focus stolen during acceptance → quiet notification, no auto-navigate
- Overflow handling: 20+ tabs shows searchable switcher, active tab reachable
- Cross-app entry: Opening same investigation from multiple sources reuses tab
- Draft consumption: After successful launch, Home draft cleared exactly once

### Exact Existing Test Targets to Reuse/Extend:
- `home_order_renames_and_nine_routes_agree()` (app.rs:7940)
- `home_offers_recon_and_brain_and_only_recon_has_chat()` (app.rs:7551)
- Existing draft persistence tests (if any in test suite)
- Existing investigation creation tests
- Existing tab/switching tests from prior TUI work

## Out of Scope:
- UI rendering details (positioning, colors, animations) - handled in H5/H6
- Specific tab status indicators (Running, Queued, etc.) - handled in H6
- Keyboard shortcuts for tab navigation - handled in H6
- Example prompts action - handled in H5/H6
- Model role metadata display - handled in H5
- Slash command handling in Home composer - handled in H5
- Attach context (@) and report mode reuse - handled in H5
- Visual positioning verification - handled in H5/H6
- Integration with other apps (Jobs, Logs, etc. for tab switching) - handled in H7
- Restart reconciliation beyond tab state - handled in H7

## Current Diff / Prior Changes to Preserve:
- Current work tree clean (no uncommitted changes)
- Preserve all existing TUI functionality from phase `2026-10-06-argos-tui-spec-implementation`
- Preserve existing draft persistence mechanism
- Preserve existing investigation creation and orchestration
- Preserve existing command registry and input handling
- Preserve existing theme and layout system
- Preserve existing focus and overlay systems

## Return: Changed Files, Commands and Exit Results, Unresolved Risks, Next Task
### Expected Changed Files:
1. `crates/argos-osint-bin/src/tui/app.rs` - Primary state/logic changes
2. `crates/argos-osint-bin/src/tui/ui.rs` - Tab strip rendering and layout
3. `crates/argos-osint-core/src/recon.rs` - Possible persistence helpers (minimal)
4. New test files for state/seam validation (to be determined)

### Expected Commands:
- `cargo build` (to verify compilation)
- Focused tests for state/persistence logic
- `cargo test --workspace` (verification gate)

### Unresolved Risks:
1. Tab state persistence volume: Ensure we don't bloat storage with excessive tab history
2. Submission token cleanup: Need mechanism to purge old tokens to prevent DB growth
3. Focus tracking accuracy: Ensuring we correctly detect when user steals focus during acceptance
4. Tab history bounds: Implementing the bounded 20-ID history for reopen feature
5. Integration with existing overlay system: Ensuring tab strip works correctly with popups

### Next Task:
Upon successful completion of H3 (state/crash tests pass), proceed to H4: Launch vertical slice (Home submit creates normal investigation + opens Recon; mock delayed/failing acceptance & duplicate-submit tests pass).