# Brain browsing and Intel Focus Brief verification

Implementation baseline: `13c635bd5377eae836ec22b494619e7d6ba68254`. The reviewed change introduces Claims/Actor-name browsing, read-only graphs, typed association destinations, retired article Recon execution and ingest-authoritative Related associations with two distinct shared entity mentions. Investigation detections require Recall on; original Claim Sources retain their provenance.

## Scope and reproducibility

Affected components: Brain browse list, `brain_detail::RelatedView`, graph/summary panes and Intel's clipped Full Article/Related cards, compact TabBar and independent scrolling. Presets: Claim graph above Related/Summary; Actor Graph/Related at 65%/35%; Intel at 24%/48%/28%. No Profile geometry or metric registry changed.

Fresh `TestBackend` captures covered 160×50, 120×40, 100×32, 80×24, 60×18 and 40×20. The final selection contained 84 Brain, 30 Intel Related and 36 Intel body/Actor PNGs. All selected PNGs received visual review through an image-capable viewer. Brain's final capture regenerated all 84 cases: 70 cell and PNG hashes matched the previously reviewed capture exactly, and 14 changed cases received fresh inspection. Those changes cover Actor Graph focus/counts, compact Claim actor titles and loading animation frames. Subsequent source changes affected only Intel reader geometry and its empty-body fixture. Final Intel Related captures followed the geometry correction; the body/Actor capture matched the executable source hashes before the final Back-offset correction described below. Raw working evidence was consolidated here; nine representative PNGs and hashes are retained in [screenshots/brain-intel-2026-10-14](screenshots/brain-intel-2026-10-14/receipts.json).

Run each capture into a new local directory. Use a Python environment with Pillow; captures isolate Cargo artifacts in `target/agents`, remove `ARGOS_EMBED` and use temporary state. The capture fixture assertions run even for excluded images.

```sh
python3 scripts/tui_review.py --test dump_brain_kind_screens --ignored --include 'brain-*' --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 --reason 'Claims/Actor browsing, graph, tabs, paging, empty/loading/error and compact states' --output .planning/brain-intel-review/brain
python3 scripts/tui_review.py --test dump_atlas_related_screens --include 'intel-*' --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 --reason 'Related beneath Full Article, scope, paging, loading/error and help' --output .planning/brain-intel-review/related
python3 scripts/tui_review.py --test dump_article_insights_screens --include 'intel-body-*' --include 'intel-actors-*' --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 --reason 'Full Article borders and independent scrolling; Actors without claims and reachable tail' --output .planning/brain-intel-review/body
```

## Visual results

| Contract | Reviewed result / retained example |
| --- | --- |
| Claims and unique Actor-name rows | All contains their union; Actor rows show local counts and disabled aggregate mutations. [Wide list](screenshots/brain-intel-2026-10-14/brain-list-all-160x50.png). |
| Claim graph and summary | One Topic, only Claim/Topic/Entity/Article/Source labels, retained tree marks and attribution; Summary remains separate. [Claim split](screenshots/brain-intel-2026-10-14/brain-claim-split-120x40.png). |
| Full Actor associations | Graph contains 45 article associations without the former 40 cap; related lists page with totals. [Actor graph](screenshots/brain-intel-2026-10-14/brain-actor-graph-100x32.png), [compact Articles](screenshots/brain-intel-2026-10-14/brain-actor-tab2-80x24.png). |
| Compact selected titles | Selected actor title remains above its reason; no orphaned reason or See more/text collision. [40×20 Claim Actors](screenshots/brain-intel-2026-10-14/brain-claim-tab0-40x20.png). |
| Intel center stack | Full Article precedes Related; focus reveals Related while retaining both card headers. [Wide](screenshots/brain-intel-2026-10-14/intel-related-articles-160x50.png), [compact Reader](screenshots/brain-intel-2026-10-14/intel-related-articles-60x18.png). |
| Body states | Empty, loading and acquisition-error cards keep their border and explicit state. Long body text reserves See more space. [Long body](screenshots/brain-intel-2026-10-14/intel-body-bordered-long-80x24.png). |
| Extracted Actors | Actors remain available without Claims; scrolling reaches the final Actor and Links. [Actor tail](screenshots/brain-intel-2026-10-14/intel-actors-tail-60x18.png). |
| Empty/loading/error Related | Clear state text, retained Intel scope control, no invented fallback rows. Theme colors and focused borders retained. |

At 40×20, navigation/action labels and status hints truncate within the viewport; section tabs and keyboard routes remain reachable. This is a compact layout, not a too-small placeholder.

Historical styling comparators inspected: `docs/screens/brain-graph/p4-brain-actor-graph-120x40-after.png` and `docs/screens/intel-sequential/after/seq-5-completed-160x50.png`. They support retained Argos tree, color, panel and confidence styling. No approved reference for the replacement layout was supplied; the written spec governs the intentional layout changes.

Review corrections: Actor aggregates initially inherited the Claim title; local totals were absent; short Related panes could show only a reason; See more could overlap long text; compact Actor Graph fixtures focused Related; Intel's revealed Related clipped the Full Article header. Each affected case was fixed and recaptured. The PNG renderer now draws the actual ★/◆ cell symbols when the platform monospace font lacks them.

## Interaction results

The workspace suite exercises keyboard and pointer routes separately from screenshots: Claims/Actors filtering, aggregate mutation guards, each typed Related destination, independent tab offsets, page expansion, cross-app Back, stale exact-article revisions, no graph-node navigation, Full Article scrolling/See more and Related rows scrolling before the center stack. In particular, `brain_thread_association_back_restores_the_original_group_and_tab` verifies the original snapshot after another Brain detail was visited; `intel_related_pages_its_rows_before_the_middle_stack` verifies row-boundary handoff. Final review added `intel_related_back_restores_the_original_full_article_offset` and preserved that offset in the article Back snapshot. This changes return-state restoration without changing the captured fixture buffers.

## Data and lifecycle results

Core fixtures cover normalized-name grouping without identity merges, redirects, hidden kinds, two-actor claim refs, more than 40 associations, Recall-on/off receipts, deleted threads, exact duplicate-URL occurrences and selective/idempotent backfill. Related fixtures reject single-entity/repeated-name overlap, stale revisions and view-time name/canonical shortcuts. Historical report admission/restart is rejected; retirement fences leases and descendants without backfilling unrelated owners. Cached validated briefs republish once under publication policy version 2, reuse stage/fidelity outputs and retain prior canonical data on rollback.

Confidence fields and source-only caps remain unchanged. No Profile metric semantics changed.

## Validation and limits

Final formatting and strict standard Clippy passed. Standard workspace tests passed: binary 222 passed / 10 ignored, core 984 passed / 8 ignored, integration 3 passed, doc tests 0. NLP strict Clippy and full workspace tests passed: binary 223 passed / 10 ignored, core 989 passed / 8 ignored, integration 3 passed, doc tests 0. The final navigation correction is included in this NLP workspace run. After that correction, formatting, strict standard Clippy and the complete standard binary suite passed again (223 passed / 10 ignored); standard core source was unchanged from the passing full workspace run.

Live terminal interaction and live provider requests were not exercised. Tests use local fixtures/mock HTTP services; default LanceDB and ignored MiniLM integration were not run. `graphify update .` completed with missing SQL-parser and unsupported-symbol warnings; the prescribed graph artifacts are refreshed. Updated figures preserve paired HTML/SVG content and describe Brain browsing, two-entity Related matching, brief publication and the historical report lifecycle.
