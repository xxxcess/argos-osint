# Native workstation redesign — acceptance

[Handoff](handoff.md) · [Documentation audit](phase8-design.md) · [Backend audit](phase9-design.md) · [Design contract](../../docs/wiki/code/native-design-system.md)

Baseline: `d219d5964a29d50fa99de4cdcf2b228391576b4a`, `feat/atlas-zero-shot`. Results below cover the implementation in the accompanying feature commit. Commit `241af22` separately records the build-progress helper. No deployment or hosted-platform approval is implied.

## Verification status

| Area | Current evidence | Status |
| --- | --- | --- |
| Native, contracts, protected settings | 120 library tests:33 contracts,68 desktop,19 manager; captured revision, model-switch and durable deletion recovery regressions included | PASS on final source |
| Protected legacy-folder compatibility | 10 integration tests; folders/data preserved | PASS, unchanged behavior |
| Web Connect, logout and matte views | Final isolated dev Playwright8/8; real logout POST200, old-cookie replay401; four visual cases/eight inspected images | PASS |
| Web static/unit checks | Typecheck/lint;23 Connect tests,46 theme/contrast/shared-UI tests;11 dev-HMR boundary tests | PASS |
| Native build progress | 19 Python tests, including output visible before child exit, bounded diagnostics, timeout and safe launch handling | PASS |
| Review app | Actual agent debug build/package/validation passed at the earlier implementation checkpoint; live Cargo progress observed | PASS for that package; final source unit checks recorded separately; GUI not opened |
| Native rendering |32 base captures plus8 affected recaptures;16 final selected wide1x/compact2x PNG/tree pairs inspected across all eight surfaces | PASS for inspected base states; installed GUI/input/platform evidence is separate |
| Final Rust static checks | `cargo fmt --all -- --check`; workspace/all-target Clippy with warnings denied, including all final API/native/manager regression inputs | PASS |
| Full embedded gate | Workspace executable compilation stopped at <3 GiB free reserve (exit130), before runtime tests; final static checks passed separately | BLOCKED by disk capacity; final client/contracts/manager tests passed |
| Docs/file audit |29 maintained docs;831 local links/anchors;11 exact XML-valid HTML/SVG pairs; changed/new figures inspected | PASS |
| Whitespace | Maintained-file staged check passes; original supplied Markdown retains its trailing spaces/EOF exactly | Source-bundle findings intentionally preserved |
| Original bundle | All11 extracted entries match ZIP bytes; source/archive preserved | PASS |

## Resolved findings

| Finding | Repair / retained guarantee |
| --- | --- |
| Ten failing native tests and retired UI warnings | Current production/handler assertions replace retired controls; privacy, masking, consent, replay, cursor and total assertions retained; no ignored tests or lint suppression |
| Direct Cycles entry lacked configuration | Entry loads current Models revision before enabling mutation; navigation still never submits |
| Jobs total assertion matched window geometry | Inspect accessible names/values/descriptions, preserving paging and dashboard-versus-matched-total assertions; final68 native tests pass |
| Compact log columns overflowed | Measured responsive columns;320-point virtualization and selected full safe summary retained |
| Browser disconnect clicked before hydration | Wait for API-ready state; assert real logout and rejection of the prior session |
| Dev HMR path rejected | Exact dev-only current/legacy paths with unchanged same-origin and production denial |
| Migration could revive inactive authorization | Preserve inactive v2 registration/no binding, stable workstation identity and accepted causal work |
| In-flight cycle read could outlive revocation | Revalidate authority after loading rows; deterministic real-repository regression added |
| New cycle contract missing required runner | Register `storage_cycle_history` in required-mode contracts; ordinary optional harness follows existing policy |
| Quiet build command | Immediate flushed stages + live Cargo stderr; exact stdout artifact parsing and failures preserved |
| Cycle Delete omitted API configuration revision | Precommit review demonstrated API428; capture/recheck reviewed revision and validate the exact journal payload; native/manager regressions pass, API regression compiled in final checks but storage execution is blocked |
| Obsolete variant draft after route change | Precommit review demonstrated API400; prune pending updates to final routes, preserve fallback/server settings; native regression passes |
| Stale workspace-era fixture ports | Obsolete arguments/URLs/error formatting corrected without weakening isolation assertions |

The required embedded runtime gate remains blocked by capacity. No storage/API runtime pass is claimed for the audited repairs. Earlier failed runs remain failures; their raw history is retained in local progress/evidence.

## Reproduce

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  bash scripts/local_matrix.sh --storage embedded
ARGOS_WEB_TEST_DEV=1 ARGOS_WEB_PROFILE=dev ARGOS_WEB_TEST_PORT=3217 \
  pnpm test:e2e tests/web/connect-gate.spec.ts tests/web/visual-regression.spec.ts --timeout=60000
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p test_fleetworks_candidate.py
python3 scripts/fleetworks_candidate.py build-native --agent --target host --profile debug \
  --output .agent-scratch/native-workstation-resolution/native-app
```

Native captures use `fleetworks_verify.py native-ui --agent` with the executable-validated `redesign-{status,seed,cycles,brain,tools,models,jobs,setup}` scenes,1180×780/640×480 and1×/2×. Reuse unaffected inspected evidence; rerun affected profiles after a demonstrated change.

## Limits and provenance

- No actual GUI launch, accessibility audit on other physical hardware, second Mac/Linux architecture, real-provider inference, Docker deployment or hosted CI was performed. Mutation testing stays hosted.
- Supplied mockups/audit claims are reference inputs. Archive SHA-256: `40f0251c2c4f46d105f256fa99a9c3d7c851546c49ba961a540eee8cacc4897a`. Its proposed November13 audit date was not presented as a local audit.
- Removed only proven obsolete retired-agent executable/library images (87 files) and incremental units (126 directories), after source/ownership/no-open checks. Current native/service/dependency artifacts, user `target/debug`, global caches, homes, running apps and frontend `.next` were preserved. No other cleanup is authorized by this record.
