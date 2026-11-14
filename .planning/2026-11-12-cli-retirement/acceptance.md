# CLI retirement acceptance

## Scope and disposition

The supported user entry points are the Rune macOS app, independently operated
Docker backend services and Intel Web. This change retires Venture, the six
Fleetworks terminal clients and the legacy `argos` executable. It does not claim
previously unverified installed-app, platform or real deployment gates passed.

Shared contracts/workspace libraries and backend service/support executables
remain. Existing operator workspaces, native folders, credentials, datasets,
service volumes and debug caches are preserved. Legacy host-process management
and terminal-only import/admin commands are retired with their executables.

Core fixtures previously activated only by the legacy executable's development
dependency now belong to service-host development dependencies. They remain
absent from the normal/build-only network service graph.

## Verification

| Check | Status | Evidence / limit |
| --- | --- | --- |
| Retired source disposition | PASS | 81 tracked files removed from the three executable crates; no untracked files existed in those trees. |
| Workspace and lock | PASS | Nine retained members; native app is default. Locked host metadata resolves. 69 obsolete package entries removed, no packages added or upgraded. |
| Fixture feature boundary | PASS | Workspace metadata includes Core fixtures; normal/build-only service graph has classifier, NLP, server and Surreal features without fixtures. |
| Rust formatting | PASS | `cargo fmt --all -- --check`. |
| Strict workspace/all-target Clippy | PASS | All nine retained packages/all targets, warnings denied; 9m34s using shared implicit host debug cache. |
| Scoped native tests | PASS | 119 executed, zero failed/ignored: 32 contracts, 61 desktop, 16 workspace unit and 10 workspace integration. |
| Delivery/static/policy checks | PASS | 84 script + 14 deploy Python cases, 21 Node policy cases. Five read-only resolved Compose variants pass (backend/Web/dev/Atlas/all). Project collision and daemon mismatch reject before mutation/cleanup. |
| Documentation/diagram checks | PASS | 1,722 global relative references, zero broken. 14 selected pairs pass parity/XML/accessibility/budgets; 14 rendered, nine inspected. Five uninspected images remain unclaimed. |
| Full embedded workspace matrix | INCOMPLETE, prior capacity blocker | Previous cutover compilation was interrupted for insufficient disk. This retirement does not convert it to a pass or repeat a likely capacity failure. |
| Native installed GUI and Docker runtime | USER-OWNED | No app launch, container startup, image replacement or user-stack mutation by this task. Hosted Docker/platform evidence remains separate. |

## Reproduction

```sh
cargo fmt --all -- --check
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked -- -D warnings
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 python3 scripts/agent_cargo.py test --locked \
  -p fleetworks-contracts -p fleetworks-workspace -p fleetworks-desktop
python3 -B -m unittest discover -s scripts -p 'test_fleetworks_*.py'
python3 -B -m unittest discover -s deploy -p 'test_*.py'
node --test .opencode/tests/*.test.mjs
python3 scripts/fleetworks_doc_links.py
```

The read-only Compose check used a fresh isolated review with explicit API/Web
ports and resolved the backend, Web, development, Atlas and combined overlays.
It verified the Web credential target and repository-root development build
context. It did not start a Docker daemon operation or service.

Rust checks use `python3 scripts/agent_cargo.py ... --locked` and the shared debug
cache. Cargo metadata/tree are non-build inspection commands. The lock prune
uses existing versions, and subsequent validation is locked.

The maintained retirement disposition and replacement commands live in
[the repository retirement record](../../docs/archive/cli-retirement.md),
[README](../../README.md) and [deployment guide](../../deploy/README.md).
