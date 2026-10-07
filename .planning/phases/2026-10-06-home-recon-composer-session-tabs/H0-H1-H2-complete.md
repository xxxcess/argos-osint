# H0-H2 Completion Summary

## H0: Resolve Harness and Current Phase ✓
- Verified OpenCode configuration:
  - Plan agent: google/gemini-3.1-pro-preview (high variant, no edits/bash)
  - Build agent: google/gemini-3.1-pro-preview-custom-tools (high variant, edit/bash allowed)
  - Explore agent: google/gemini-3.5-flash-lite (no edits, bash allowed)
- GSD state exists at commit 345689b
- Graphify graph exists with 5190 nodes, 13216 edges (last built at 467dc39, 2 commits behind)
- Current work tree clean with active phase being 2026-10-06-argos-tui-spec-implementation
- Ready to proceed to H1 exploration

## H1: Map Feature Seams ✓
- **Home Layout & Composer**: 
  - `home_rows()` (ui.rs:467-518) - builds home screen layout
  - `draw_home()` (ui.rs:4149-4216) - renders home screen  
  - `composer_height()` (ui.rs:57-63) - calculates composer height
  - `composer_parts()` (ui.rs:146-150) - splits composer area
  - `chrome()` (ui.rs:75-96) - defines layout regions
- **Draft Persistence**:
  - `flush_draft()` (app.rs:6269-6277) - saves draft to persistence
  - `draft_dirty` flag (app.rs:643) - tracks draft changes
  - Multiple persistence calls during navigation
- **Investigation Creation**:
  - `new_thread()` (recon.rs:540-556) - creates new investigation thread
  - `.new_thread()` (app.rs:1754) - app-level wrapper
  - `send_recon()` (app.rs:1762) - sends recon command
  - `enter_investigation()` (app.rs:1711) - enters investigation
- **Navigation**:
  - `go_home()` (app.rs:1209-1215) - returns to home
  - `set_focus()` (app.rs:1615-1629) - sets UI focus
- **Test Seams**:
  - `home_order_renames_and_nine_routes_agree()` (app.rs:7940)
  - `home_offers_recon_and_brain_and_only_recon_has_chat()` (app.rs:7551)

## H2: Freeze Contracts and Packets ✓
- **Home Draft Key**: Separate draft storage per Argos data/profile scope, independent of investigation drafts
- **Launch State Machine**: Editable → Accepting → Accepted with investigation/run reference OR RecoverableFailure
- **Persistence Boundary**: Reuse existing transaction/checkpoint from draft persistence with submission token for duplicate prevention
- **Tab Identity**: Durable investigation ID (Thread.id) as tab key
- **Navigation Intent**: Track last user navigation to prevent focus theft during async acceptance
- **State Restoration**: Define per-tab vs global state ownership to avoid duplication
- **Input Precedence**: Completion/overlay → Focused control → App action → Global command
- **Layout Measurements**: Baseline title row, apps row, launch block bounds, footer row, minimum blank-row reserve

## Next Step: H3 - State and Persistence Foundation (Build Agent)
Ready for Build agent to implement:
- Home draft ownership mechanisms
- Stable tab identity system  
- Launch acceptance contract with durable references
- Minimal additive persistence for submission tokens
- Test seams for state/crash validation