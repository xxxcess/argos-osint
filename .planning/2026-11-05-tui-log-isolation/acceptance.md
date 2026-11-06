# TUI diagnostics and live Atlas Coverage

## Delivered behavior

- Venture keeps application-TUI child stderr off the interactive terminal while preserving redirected files, CLI logging and captured launch-failure notices. Its stderr reader finishes before the next launcher screen.
- During the application TUI, first-party warning/error traces become bounded, redacted `runtime.diagnostic` events in the existing Logs store. Queue scheduling and asynchronous storage writes suppress recursive diagnostic capture. The existing ordered queue and shutdown drain own the writes.
- Terminal-output suppression starts before fallible raw-mode setup and remains active through terminal restoration.
- Atlas emits committed classification coverage before metadata, and committed native coverage after each stage before announcing the next stage. Pause and resumed post-metadata runs refresh durable counts. Repeated snapshots do not count retries as new output. Publisher coverage distinguishes pending metadata from settled unavailable lookups.
- Live TUI regression fixtures deliver successive Stats/Status events and render classification and actor-stage counts at 140×44 and 100×36. Layout, theme, admission policy, storage schema and source/ownership fences are unchanged.

The previously completed Atlas implementation and agent guidance are committed as `4ef1140`. This follow-up is separate. The unrelated local `.opencode/opencode.json` model selection is excluded.

## Verification

Status: pending final focused verification. Do not interpret planned commands below as passed.

Focused native checks use `target/agents`, default embedded/NLP/classifier features, locked dependencies, two build jobs, two test threads and private homes:

```sh
python3 scripts/agent_cargo.py test --workspace --lib --locked -- \
  logging::tests \
  child_stderr_respects_terminal_ownership_and_keeps_failure_details \
  logs_record_errors_durably_and_scroll \
  logs_clear_and_filters_reject_stale_in_flight_snapshots \
  logs_clear_drains_queued_lines_and_late_writes_mark_dirty \
  atlas_live_coverage_shows_article_metadata_units \
  live_coverage_reconciles_committed_cohorts_without_counting_retries
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked -- -D warnings
```

One explicit fresh isolated replay exercises the public Atlas cycle and asserts classification/metadata/publisher/Related snapshots are visible before actor processing. Set `ARGOS_HOME` and `ARGOS_BENCH_HOME` to the same new absolute temporary directory, `ARGOS_BENCH_MODE=fresh` and `ARGOS_BENCH_CALIBRATION=off`, then run:

```sh
python3 scripts/agent_cargo.py test -p argos-osint-core --features fixtures \
  --test classifier_cycle_benchmark --locked offline_fixed_whole_cycle_benchmark \
  -- --ignored --exact --nocapture
cargo fmt --all -- --check
```

This is functional replay evidence, not a new comparable performance cohort. The earlier canonical benchmark results describe the implementation committed in `4ef1140`.

The prior complete native gate belongs to that earlier implementation. It is not represented as having executed these later fixes. The follow-up uses focused behavior, public-cycle, Clippy, formatting and visual checks; the full native matrix has not been repeated. Server/all-platform matrices, local mutation execution, live providers and opt-in embedding checks are not run for this follow-up.

Maintained contracts: [CLI/TUI interface](../../../docs/wiki/code/interfaces.md) and [Atlas completion/recovery](../../../docs/wiki/concepts/atlas-classification-and-related-articles.md#completion-recovery-and-compatibility). Existing diagrams remain accurate because stage ownership and data flow are unchanged.

## Applying the change

The existing user-owned Atlas/Venture processes are preserved. Rebuild and restart Venture to use the fixes; committing source does not replace an already running binary.
