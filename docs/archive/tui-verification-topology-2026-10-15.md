# Seeded topology and list density verification

> Historical record. For current behavior, start at the [wiki](../index.md).

Implementation baseline: `78dea7c`. The user approved the Graph first interactive
demonstration, then requested its production layout and components. The
[maintained specification](../wiki/concepts/topology-mapper.md) defines the resulting contract.

## Scope

Affected components: topology GraphCanvas, inspector, relationship ScrollPane,
measured actions, compact TabBar, Brain graph entry panes, typed Intel claim/actor
records, and explicit dense `ListGeometry` consumers. Preset: Topology / Graph
first. The shared shell/theme and spaced constructor defaults are preserved.
Profile geometry and metric semantics are unaffected; no Profile PNG sweep is
required. The workspace gate still covers its existing tests.

Topology captures cover 160×50, 132×40, 131×40, 120×40, 100×40, 100×34,
100×33, 100×32, 80×24, 60×20, 60×18 and 40×20. These test both width ratios,
the 32-body-row split boundary, the smallest supported body and too-small
guidance. Graph, selected edge, Controls and initial loading use every size;
isolated, stale, unavailable and crowded cases additionally use 160×50, 80×24
and 60×18. Home/palette/Atlas/Intel density and Intel typed entries use the six
standard sizes 160×50 through 40×20. Brain claim/actor graph origins use the same
six sizes.

## Visual review

All 126 final selected PNGs received actual image review: 60 mapper states,
54 density/Intel entries and 12 Brain graph origins. The approved interactive
Graph first reference supplied the panel hierarchy, proportions, controls and
selection/recenter behavior; terminal cells require shorter labels and compact
pages. Eight representative PNGs and final code/artifact hashes are retained in
[screenshots/topology-2026-10-15/receipts.json](../screenshots/topology-2026-10-15/receipts.json).

| Contract | Final observed result |
| --- | --- |
| Approved wide hierarchy | Dominant canvas left, selected inspector and spaced relationships right; seed independent of selection. [160×50](../screenshots/topology-2026-10-15/topology-graph-160x50.png). |
| Adaptive modest graph | All nine fixture nodes fit at120×40 and80×24; no dangling connectors. [120×40](../screenshots/topology-2026-10-15/topology-graph-120x40.png). |
| Compact edge inspection | True source/predicate/target, tentative state and both endpoint actions remain visible; receipts scroll. [80×24 Details](../screenshots/topology-2026-10-15/topology-edge-80x24.png). |
| Minimum supported body | All actions remain reachable at60×20 terminal; ancillary Controls coverage prose clips within its pane. At60×18 terminal, Back and accurate size guidance remain. [Minimum Controls](../screenshots/topology-2026-10-15/topology-controls-60x20.png). |
| Requested density | Home, palette, Atlas lists and Intel headlines have no inter-item blank rows. Section gaps and ordinary records remain spaced. [Home](../screenshots/topology-2026-10-15/density-home-80x24.png), [Atlas articles](../screenshots/topology-2026-10-15/density-atlas-articles-80x24.png). |
| Exact typed entries | Wrapped Unicode claims retain complete text and record highlights; metadata actors survive absent claims and tail actors reveal fully. [Intel Claims](../screenshots/topology-2026-10-15/intel-topology-claims-60x18.png). |
| Brain origin clarity | Compact graph rows remain visible before metadata that would fill the pane. [Claim path](../screenshots/topology-2026-10-15/brain-claim-split-60x18.png). |
| State truthfulness | Isolated roots show one node/zero relationships; initial loading/error show no graph; matching stale snapshots remain explicit; crowded columns report offscreen nodes and pan guidance. |

At 131×40 every measured inspector action remains visible. The 100×34 terminal
splits its 32-row body; 100×33 stacks. At 40×20, the mapper shows size guidance;
other existing compact labels/Atlas cycle timestamps ellipsize within bounds.
The final U+250A renderer correction affects only topology connectors; all 54
density/Intel PNGs were rerendered and remained byte-identical.

Initial actual-buffer PNG inspection found and corrected:

- A modest nine-node graph overflowed at 120×40 and 80×24. Adaptive four/two/one
  row node presentations now fit modest hop columns; crowded columns retain a
  pannable extent. Edges to hidden nodes no longer leave dangling stubs.
- Inspector actions could wrap beyond their guessed height at 131×40. Required
  rows now follow the measured labels, preserving every action.
- Initial loading/unavailable states incorrectly described a retained snapshot;
  placeholders and footer state now distinguish initial and refresh cases.
- The minimum notice now distinguishes 60×18 content cells from a 60×20 terminal
  including the shared shell.
- Wide Unicode claim text lost trailing words during character-count wrapping.
  Typed Intel rows now use terminal-cell measurement before hit-range layout.
- Compact Brain Claim headers could consume the entire graph viewport. In that
  case graph records precede the same metadata, which remains scrollable.
- The PNG reconstruction font lacked graph arrow/dash/Inference symbols and CJK
  text. Geometric graph symbols, optional recorded CJK font fallback and
  background-first painting preserve those cells in the review images. This
  does not certify every native terminal font.

## Interaction evidence

PNG review assesses appearance; separate binary tests exercise exact Brain and
Intel entry, metadata actors without claims, raw claim identity, measured wrapped
targets, recenter versus selection, edge endpoint history, Back and deleted
origins, stable focus after reorder, same-seed option races, hidden-app status,
filters, independent tabs/scrolls, wheel/Page panning and dense-list pointer rows.
Representative regressions:

- `topology_entries_recenter_and_generation_are_scoped`
- `topology_intel_claim_uses_raw_identity_and_clicked_origin`
- `topology_intel_metadata_actor_entry_restores_brief`
- `topology_back_checks_its_owner_and_handles_deleted_origin`
- `topology_endpoint_history_preserves_edge_and_original_focus`
- `topology_rejects_obsolete_same_seed_request_options`
- `topology_refresh_rebinds_stable_focus_and_preserves_other_app_status`
- `topology_graph_wheel_and_pages_pan_the_same_canvas`
- `intel_topology_records_preserve_wide_unicode_and_trailing_words`
- `list_spacing_populated_screens`

## Data evidence

Core SQLite fixtures assert distinct disconnected seeds, real depth 1/2/3,
isolated/missing roots, canonical versus explicit name groups, direct article
mentions/provenance without Related pairs, visible Inferences and topic/category
projections, exact source revisions and stable direction across entry seeds.
They also cover publication owner/job/local-ID collisions, metadata-only actors,
stale artifacts, cycle filtering before limits, bounded read/output/path closure,
truthful omissions, continuation rejection and mutation sequence. Scope analysis
rejects retained projections as canonical premises. Reverse typed actor payloads
remain usable when the optional index row is absent. User claims marked
`unassessed` remain unassessed in the inspector.

These fixtures verify IDs, receipts and graph closure, not just counts. No live
provider activity or analytics metric interpretation is inferred from screenshots.

## Reproduce

Run fmt, strict Clippy and the offline workspace gate in that order, with
`ARGOS_EMBED` unset. Use `python3 scripts/agent_cargo.py` and locked dependencies
without default features. Local mock-provider sockets require host permission in
restricted environments.

Use a Python environment with Pillow and a fresh output directory for each run:

```sh
python3 scripts/tui_review.py capture --output .planning/topology-review/mapper --test dump_brain_topology_screens --include 'topology-*' --size 160x50 --size 132x40 --size 131x40 --size 120x40 --size 100x40 --size 100x34 --size 100x33 --size 100x32 --size 80x24 --size 60x20 --size 60x18 --size 40x20 --reason 'Seeded Graph first mapper, split thresholds, compact controls and data states'
python3 scripts/tui_review.py capture --output .planning/topology-review/entries --test dump_topology_density_and_entries_screens --include 'density-*' --include 'intel-topology-*' --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 --reason 'Requested density exceptions and exact typed Intel entry records'
python3 scripts/tui_review.py capture --output .planning/topology-review/brain --test dump_brain_kind_screens --ignored --include 'brain-actor-graph-*' --include 'brain-claim-split-*' --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 --reason 'Brain graph-pane entry origins and compact path visibility'
```

Inspect emitted PNGs with an image-capable viewer and record the ledger; capture
generation leaves review pending. Manifests record source/cell/PNG hashes, font,
renderer and isolated Cargo paths. Selected durable evidence is linked above.

## Validation and limits

Final formatting and strict workspace/all-target Clippy passed. The final offline
workspace run passed with four test threads: binary 263 passed/10 ignored,
core 1010 passed/8 ignored, integration 3 passed, doc tests 0; 1,276 passed in total.
The capture assertions also passed for all three selected exporters, including
the explicitly ignored Brain origin fixture. The screenshot tooling regression
(`node --test .opencode/tests/tui-review.test.mjs`), final source/PNG receipt
hash checks, HTML/SVG parity/XML validation and `git diff --check` passed.

An earlier unrestricted parallel run passed the binary suite but failed two
existing brief-stage heartbeat/lease timing assertions. Each passed in isolation;
the complete final run with four threads passed both. Their implementation/tests
were not changed. The final claim-assessment correction and every other source
change are included in that final full run.

The final workspace command was:

```sh
env -u ARGOS_EMBED ARGOS_HOME=/private/tmp/argos-topology-check python3 scripts/agent_cargo.py test --workspace --locked --no-default-features -- --test-threads=4
```

Live terminal interaction, live providers, default-feature LanceDB and ignored
MiniLM integration have not been exercised.
The demo and fixtures use fictional data. `graphify update .` completed with the
existing missing SQL-parser/unsupported-symbol warnings; only the prescribed
graph artifacts are retained. The existing topology HTML and SVG are consistent
and describe seeded reads separately from explicit scope analysis writes.
