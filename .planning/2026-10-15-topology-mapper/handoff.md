# Topology mapper handoff

The approved Graph first demo is implemented in Argos TUI: real seeded two-hop
neighborhoods, exact Brain claim-path/actor-group and Intel claim/actor entries,
inspector/relationships, recenter/history, compact pages and requested dense
lists. Ordinary list spacing and theme conventions remain intact.

Read the [spec](../../docs/topology-mapper.md) and
[verification](../../docs/tui-verification-topology-2026-10-15.md). Core ownership
is `brain_topology{,_query}.rs` / `brain_scope.rs`; TUI ownership is
`topology{,_layout}.rs`, `app.rs`, `ui.rs`, `components.rs`, `graph.rs` and
`brain_detail.rs`. The existing topology HTML/SVG is updated. Public DTO/seed
extensions require Rust caller updates documented in the spec. Default depth is
two; no schema migration or manual version bump is included.

Checks passed: fmt, strict Clippy, 1,276 offline tests with four threads, 126 reviewed
terminal captures, screenshot tooling, receipt hashes and diagram parity. Two
existing lease timing tests passed isolated and full reruns. Live terminal,
providers, LanceDB defaults and MiniLM remain unexercised.

Next action: restart Argos and smoke-test the Brain/Intel entries with persisted
data in the user's terminal.

Prepared with [Argos ecc-checkpoint](../../.opencode/skills/ecc-checkpoint/SKILL.md).
