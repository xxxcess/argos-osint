# TUI overhaul verification — 10 October 2026

Implemented the supplied `ARGOS_TUI_OVERHAUL_IMPLEMENTATION_SPEC.md` against baseline `864bffbe65170a0cde43fe979db5a656d2410ac7`. The change uses TabBar/ActionBar, ScrollPane, MeasuredEditor, TranscriptBlock, DetailTable and Overlay, preserving Profile Dashboard/Report geometry, all 20 primary metric IDs, theme tokens and authoritative investigation facts.

## Reproduction

Run formatting, strict Clippy and the offline workspace suite in that order, as documented in [TUI verification](tui-verification.md). All compilation uses `scripts/agent_cargo.py`, locked dependencies and no default features, with `ARGOS_EMBED` unset and temporary `ARGOS_HOME`. Local mock-provider sockets require sandbox permission. Agent artifacts remain in `target/agents`.

Create a task-owned Python environment with Pillow, then capture into fresh plan directories:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<plan>/tui/non-profile --test dump_overhaul_screens
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<plan>/tui/profile --test analytics_viewports_and_data_states
```

Non-Profile fixtures cover Home, all nine apps, three compact Tools pages, Configs draft/error, palette, help, full-record, memories, choice, Intel mode, fallback and resume overlays, transcript/history and article half-block imagery at 160×50, 120×40, 100×32, 80×24, 60×18 and 40×20. Profile covers all primary IDs, Dashboard/Report pages, seven viewports and empty/loading/stale/System states. Capture manifests record source fingerprints and rendered-cell hashes; capture itself does not certify visual or interaction correctness.

## Defects found and corrected

- First visual review found imagery occupying only its source pixel size. Background encoding now scales with contain fit to the terminal rectangle while preserving aspect ratio.
- Configs retained analytics footer/header hints after becoming a page. It now labels its own editor and actions.
- A new focus/activation test caught the shared arrow handler scoped to the fallback popup. It now routes focused tab rows independently of activation.
- Export overwrite confirmation was cleared before use. A second Export now confirms only the same resolved destination.
- Configuration validation locations now parse actual syntax line/column; exact editor revision gates Save and apply. Direct buffer changes and Ctrl+Enter cannot commit.
- Supporting Report datasets previously formed one unbounded text stream. They now have sticky headers, fixed visible row budgets and independent offsets, with selected-record prose in its own following section.
- Full-size Report review caught a gutter row counted as data: 11 rows instead of 10. Drawing now caps tall sections at 10 and short sections at 3, omitting the extra gutter when a section is clipped. The 240-record test asserts the exact initial `Rows 1–10 / 240` budget, verifies `Rows 1–3 / 24` at 60×18, then scrolls to the final record.

## Interaction and metric evidence

Tests distinguish focused inactive tabs from active pages, suspend analytics reads on System/Configs, restore category expansion after search, reject disabled palette dispatch, discard stale image completions, validate article HTTP(S) URLs, preserve paused transcript anchors above 65,535 rows, replace streaming synthesis without duplication, and retain manual folds. Report tests exercise 240 records and independent section offsets without expanding the card. Existing configuration rollback/security and metric cohort/coverage/low-N tests remain authoritative.

`graphify update .` refreshed the three prescribed tracked graph artifacts. It reports the existing missing `tree_sitter_sql` dependency for six SQL files and symbol-free data/module files; no SQL/core behavior was changed.

Final source check gate passed: `cargo fmt --all --check`; isolated workspace/all-target strict Clippy with `-D warnings`; isolated locked/no-default-features offline workspace tests. Results: **178 CLI/TUI + 709 core + 3 search fixtures = 890 passed, zero failures, eight ignored**; doc tests contain zero cases. The full gate was rerun after the Report row-budget correction. `git diff --check` also passed.

## Visual evidence

An image-capable viewer inspected all 162 non-Profile captures (run-02) and 60 Profile captures (run-03), using viewport contact sheets plus full-size Configs, Tools Test, Intel bulletin/brief, Recon and Report PNGs. Run-01 was superseded by image scaling and Configs hint fixes. Run-03 found an extra data row in Report tables; run-04 then exposed an unnecessary gutter reducing clipped tables to two rows. Final run-05 has 53 PNG hashes identical to reviewed run-03 and seven changed Report PNGs, all inspected again, plus full-size 120×40 and 60×18 confirmations of 10/3 rows. These corrections affect Report row count only; non-Profile buffers remain unchanged.

| Actual PNGs / sizes | Visual observation |
| --- | --- |
| `app-0` through `app-8`, `home`, six sizes | Shell, buttons and empty/loading states fit; wide layouts retain their columns. Compact layouts clip labels and retain navigation. |
| `configs-draft`, `configs-error`, six sizes | Three parent buttons, destination/Export, bordered editor and distinct Verify/Save actions. Tall editor exceeds eight visible lines. The minimum view reveals the caret and retains actions; errors do not expose credential values. |
| `tools-page-0/1/2`, six sizes | Category headers and selected tool remain distinct; compact Catalog/Documentation/Test buttons expose separate panes. Test keeps JSON and Run/Cancel/Raw controls visible. |
| `palette`, `help`, `record`, `overlay-*`, six sizes | Scoped borders and close controls fit. Palette query stays above nonselectable group headings. Long content has a continuation affordance. |
| `intel-image`, `intel-brief-image`, six sizes | Half-block gradient scales with contain fit; wide bulletin puts it left of preview, narrow bulletin stacks when height permits. Brief places image first and Visit/Reload below preview; lower sections remain scrollable. Very short bulletin height collapses the image. |
| `recon-transcript`, `recon-history`, six sizes | User surface/edge and Markdown headings/paragraph spacing are distinct. Wide Investigation occupies 30%; compact transcript remains dominant. Paused history exposes Jump to latest. |
| Profile page matrix, seven sizes | Three upper buttons, lower active marker, unchanged series colors/units and simultaneous wide panels. Compact pages crop fixed panels and scroll. 40×20 retains navigation and an explicit size notice. |
| `profile-empty`, `profile-loading`, `profile-stale`, `profile-system` | Unknown stays N/A; stale snapshot remains visible with failure status; System has Host/Paths controls and no analytics content. |

Selected actual buffers are retained in [screenshots/tui-overhaul-2026-10-10](screenshots/tui-overhaul-2026-10-10/). Their provenance manifests and hashes accompany the selection. Geometry/action tests certify reachability separately from these PNG observations.

## Limits

The supplied approved image reference files were unavailable; comparison uses the written design contract. PNG review uses an image-capable viewer, with Courier New rasterization of actual TestBackend cells. Courier New renders some CJK/emoji as missing-glyph boxes; cell-width/state tests pass, but terminal font rendering remains unverified. Native terminal Kitty/Sixel/iTerm graphics, OS browser launching, live providers, default-feature LanceDB and ignored MiniLM integration are separate checks and were not exercised. No real credentials or user state were used.
