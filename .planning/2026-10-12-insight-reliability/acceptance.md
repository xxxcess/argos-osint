# Insight extraction reliability acceptance

The change replaces serial per-item assessments with bounded ID-addressed batches,
retains complete source coverage, and separates immutable checkpoints from explicit
retry epochs. It was developed against baseline `79d9dd2` for the supplied Argos
Insight Extraction Reliability Fix spec. No application version was manually changed.

## Compatibility

Schema 32 is additive and retains schema-31 checkpoints, report bundles and unrelated
tables. CLI/configuration and the 72-hour operational-history policy remain compatible.
New fields are `OperationScope.insight_contract` (false for ordinary operations),
`ExecuteOptions.attempt_limit` (None), `ChainReport.terminal_failure`/`stopped`
(None/false), `JobDetail.insight_progress` (empty vector), and
`PipelineOutput.item_gaps`/`withheld_count` (empty vector/zero).
Public Rust struct literals must initialize them; prefer `OperationScope::new` and
default-based literals where available. This is a breaking core source change and
therefore a minor release impact during 0.x; Release Please owns the eventual bump.

## Reproduction

Keep `ARGOS_EMBED` unset and use a disposable `ARGOS_HOME`. Agent artifacts use
`target/agents`; the user's normal TUI build directory is untouched.

```sh
cargo fmt --all --check
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings
python3 scripts/agent_cargo.py test --workspace --locked --no-default-features
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features --features nlp -- -D warnings
python3 scripts/agent_cargo.py test --workspace --locked --no-default-features --features nlp
python3 scripts/agent_cargo.py test -p argos-osint-core --locked --no-default-features --features lancedb store::publication::tests
python3 scripts/agent_cargo.py test -p argos-osint-core --locked --no-default-features --features nlp investigation::insight_reliability_tests -- --nocapture --test-threads=1
```

Mock HTTP tests require local socket access.

## Executed checks

- Formatting passed. Strict workspace/all-target Clippy passed both without default
  features and with NLP; the diagnostic-only timing correction also passed NLP Clippy.
- Final offline workspace run: 195 TUI, 793 core and 3 integration tests passed;
  9 intentionally ignored. The full NLP workspace run passed 995 tests with 9 ignored.
  Its final identity-role delta was separately verified by all 10 reliability fixtures
  with NLP enabled, after strict Clippy in both modes.
- Four maintained HTML/SVG pairs are synchronized and parse as valid SVG XML:
  Intel report, investigation harness, persistence and Recon turn. Labels and arrows
  were checked against the final coverage, failure, retry and publication paths.
- The prescribed graph update completed (9,487 nodes, 23,387 edges); generated JSON
  parses and all edge endpoints resolve. Only graph.json, manifest.json and
  GRAPH_REPORT.md are retained in Git.
- Shared version consistency passed at 0.1.0. The breaking commit message checker
  passed with minor release impact, and staged whitespace checks passed before commit.
- Separate LanceDB publication lane: all 12 tests passed using deterministic
  embeddings, including exact ID/revision verification, disabled/failed embedding
  recovery, concurrent edits/deletion, damaged-row repair and crash after vector write.
  Its test build emitted the existing unused test-only `publication_fault` reexport
  warning; this is not a certificate of default-feature Clippy or real MiniLM.

## Contracts exercised

- One-window report: eight claims, five inferences and eight proposed links complete
  in five actual outbound requests. Thirteen canonical Brain rows, one active snapshot
  and thirteen index-outbox rows commit; compatible warm continuation dispatches zero.
- Overlapping multi-window report: deduplication/caps precede assessment, full
  extraction and Synthesis coverage includes the late denial, and reconciliation
  avoids the former oversized aggregate merge. Every measured wire body fits budget.
- Invalid output consumes one precise repair, then a declared fallback; ordinary
  validator failure remains terminal. Compact citations reject unknown IDs, invalid
  UTF-8 boundaries, stale artifacts and native-pointer coordinate errors.
- Same-name batches retain Entity Resolver routing and their original question
  thresholds. Its route participates in article/Recon work identity while report-only
  extraction ignores unrelated resolver changes.
- Optional failure preserves qualified work with withheld diagnostics. Explicit retry
  reuses completed checkpoints, retries only exhausted work once per epoch, and never
  turns reopening or restart into a new retry. Relevant captured-route changes admit
  replacement; unrelated role changes do not.
- Recovery isolates a failed owner; fencing, rollback, cancellation, stale revisions,
  user edits, tombstones, report-only replacement and additive migration remain covered.
- Intel actions resolve the currently selected owner even when displayed progress is
  stale. Jobs exposes extraction retry separately from exact-revision index retry.

## Measured requests and timing

The final serial NLP reliability fixture run passed all 10 tests. Fixtures explicitly
disable preprocessing and embeddings to isolate wire/checkpoint behavior. Outbound
sizes are serialized HTTP JSON bodies, not prompt estimates or headers. Measured
durations below are local mock transport/validation attempt durations, not provider
compute-only latency; queue duration is a separate persisted field.

| Fixture | Requests | Body sizes in bytes | Queue total | Request total | Cold / warm |
| --- | ---: | --- | ---: | ---: | --- |
| One-window report, 8/5/8 | 5 | 2706, 23322, 5971, 18111, 12655 | 70 ms | 90 ms | 1756 ms / 369 ms |
| Same report, compatible warm continuation | 0 | none | no dispatch | no dispatch | zero new wire attempts |
| 66,456-byte report, nine complete windows | 29 | range 6404–44798; all ≤49152 | 504 ms | 421 ms | cold source coverage + settlement |

The larger fixture performs nine extraction, nine Synthesis, nine reconciliation and
two assessment requests. Its nine merge attempts measured 153 ms of queue duration
and 122 ms of request duration in total. The five-request report requires no separate
merge. End-to-end cold/warm durations also include database/checkpoint/preparation and
publication work; they are not the sum of provider attempts. No production speedup
is inferred from these deterministic mocks.

## Terminal review

The fresh `insight_reliability_screens` TestBackend capture passed its fixture test,
emitted 15 valid cell dumps/PNGs, and every PNG was opened with an image-capable viewer.
Captured source hashes match the final source files. Components are Intel Briefing's
Inferences ScrollPane, Jobs DetailTable/detail ScrollPane and action buttons, and the
Intel command palette entries; presets are Intel Briefing and Jobs list/detail.

Visual: failed and partially published states were reviewed at 160×50, 80×24 and
60×18, together with both palette actions at each size. Wide and compact diagnostics
wrap without crossing cards or the footer. At 60×18 the initial Intel view shows
stage/withheld/index progress and Jobs wraps Retry extraction onto a visible second
toolbar row; longer diagnostics remain below the initial fold in existing scroll
panes. Palette action labels remain complete while descriptions use normal ellipses.
No new overlap was observed. Existing unavailable-time and empty-list conventions
remain intact.

Interaction: the fresh capture asserts retry label, focus membership and mouse hit
targets at all three sizes. The final workspace also passed the selected-owner action
guard and existing Intel stacked-section scroll, Jobs and navigation tests. Core
fixtures separately verify that retry executes only authorized unfinished work.

Metrics: screen fixtures use synthetic progress counts, empty datasets and unavailable
timings; they certify formatting rather than real cohorts. Canonical row/outbox counts
and exact index revisions are independently verified by core/vector fixtures.

Intel 160×50 and 80×24 image-column references from 2026-10-11 were inspected for
theme/navigation conventions. No exact approved failure/partial-publication or Jobs
reference exists; those states were checked against the written design/component
contracts. Profile/shared shell/theme geometry did not change, so no Profile/global
capture sweep was required. Logs rendering is unchanged and linked diagnostics are
covered by existing tests. Live terminal/provider state was not exercised.

Selected unchanged PNGs from that reviewed run:

- [Partial publication and pending index](../../docs/screenshots/insight-reliability-2026-10-13/insight-intel-published-80x24.png)
- [Failed extraction and its retry](../../docs/screenshots/insight-reliability-2026-10-13/insight-jobs-failed-80x24.png)
- [Extraction actions at minimum width](../../docs/screenshots/insight-reliability-2026-10-13/insight-palette-60x18.png)

Using the Python/Pillow environment described in [TUI verification](../../docs/wiki/code/tui-verification.md),
reproduce into a new directory:

```sh
python scripts/tui_review.py capture --output .planning/2026-10-12-insight-reliability/tui/run-new \
  --test insight_reliability_screens --include 'insight-*' \
  --size 160x50 --size 80x24 --size 60x18 \
  --reason 'Insight extraction progress and retry controls in Intel/Jobs and palette'
```

## Limits

Measurements use deterministic local mock responses and disposable databases, not
live providers or a user's failed production owner. They establish request bounds
and checkpoint behavior, not production speedup. Real MiniLM download/inference and
live terminal/provider recovery are not claimed. SQL graph parsing is unavailable
in the installed graphifier; Rust structure, migration tests and maintained diagrams
are checked separately.
