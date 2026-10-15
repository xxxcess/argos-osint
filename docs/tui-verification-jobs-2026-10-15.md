# Jobs controls verification — 15 October 2026

## Scope

Jobs only: ActionBar, measured job table and scrollable detail preset. The table/detail split remains at body width 100; short bodies use one-row actions. Shared theme, shell, navigation and Profile geometry are unchanged. Recon has a data/publication guard, with no presentation changes.

Controls clear Completed, Stopped (cancelled/superseded) or Failed (failed/partial) history globally. Running descendants/tasks and services are protected. Stop job targets only the selected job and its owned work: it dequeues queued work, cancels scheduled retries and safely stops supported running jobs while retaining history. Unrelated running and queued jobs continue. Service detail distinguishes long-lived availability and age from individual task work.

## Reproduce

```sh
uv venv .agent-scratch/jobs-status-tui-env
uv pip install --python .agent-scratch/jobs-status-tui-env/bin/python Pillow
.agent-scratch/jobs-status-tui-env/bin/python scripts/tui_review.py capture \
  --output .planning/2026-10-15-jobs-status-controls/tui/run-01 \
  --test jobs_status_controls_and_service_screens --include 'jobs-*' \
  --reason "Jobs status clearing, stop/dequeue, compact controls, service explanation and loading/empty/retained-error/help states"
```

The focused exporter covers running jobs at 160×50, 120×40, 100×32, 80×24, 60×18 and 40×20; queued/retry-wait at 80×24; service detail at 160×50, 80×24 and 60×18; loading/empty/retained-error at 80×24; help at 80×24 and 60×18. No unrelated app capture is required.

## Visual review

All 16 fresh PNGs were opened and inspected. Wide 160/120 and breakpoint100 preserve bordered actions and the table/detail split without overlaps. Compact80/60/40 retain every clear action and Stop job. At40 columns, summary/footer and optional table detail abbreviate normally; full job detail remains available. Queued/retry-wait show the same Stop control. Service160/80/60 omit Stop and show the long-lived availability/service-age explanation. Loading/empty/retained-error80 preserve the actions and expected content state; Help80/60 shows the new commands and scrolling affordance. No output fixes were needed.

Retained evidence: [wide](screenshots/jobs-2026-10-15/jobs-running-160x50.png), [minimum](screenshots/jobs-2026-10-15/jobs-running-60x18.png), [40 columns](screenshots/jobs-2026-10-15/jobs-running-40x20.png), [queued](screenshots/jobs-2026-10-15/jobs-queued-80x24.png), [service](screenshots/jobs-2026-10-15/jobs-service-60x18.png), [retained error](screenshots/jobs-2026-10-15/jobs-error-80x24.png), [help](screenshots/jobs-2026-10-15/jobs-help-60x18.png). [Capture receipts](screenshots/jobs-2026-10-15/receipts.json) record source/renderer and all 16 fixture hashes, marking the seven retained PNGs. Capture ran one selected test successfully; its source hashes matched after review, before the test-only follow-up below. The original capture fingerprints remain unchanged in the receipts.

No supplied approved image reference was available; review used the maintained TUI design/component contracts. TestBackend images reconstruct actual terminal cells; no live-terminal claim is made. Timing values are synthetic fixtures, not a latency benchmark. Header counts are global Store facts; queued/service cases override the selected row for presentation checks. Existing core timing/cohort tests validate metric semantics separately.

## Interaction and data evidence

The Jobs fixture checks all clear controls and Stop job for keyboard focus and pointer parity at every listed viewport. It exercises clear-by-status, preservation of running jobs, stopped filtering and empty selection after clearing. Registry cancellation retains its cooperative behavior. Core tests cover queued/running cancellation, stale completion fencing, service exclusion, status separation, rollback and durable index/data preservation. Recon tests cover default-off transcripts, per-thread isolation, publication after explicit opt-in, and disabling Recall after dispatch without losing earlier opted-in memories.

The final `cargo fmt --all --check` and strict workspace Clippy passed. Offline `test --workspace --locked --no-default-features -- --test-threads=4` passed with ARGOS_EMBED unset and temporary ARGOS_HOME: 264 CLI/TUI tests, 1,022 core tests and three integration fixtures (1,289 passed, 18 ignored); doc tests passed with no cases. All agent Cargo work used `target/agents`. Live providers, default-feature LanceDB and ignored MiniLM checks are separate and were not requested.

The subsequent selected-job follow-up adds only regression assertions in `app.rs` and `job_cleanup.rs`, with no changes to application rendering or the screenshot exporter. `jobs_stop_targets_selected_job_only` exercises the pointer button and `c` shortcut on selected running, queued and retry-scheduled rows while checking that unrelated running/queued rows and tasks remain untouched. The expanded `stop_dequeues_or_fences_running_work_and_retains_history` checks that another running lease can still complete and another queued task can still be claimed. Follow-up formatting and strict workspace Clippy passed. Both named tests passed (one test per command, zero failures) with ARGOS_EMBED unset, temporary ARGOS_HOME and locked/no-default-features through `agent_cargo.py`; the TUI test used `test --workspace`, and the core test used `test -p argos-osint-core`. The earlier full-suite totals precede these test-only edits. Existing visual evidence is retained because this follow-up changes tests only.

## Compatibility and diagrams

No schema/config migration or version bump. Persisted cancelled/superseded states remain unchanged; the public `JobStatusFilter` adds `Stopped`, which downstream exhaustive matches must handle. Existing automatic cleanup APIs remain compatible. Jobs lifecycle and Recon turn HTML/SVG pairs now describe automatic retention, explicit terminal-status clearing, safe stopping and per-transcript Recall opt-in. Existing cancellation states and lease fences are reused.
