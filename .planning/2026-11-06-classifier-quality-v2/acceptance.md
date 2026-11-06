# Handoff disposition — user requested wrap-up

The user ended further verification after the prolonged session. No full green gate is claimed.

Implemented in the workspace: the classifier specification, independent derivative scheduling, optional Venture dataset-change retraining, and subagent guidance. The late cleanup repair and its restart/rejection regression are written. The repair compiled through core, then its broader build was interrupted at the user's request (exit130); the focused regression and final benchmark reruns did not execute. A final readiness-audit adjustment is formatted but uncompiled. The selected agent binary is not certified current. No user service was restarted or model activated.

Earlier evidence: all73 canonical embedded targets accounted for,274 actual behavioral cases passed across reused/resumed runs,1 manual benchmark ignored,2 server-only cases inapplicable. This predates the cleanup repair. Earlier format/Clippy passed before that repair. The unchanged recon cancellation test failed33.05514ms against15ms. Synthetic quality limitations below remain.

The outstanding work is the focused cleanup regression, current-source checks, final binary/CLI refresh and two native measurements. Work is stopped; these are not passes.

---

# Implementation acceptance record

Status: final verification in progress; no green full gate is claimed. The separate required embedded run completed59 outer targets with exit0:210 harness-reported passes represent208 executed behavioral cases and two embedded-mode early returns. Together with14 reused targets/66 actual cases, the audit accounts for all73 canonical targets and274 actual behavioral cases on the compiled revision before the superseded-publication cleanup repair. That production repair and its new regression remain pending verification; this coverage does not certify the final source. Final selected-binary CLI refresh and exact-binary resource measurements remain pending.

The full single-script gate has not passed. The unchanged recon cancellation case failed in isolation at33.05514ms against its15ms bound. Seven involved source files equal HEAD, but an independent HEAD binary was not executed. The earlier required storage run was interrupted with exit130 after preserved failures; repaired cases have since passed focused checks. A completed target, a mode-selected skip and an interrupted run are recorded separately.

## Verification ledger

| Scope | Actual evidence | Current disposition |
|---|---|---|
| Classifier dataset/features/ranking/training/readiness |17 /18 /7 /19 /12 passed; learning3 and manual repair2 passed | Relevant classifier inputs unchanged by later storage-reader repairs |
| Review/import/approval/durable fitting |5 passed,138.72s in workspace native execution | Actual embedded behavior; reuse recorded in coverage audit |
| Native admission/pagination/recovery |7 passed,476.44s | Actual embedded behavior on prior compiled revision; cleanup changes require affected refresh |
| Historical raw checkpoint preservation |5 passed,109.80s in required-mode run | Stored JSON/cursor/state preserved; public readiness overlay tested separately |
| Dossier/topology/independent derivatives after reader repair |7 /5 /3 passed,36.52s /13.89s /29.05s | Reader fixes verified together before later cleanup repair |
| Indexed current actor window/cursor |1 passed,108.39s |62 actual native occurrences, window50, Unicode and validated body-change continuation |
| API/CLI/TUI behavior |API8, stack CLI6, CLI/TUI library334 passed;6 library fixtures ignored | Unaffected contract/render inputs reused; ignored fixtures are not passes |
| Format/Clippy after dossier repair |Format passed; all-target workspace Clippy passed3m46s | Prior compiled revision; final cleanup source refresh pending |
| Canonical standalone CLI smoke |Earlier pass334.06s before the small dossier repair | Final selected binary refresh pending |
| Separate required embedded contract coverage |59 outer targets PASS,210 reported/208 actual cases;14 reused targets/66 actual cases | All73 targets accounted for;274 actual cases before cleanup repair; one manual benchmark ignored |
| Superseded-publication cleanup |Production repair and actual A→B/cancel/reject/reopen/C regression prepared | Pending integration/execution; no final-source behavioral pass claimed |
| Native resource/convergence measurements |Earlier measurements below | Provisional until final binary identities/reruns are recorded |
| Recon cancellation latency |Isolated33.05514ms versus unchanged15ms limit | Failed; no source or limit adjustment |

Coverage is audited in `.agent-scratch/classifier-v2-remaining-verification-completion.json`, preserving the original target audit at `.agent-scratch/classifier-v2-remaining-verification.json`. The completed run is `.agent-scratch/classifier-v2-required-embedded-remaining.log` (exit0,1592.26s). Its59 outer targets comprise58 behavioral targets plus the embedded-mode early return for `storage_server`. The authentication case in `storage_contracts` also returns before server-only behavior; those two reported passes are excluded from208 actual cases. The manual `storage_curated_links::offline_fixed_focus_benchmark` remains ignored. Embedded portions of storage_open/runtime/contracts executed, while their network/server-specific branches remain outside scope. Nested child harness summaries, doc tests, mode skips and zero-match companion targets do not inflate these counts.

## Implemented behavior

- Versioned v2 corpus, strict four-partition validation and artifact hashes; frozen v1 remains explicit and unchanged.
- Shared Natural unigram/bigram pipeline, Train-only vocabulary, sparse count parity, bounded explanations and original source spans; neutral-only phrases and hard-line adjacency are excluded.
- Eligible-domain ranking admits one/two supported domains, records ambiguous primary ranking and abstains instead of inventing dominance. Legacy receipts remain historical evidence.
- Calibration-only isotonic/sigmoid fitting with independent Development selection and explicit synthetic provenance; actual review export/import/approval, durable fitting, measured promotion and rollback guards.
- Admission commits durable independent derivative jobs. Collection can finish while Actors/Reconcile remain pending. Per-home scheduling uses one existing serialized publisher, independent leases/checkpoints, bounded dispatch, foreground fairness and startup replay.
- Native generation filtering hides stale relationships. A final audit found that supersession could strand already committed cleanup; preserving its runnable owner/frontier and blocking newer actor publication is under repair and unverified. Current admitted denominators, actor/reconciliation readiness and index outbox remain separate.
- Venture verifies selected dataset bytes against the active seed hash and prompts for optional retraining. Declining keeps the usable model and service access.

## Fixed identities

- Dataset SHA-256: `625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93`.
- Natural bundle SHA-256: `db11da2ea61182ed36bfcd207b5b0fb6310b110430638ce47b0e37ecdb792646`.
- Source manifest SHA-256: `a22805bbbb7b9c8e962b1a95a0dacdfdc22f63d533c27df4a84c8d3eb23119b1`.
- Default native model payload SHA-256: `70803eacdd6f1a3188d3bf23c9ebb66463f527802553d814a27b61ea8b158557`.
- Frozen gate SHA-256: `7015c59c87de6c028fc361159099647f58c16a633dc1eb17acab13d645a51ff3`.
- Native host x86_64-apple-darwin; embedded storage; default embedded,nlp,classifier plus workspace fixtures; target/agents debug; CARGO_BUILD_JOBS=2. CPU speed limits20–58% were observed during the session; cross-run times do not establish a speedup.

## Development ablations

Train440, Calibration230, Development165 distinct synthetic families. All variants use identical partition bytes; old Bayes is the frozen HEAD bundle/bridge retrained on the same Train corpus. Calibration is fit only on Calibration. Both admission policies are evaluated. Full per-domain confusion, tie frequency, uncertainty and sigmoid comparisons: `.agent-scratch/classifier-quality-v2/ablations/evaluation.json`.

| Variant | Macro F1 | Coverage | False OOS | Invalid selected sets |
|---|---:|---:|---:|---:|
| old_bayes | .93954 | .8970 | 1 | 8 |
| corrected_unigrams | .93210 | .9091 | 2 | 14 |
| plus_bigrams (selected pipeline) | .93661 | .8909 | 1 | 11 |
| plus_vocabulary_support (minDF2) | .91706 | .8727 | 1 | 9 |
| plus_field_features | .88548 | .8121 | 1 | 7 |

No uniform quality improvement is demonstrated. MinDF2 and field namespaces are not adopted. Isotonic remains selected over sigmoid by Development evidence. The corpus remains synthetic and cannot establish production quality. Development contains no three-plus-domain ground truth; its coverage does not provide that estimate.

## Native resources and Test disclosure

Earlier post-feature-fix native Development measurement PASS1/1 (provisional exact-binary provenance pending): fit8.594s; cold restore2.001361s;63 warm singles p50=6.320ms/p95=6.795ms; warm batch64 p50=224.370ms/p95=231.703ms. External process peak RSS110149632bytes; inner16.58s. This is the default1020-row workload, not a worst-case memory guarantee or whole-UI latency measurement. Logs: `.agent-scratch/classifier-v2-postfix-native-development.{log,json}`. Node five-variant maxRSS261939200bytes is aggregate V8 diagnostic evidence, not native per-model memory.

The original Test was evaluated after the initial freeze. A later contract audit found neutral-phrase and hard-line correctness defects. The fixes were independent of Test labels; there was no Test-driven parameter selection. Original artifacts remain archived. The subsequent185-family run is explicitly regression evidence, not fresh blind validation: `.agent-scratch/classifier-v2-test-regression.json` and `pipeline-freeze.json`.

Current Test regression PASS1/1: macroF1=.9240279668, coverage=.8648648649, false OOS1, invalid selected sets14, selected true memberships189/203, singles retained106/120. Governance34/39=.8718 and environmental35/40=.875 fail the unchanged90% precision rule. Four other domains pass point precision/sample counts; synthetic provenance still fails reviewed-real production approval.20 three-domain families yielded11 valid selected subsets. A future time-separated real Test cohort remains necessary.

## Derivative convergence and bottleneck evidence

Actual native three-occurrence cohort: two occurrences share two catalog actors; one yields zero actors. Both schedules use identical model/source bytes and real metadata/Related prerequisites. A correction retires old native mentioned_in links, retains shared actors through the remaining source and removes the old Related article pair. Semantic inventories, mentions and Related rows converge equally. This does not exercise tentative Brain Related between distinct unconfirmed identities.

Earlier generation-zero cohort PASS1/1 (before later reader repairs; final exact-binary rerun pending): `.agent-scratch/classifier-v2-generation-zero-cohort.log` (37.15s inner/47.77s process; peak process RSS275386368bytes). CohortSHA `68cb5a06283b7ae09a94bcf1dba77445c224fae3ce033bae6cc4aa92b70c94ac`; fixture modelSHA `50179a550359df1f19982ced681128e6a1912f11325f5b9837997668eb58bc32`.

- Admission/handoff: synchronous1290ms, independent1285ms.
- After shared prerequisites: synchronous ready3690ms; independent first actor850ms, actor-ready2441ms, reconcile-ready7007ms over9 activations.
- Independent correction convergence2818ms over3 activations.
- Earlier synchronous post-prerequisite frontier100 probes/600 SQL statements/1.493128s;6 publisher batches/3.181132s execution; max queue wait106us. Independent100 probes/600 statements/1.452123s;6 batches/3.193423s; max wait95us.418 bounded numeric events,0 dropped.

Reconciliation repeatedly traverses indexed frontiers and dominates this small fixture; publisher queue waiting is negligible here. At generation0, older negative-generation probes are impossible. Guarded generation0 dispatch now uses6 instead of12 statements per probe; positive generations retain all probes. This halved measured frontier SQL from1200 to600, with no source/support/lease fence removed. It did not materially improve total wall time. Background scheduling improves collection independence; total independent convergence is slower in this quiet small cohort. Original live39-minute job was not replayed and its exact production bottleneck is not proven.

## Visual and static documentation evidence

The existing launcher behavior filter passed16 tests. All15 selected launcher PNGs and3 Atlas readiness PNGs were inspected earlier as accepted previews. The identity audit at `tui/existing-evidence-audit.json` verifies all18 existing cell/PNG hashes and42 relevant UI/fixture/renderer sources per capture. The sole launcher-source difference is removal of an unused Span import; reconstructing it exactly matches the captured source hash. No new capture, image inspection or interaction pass is claimed. Final integration acceptance remains separate.

Earlier repository Playwright diagram previews passed2/2 and were inspected. After the cleanup-handoff diagram change, root ran the repository Playwright Atlas preview (1PASS,3.4s) and inspected `atlas-pipeline-cleanup-preview.png`, reporting no overlaps; this is a new diagram preview, separate from the carried-forward TUI captures. Current static checks validated193 local file links across8 changed Markdown files, both SVG XML documents, and matching inline/export text labels (Atlas46, classifier30). Input hashes, preview/log hashes and review attribution are recorded in `.agent-scratch/classifier-v2-evidence-static.json`. The early-delegation guidance lives in the canonical agent workflow, with a concise AGENTS pointer, clear writable ownership, one Cargo/cache owner, resource-aware concurrency and explicit follow-up wakeups. No additional approval gate was added.

## Remaining verification

- Integrate and execute the superseded-publication cleanup regression, then refresh affected admission/actor/lifecycle/derivative contracts and final static gates. The completed separate embedded coverage applies to the earlier compiled revision.
- Refresh the canonical CLI smoke on the final selected binary.
- Build/hash the two final benchmark binaries and record actual resource/cohort reruns. Do not rescore Test or retroactively label old binary identities as current.
- Finalize the acceptance and cache handoff. Preserve the unchanged cancellation failure as a failed gate result unless new valid evidence resolves it; do not call source identity a baseline pass.

No live-provider, local server/all-platform matrix or local Rust mutation execution. No hosted pass is claimed for unpublished changes. No user production model/data activation or service restart was performed.
