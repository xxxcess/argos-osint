# Native pairing and Keychain — verification

[Connections](../../docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target) · [Native design](../../docs/wiki/code/native-design-system.md) · [Handoff](handoff.md)

Implementation baseline: `b8da29f`. Dependency/lockfile checkpoint: `6eef4cc`. This record separates source implementation from actual runtime and installed-app evidence.

| Area | Evidence | Status |
| --- | --- | --- |
| Native production graph | Scoped desktop/contracts/workstation library check | PASS before two recovery repairs; subsequent unit compile covers repaired source |
| Rust client/native/settings tests | 41 contracts (including 7 transport + 1 pairing contract), 79 native (including 4 Runtime + 7 effect/fence), 26 workstation (including 6 fake-store + 1 OSStatus) | PASS 146/146; fake Keychain and temporary homes only |
| Operator command | Six offline approval/denial/revocation/transport/file-safety tests | PASS |
| Fresh review provisioning | Nine temporary-directory tests; independent Operator token/API-only mount | PASS |
| Locked host resolution | Full host-filtered Cargo metadata with `--locked --offline` | PASS; no dependency upgrades |
| Linux image dependency resolution | Isolated manifest/stub-only BuildKit metadata for runtime/search graphs | PASS for both Linux graphs with --locked; no release image or stack changes |
| Backend production compiler | Exact runtime server/nlp/classifier features through shared host debug cache, 4m40s | PASS |
| Backend storage/security assertions | Three API pairing tests in required embedded mode: lifecycle/admission/revoke/config-lock/policy, persisted reopen and rejected authority fields | PASS 3/3 in 22.44s; overall filtered driver intentionally interrupted afterward (exit 130) |
| Native captures | 24 actual Rune captures; 12 selected PNG/tree pairs inspected (six scenes at 1180×780/1× and 640×480/2×) | PASS for selected synthetic views; real Keychain/installed GUI separate |
| Docs/diagram checks | Original 16 Markdown files/420 links/5 figures; workflow updates cover 8 reviewed guides/entrypoints with 175 local path/anchor checks and 2 exact XML-valid rendered/inspected diagram pairs | PASS; workflow links, bounds and whitespace checked; later bounded audit: 11 relevant docs/plans, 203 local links/anchors, two unchanged diagram pairs valid |
| Formatting | Full `cargo fmt --all -- --check` after authorized formatting repair; assertions unchanged, original formatting retained in scratch | PASS on the current test-fixture repair checkpoint |
| Workspace all-target Clippy | Whole workspace/all targets with `-D warnings`; fixture compiler/lint repairs below | PASS after all six case repairs; final all-target rerun 3m33s |
| Full local embedded gate | Prior collection failed six cases across four targets; all six repaired cases pass in focused required-mode execution | RUNNING at `712ed06`; formatting, Clippy and full workspace pass (3511s including 11m11s compilation/linking); host help PASS (928s); broad required storage restarting after runner graph repair |

## Implemented boundaries

- Public pairing opens only literal start/poll POST routes after Host/Origin checks; no generic unauthenticated API access. Operator approval/deny/revoke has separately authenticated purpose and scope.
- Durable schema 10 stores proof/token hashes and issuance metadata. HMAC derives one recoverable issued token from the client-held polling secret; repeated delivery never creates replacement authority. Current issuer policy, reset epoch, installation and revocation fence use/read/admission.
- Configuration write and revoke share ConfigLock→registry-guard order. A waiting stale write revalidates after acquiring ConfigLock. Global proofNone requests also revalidate dynamic authority.
- Native metadata remains non-secret; bearer/polling secrets use exact installation/origin Keychain items. Missing/locked/denied storage never adopts a token file. Pending starts and issuance promotion survive restart; Retry preserves exact inputs. Explicit restart requests new approval.
- Ordinary setup has one actionable state and no provisioning flags or pre-auth registration/readiness noise. Background/focus/destination changes cancel polling and fence stale UI replies. Saved selected origins reconnect; explicit new origins do not silently pair.
- Fresh review initialization creates an independent 0600 Operator credential mounted only into the API gateway. Existing review directories, token files, identities and journals are not overwritten/backfilled.

## Resolved findings

| Finding | Repair |
| --- | --- |
| Docker rejected `--locked` | Synchronize only the existing HMAC/Security Framework direct dependency entries (`6eef4cc`); retain lock enforcement |
| Failed final Keychain write blocked exact Retry | Retained issuance bypasses challenge-metadata replacement and resumes exact poll/auth/promotion; native regression passes |
| Explicit startup URL could inherit another selected origin's restore trigger | Restore only known pairing/pending metadata for the requested origin |
| Paired authority could outlive revoke while awaiting ConfigLock | Revalidate global authority and serialize configuration write/revoke; deterministic API regression added |
| Fresh review had no Operator approver | Add a distinct private Operator credential and gateway-only mount; provisioning regression passes |

## Limits

Actual user Keychain access, installed GUI/OS dialogs, signing, both Mac/Linux architectures, real provider calls, deployment/bootstrap against retained user data and hosted CI are not executed by these synthetic/unit checks. No user home, credentials, running stack, images or unrelated `dist/` were modified.

Earlier attempts were capacity-blocked, including workspace Clippy interrupted at <1 GiB free reserve (exit 130). The repairs below now provide room for the remaining checks. Source/tests or compiler success alone do not prove stored authorization behavior.

## Reproduce remaining checks

Use a clean checkout of the feature commit so unrelated working-tree edits do not affect formatting. Reuse the shared implicit host debug graph; preserve user/global caches.

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  bash scripts/local_matrix.sh --storage embedded
```

Focused required-storage regressions live in API `pairing::tests` and `admission::tests::captured_configuration_stays_locked_through_acceptance_then_save_fences_fresh_nonce_and_preserves_replay`. The full gate's explicit required embedded phase must execute the retained storage harness, rather than treating an ordinary absent-harness skip as success.

The backend build issue was independently reproduced from BuildKit history and the lockfile fix validated against both isolated Linux dependency graphs. The old control-service SDK `SendError` represented a closed response channel after a query result; its bounded adjacent log contained no other failure and the existing service remained healthy. No vendor log suppression or existing-stack restart was applied.

## Historical verification rerun (5fec2a9)

At source revision `5fec2a9`, reran the three client/native/workstation library suites: **146 passed, 0 failed** (41 contracts, 79 native, 26 workstation). The operator command's six tests and fresh-review provisioning's nine tests also passed again. No actual user Keychain access or app launch occurred.

The full `local_matrix.sh --storage embedded` invocation exited **1** at its first formatting step, solely on the preserved unstaged formatting changes in `crates/fleetworks-workstation/src/native/tests.rs`. Consequently, this invocation did not reach Clippy, workspace tests or the required storage phase. The focused backend pairing command was not started: free disk was about 2.8 GiB before the rerun and 2.2 GiB afterward, below the established build reserve. Those backend checks were **BLOCKED** at that checkpoint; the later scoped API pass supersedes this blocker.

Local logs: `.agent-scratch/native-pairing-rerun-units.log` and `.agent-scratch/native-pairing-rerun-matrix.log`. The later repair checkpoint resolved this formatting/capacity block. The native build/open step is reserved for the user.

## Blocker repairs and workflow guidance

| Pitfall | Resolution / current evidence |
| --- | --- |
| Dirty formatting blocks the full gate | Backed up the existing edit, ran the package formatter under the user's repair instruction, preserved assertions; full formatter now passes. |
| Disk reserve exhausted | Validated exact obsolete artifact provenance, ownership, inode/hardlinks and open-file status. Removed retired compiled artifacts (~11.75 GiB) and bounded subsets totaling 178 obsolete core storage-test executables (~48.19 GiB). All current graphs and their dependencies, metadata and incrementals, plus user `target/debug`, data and QA evidence retained. Free space rose to about 39 GiB before continued compilation. A later exact 31-file subset recovered ~8.03 GiB when the repaired Rust graph reduced the reserve to ~5.6 GiB; fresh provenance/identity and global open-file checks passed, leaving ~13.4 GiB. |
| API-only selection unexpectedly recompiles dependencies | Matching core features alone lost `log`/`tracing` and host/proc-macro unification. Interrupted that build before assertions, retained its exit-130 log, and switched once to the workspace graph with a test-name filter. |
| Cold build is slow | Observed roughly 20 minutes in RocksDB C++ compilation on this Intel host; CPU speed limit was 43%. Full Rust debug info and the embedded/NLP/classifier/fixture graph add work. These are observations, not a benchmark. Current engine compilation completed. |
| Workflow guidance is hard to discover | Updated contributor loop/cache decisions, build artifact guide, agent workflow, relevant READMEs and two rendered diagram pairs. Permanent guidance describes reusable choices; this tracker retains incident history. |

Required-mode command (all three API cases passed; the driver was then intentionally interrupted while launching unrelated zero-match harnesses):

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_BIN \
  ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --workspace --locked \
  pairing::tests:: -- --nocapture --test-threads=1
```

The filtered workspace command is INTERRUPTED, not an overall pass. Its three completed API tests remain valid scoped evidence. The collected full workspace run exited 101; repairs and required remaining phases are tracked below. Native app build/open and real Keychain checks remain reserved for the user.

### Test-target preflight repairs

The first required-mode build exited 101 before assertions: an older static-purpose fixture omitted the new `AccessPolicy::pairing_repo` field, and the pinned SDK cannot deserialize a complete query result directly into scalar `serde_json::Value`. The fixes supply `None` only in that datastore-free fixture and use supported `Vec<Value>` rows. The privacy audit also verifies an issued-credential row is present before testing that secrets are absent. Actual API pairing tests now pass.

All-target Clippy then found one unnecessary clone in a native pairing test assertion. A borrowed-slice comparison preserves the assertion; the repaired full Clippy pass succeeds. The contributor workflow now calls out test-target compilation before expensive executable generation, because a production-only check excludes `#[cfg(test)]` fixtures.

### Collected workspace failures and bounded repairs

Log: `.agent-scratch/native-pairing-check-repairs/full-matrix-collect.log`. Formatting/Clippy passed; workspace exited **101** with four failed targets (six cases). Host help and required storage were not reached. API unit tests **57/57** and API contracts **9/9** passed after typed-context/real-registration fixture repairs. The seven-case classifier admission group passed in **840.48 seconds**; that is group duration, including serialized durable work, not a single-test measurement.

| Failed boundary | Repair under verification |
| --- | --- |
| WebSocket live fixture | Real scoped Native authority, public registration and brief admission; keep Origin/auth rejection and no-polling assertions. |
| Classifier activation runtime budget | Use the existing cross-module runtime gate; preserve production budget and activation/uncertainty assertions. |
| Auth-only commit generation | Compare durable revisions within the same root; assert the independent process counter also advances. |
| Summary worker stack overflow | Box the actual drain operation before callers embed its future. Preserve normal thread stacks, five real subprocess rounds and 500 ms shutdown bound. |
| Paging fixtures in optional workspace phase | Follow shared optional harness contract. Required embedded mode still executes both full paging/privacy cases; REQUIRED=1 cannot skip. |

The old linked summary-worker chain had about 1.43 MB across four nested first-party frames. This diagnoses allocation pressure; only the rebuilt real public path can establish the repair. No stack-size or timing-limit increase is used. `--no-fail-fast` collects independent workspace failures without changing the final failing exit status. Ordinary absent-harness returns remain **SKIPPED**, never required-storage evidence.

Post-repair preflight: full workspace/all-target Clippy with `-D warnings` passed (3m33s). Its initial attempt found one cloned-reference lint in the new live fixture; fixed with `std::slice::from_ref`, without suppression. Required-mode focused library/live/stress/paging verification is now running on the workspace graph.


### Verified repairs (712ed06)

Focused command retained the workspace graph and selected libraries plus live/stress/paging targets, with `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1`, `ARGOS_SURREAL_BIN`, `ARGOS_EMBED` and `RUST_MIN_STACK` unset. Exit **0**; compilation **5m03s**. API **57/57**, live WebSocket **1/1**, core **961 passed / 6 ignored**, worker stress **8/8**, paging **2/2**; remaining selected workspace libraries also passed: **1,184 passed / 0 failed / 6 ignored** across 11 harnesses. Log: `.agent-scratch/native-pairing-check-repairs/repaired-targets.log`. No actual Keychain or GUI was used.

The normal-stack public shutdown regression passed all five subprocess rounds with the original 500 ms limit, and drop/queued-summary checks passed. Bounded fresh-binary inspection reduced the runtime blocking/callback/worker callback frames from 199,328/403,664/400,688 bytes to 2,880/7,744/2,704 bytes. Combined named reservations fell from ~1.43 MB to ~636 KB. This supports the observed runtime repair without changing stack sizes.

The complete canonical embedded gate is rerunning at `712ed06`; no overall gate pass is claimed yet. All source is frozen. API fixture checkpoint: `0e5a3a8`; workflow/docs checkpoint: `ed0b0fd`.

Full workspace stage at `712ed06` exited0: **3511 seconds** including **11m11s** compilation/linking. All148 reported harness summaries completed without failures; ordinary mode-selected storage skips are not required-storage evidence. Stress8/8 passed again14.84s, core961/0fail/6ignored, API57/57 and APIcontracts9/9. Long groups: classifier admission7/7 in627.69s and indexed links1/1 in128.65s. Host CLI help now running; dedicated required embedded contracts remain pending.


### Required-mode runner graph repair

Service-host CLI help passed at `712ed06` in **928 seconds**, building its distinct normal runtime graph. The subsequent required-storage command selected only core integration targets despite its comment claiming workspace-cache reuse. It began another dependency graph (including ICU/URL/SurrealDB) with ~7 GiB free. Root interrupted only that task-owned compile group (exit130) before storage assertions; the overall script is **INTERRUPTED**, not PASS. Completed workspace and CLI evidence remain valid.

The embedded runner now also selects `--lib --bins` to retain the workspace package/dev/transitive feature graph (including service-host core fixtures). All selected storage integration targets and required environment remain unchanged; unit harnesses deliberately run again. The server storage-only lane is unchanged. A locked no-run JSON-artifact probe passed in **3.61 seconds**: all13 selected executable artifacts were fresh and had the exact original workspace hashes, including core/API/host/paging/classifier admission. No Rust source changed; completed workspace evidence is reused. The dedicated required embedded phase will execute after this bounded runner repair.

Actual repaired-script no-run preflight passed in **1.61 seconds** with all **98 executable artifacts fresh and matching the completed workspace paths**. Independent static audit preserved all87 storage selectors, flags/environment/extras and server behavior;42 affected local links/anchors and whitespace passed. Required embedded execution now starts with source frozen.
