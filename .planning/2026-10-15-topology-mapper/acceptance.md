# Topology mapper acceptance

Baseline: `78dea7c`. The user approved the Graph first demo and authorized
production implementation. The working-tree change implements its canvas,
inspector, relationship list, compact pages, real scoped depth-two data and exact
Brain/Intel entry/history behavior. Requested list density exceptions preserve
spaced defaults. No schema or version bump is included; typed seeds/snapshot
fields are extended and default topology depth changes to two.

The maintained [specification](../../docs/topology-mapper.md) and
[verification record](../../docs/tui-verification-topology-2026-10-15.md) contain
contracts, commands, concrete fixes and limits. Eight selected reviewed PNGs and
13 final source hashes are retained in the linked screenshot receipts.

Actual checks: formatting and strict workspace/all-target Clippy passed. The
offline workspace run with four test threads passed 1,276 tests: binary 263,
core 1010, integration 3; 18 expected ignored, doc tests 0. Two preexisting lease
timing failures in an earlier parallel run passed isolated reruns and the final
full suite, without changes to those tests. All 126 final selected TestBackend
PNGs received image review. Capture assertions and screenshot-tooling regression
passed. Final source/artifact hashes, diagram XML/HTML-SVG parity, Graphify update
and whitespace checks passed; Graphify retains its existing SQL-parser warnings.

Native terminal/provider behavior, default-feature LanceDB and MiniLM are outside
the exercised checks. Crowded maps pan; minimum 60×20 terminal abbreviates labels
and ancillary Controls prose. Continuation remains explicitly unsupported.
