# Intel claim context and topology verification

Fresh actual Ratatui TestBackend captures, visually inspected on 2026-10-16. The source checkout started at `19321d1d9ef2b2d6fc4aafef6e8c47635819bb28`; `receipts.json` identifies the changed Rust sources and retained image hashes. Fixtures use fictional offline data.

## Visual results

All 14 selected PNGs were inspected with an image viewer. The retained pair demonstrates distinct contracts:

- [Completed Claims after inference failure](claims-published-after-inference-failure-160x50.png): Actors precede Claims. Claims remain published when Inferences fail and Links remain blocked. Index pending is intentional: this fixture commits canonical Claims but supplies no vector acknowledgment.
- [Selected Inference at 80×24](selected-inference-80x24.png): scrolling reveals the complete selected record, including Why and Limits; focus highlighting follows its measured wrapped extent.

The record-order and selection states were reviewed at 160×50, 120×40, 100×32, 80×24, 60×18 and 40×20. Long Actor lists intentionally place Claims below the scroll viewport. Each selected Inference remained completely visible, with readable Unicode and no new overlap. The existing Intel compact layout remains usable at 40×20. The separate topology canvas minimum-size policy is unchanged.

The failure state was also reviewed in compact Context/Jobs at 80×24. Its failure remains readable; the lower publication footer falls outside the existing short Jobs pane, so that footer is visually verified only in the wide capture. The supplied Global Jobs error screenshot established the original failure context; it was not a matching approved pixel baseline. Review used the maintained TUI design and component contract.

## Interaction and data results

`cargo fmt --all --check`, strict workspace Clippy and offline workspace tests passed, in that order. Cargo ran through `scripts/agent_cargo.py`, with `--locked --no-default-features` and `ARGOS_EMBED` unset. Results: 269 binary tests, 1,088 core tests and 3 fixture tests passed; zero failures and 18 ignored tests. Both capture tests subsequently passed.

Tests cover typed Claim/Inference topology entry, keyboard/click parity, complete-record reveal, Back focus/scroll restoration, canonical Actor/Claim/Inference/Related/article associations, indexed reverse premise traversal, carried-source identity collisions, all three scoped publication points, empty results, retries, fencing and exact-revision index status. Model mocks verify at least one analysis call per Claim, bounded classifier inputs, selective evidence, retained counterevidence and cancellation/retry behavior.

Metric review is separate: Profile metrics were outside scope; confidence scoring, theme and existing metrics were preserved. No live provider latency/quality, live terminal, default-feature LanceDB or MiniLM integration checks were performed.

## Regeneration

Use Python with Pillow installed and the repository's renderer fonts available. Run capture commands sequentially; each uses the isolated agent Cargo cache. Choose fresh output directories:

```sh
python3 scripts/tui_review.py capture \
  --output .planning/2026-10-16-intel-claim-inferences/tui/entries-fresh \
  --test dump_topology_density_and_entries_screens \
  --include 'intel-topology-claims-*' \
  --include 'intel-topology-inference-tail-*' \
  --size 160x50 --size 120x40 --size 100x32 \
  --size 80x24 --size 60x18 --size 40x20 \
  --reason 'Intel Actors-first order and selected inference reveal across existing layout thresholds'

python3 scripts/tui_review.py capture \
  --output .planning/2026-10-16-intel-claim-inferences/tui/publication-fresh \
  --test intel_sequential_stage_screens \
  --include 'seq-4-inferences-failed-160x50' \
  --include 'seq-4-inferences-failed-80x24-jobs' \
  --size 160x50 --size 80x24 \
  --reason 'Completed Claims publication despite later inference failure'
```

Generation alone does not complete visual review. Unrelated apps, Profile views, shared shell geometry and topology canvas layout were unchanged and excluded.
