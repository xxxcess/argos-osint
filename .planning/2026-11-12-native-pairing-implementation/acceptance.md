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
| Backend storage/security assertions | New repository/API pairing, revoke/config-lock, reset/issuer-policy and reopen cases | PENDING: required embedded workspace build running; assertions not yet executed |
| Native captures | 24 actual Rune captures; 12 selected PNG/tree pairs inspected (six scenes at 1180×780/1× and 640×480/2×) | PASS for selected synthetic views; real Keychain/installed GUI separate |
| Docs/diagram checks | Original 16 Markdown files/420 links/5 figures; workflow updates cover 8 reviewed guides/entrypoints with 172 local path/anchor checks and 2 exact XML-valid rendered/inspected diagram pairs | PASS; workflow links, bounds and whitespace checked |
| Formatting | Full `cargo fmt --all -- --check` after authorized formatting repair; assertions unchanged, original formatting retained in scratch | PASS; tracked Rust source matches the feature commit |
| Workspace all-target Clippy | Earlier attempt interrupted at <1 GiB reserve, exit 130; resource blocker now repaired | PENDING rerun after focused required-mode tests |

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

## Reproduce remaining checks after capacity is available

Use a clean checkout of the feature commit so unrelated working-tree edits do not affect formatting. Reuse the shared implicit host debug graph; preserve user/global caches.

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  bash scripts/local_matrix.sh --storage embedded
```

Focused required-storage regressions live in API `pairing::tests` and `admission::tests::captured_configuration_stays_locked_through_acceptance_then_save_fences_fresh_nonce_and_preserves_replay`. The full gate's explicit required embedded phase must execute the retained storage harness, rather than treating an ordinary absent-harness skip as success.

The backend build issue was independently reproduced from BuildKit history and the lockfile fix validated against both isolated Linux dependency graphs. The old control-service SDK `SendError` represented a closed response channel after a query result; its bounded adjacent log contained no other failure and the existing service remained healthy. No vendor log suppression or existing-stack restart was applied.

## Requested verification rerun

At source revision `5fec2a9`, reran the three client/native/workstation library suites: **146 passed, 0 failed** (41 contracts, 79 native, 26 workstation). The operator command's six tests and fresh-review provisioning's nine tests also passed again. No actual user Keychain access or app launch occurred.

The full `local_matrix.sh --storage embedded` invocation exited **1** at its first formatting step, solely on the preserved unstaged formatting changes in `crates/fleetworks-workstation/src/native/tests.rs`. Consequently, this invocation did not reach Clippy, workspace tests or the required storage phase. The focused backend pairing command was not started: free disk was about 2.8 GiB before the rerun and 2.2 GiB afterward, below the established build reserve. Those backend checks remain **BLOCKED**, rather than passed or skipped as successful.

Local logs: `.agent-scratch/native-pairing-rerun-units.log` and `.agent-scratch/native-pairing-rerun-matrix.log`. Resolve the unrelated formatting diff and provide sufficient capacity before rerunning the remaining commands above. The native build/open step is reserved for the user.

## Blocker repairs and workflow guidance

| Pitfall | Resolution / current evidence |
| --- | --- |
| Dirty formatting blocks the full gate | Backed up the existing edit, ran the package formatter under the user's repair instruction, preserved assertions; full formatter now passes. |
| Disk reserve exhausted | Validated exact obsolete artifact provenance, ownership, inode/hardlinks and open-file status. Removed retired compiled artifacts (~11.75 GiB) and a bounded 108 obsolete core storage-test executables (~30.02 GiB). All current graphs and their dependencies, metadata and incrementals, plus user `target/debug`, data and QA evidence retained. Free space rose to about 39 GiB before continued compilation. |
| API-only selection unexpectedly recompiles dependencies | Matching core features alone lost `log`/`tracing` and host/proc-macro unification. Interrupted that build before assertions, retained its exit-130 log, and switched once to the workspace graph with a test-name filter. |
| Cold build is slow | Observed roughly 20 minutes in RocksDB C++ compilation on this Intel host; CPU speed limit was 43%. Full Rust debug info and the embedded/NLP/classifier/fixture graph add work. These are observations, not a benchmark. Current engine compilation completed. |
| Workflow guidance is hard to discover | Updated contributor loop/cache decisions, build artifact guide, agent workflow, relevant READMEs and two rendered diagram pairs. Permanent guidance describes reusable choices; this tracker retains incident history. |

Required-mode command currently running (workspace targets compile; only matching tests execute):

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_BIN \
  ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --workspace --locked \
  pairing::tests:: -- --nocapture --test-threads=1
```

The backend assertions and full gate remain pending until their processes actually complete. Native app build/open and real Keychain checks remain reserved for the user.
