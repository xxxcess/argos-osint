# Argos — OpenCode quick guide

Shared by every role/model; complements the unchanged repo AGENTS.md. User instructions take precedence. Preserve unrelated changes.

- **Map:** Rust OSINT CLI/TUI. Core: `crates/argos-osint-core/` (storage: `src/storage/`, contracts: `tests/storage_*.rs`, harness: `tests/support/`). CLI/TUI: `crates/argos-osint-bin/`. Start documentation at `docs/README.md`.
- **Discovery:** when graph.json exists, start with `graphify query`; verify against current source. Use relevant configured skills/MCP tools and repo conventions.
- **Team:** architect owns audit/spec/review/shared plans; coder delegates disjoint EDIT_UNITs and integrates one phase; tester verifies the combined tree and relays evidence through coder. Follow `docs/opencode-v2.md` for handoffs/worktrees. Leaf modes never delegate; resume only your own child sessionID.
- **Planning:** load planning-with-files; pin `.planning/<YYYY-MM-DD-slug>/` with task_plan.md, findings.md and progress.md. Architect updates phases/Next Step. Follow `docs/planning-artifacts.md`; scratch stays in `.agent-scratch/`, unstaged.
- **Build:** Rust 1.95.0, ARGOS_EMBED unset. Use `python3 scripts/agent_cargo.py … --locked`; direct cargo fmt is allowed. Feature worktrees edit/format only. Tester runs `scripts/local_matrix.sh --storage embedded` in the combined main tree; server is CI-only. Serialize Cargo; preserve locks/processes and user's target/debug.
- **Retries:** full gate normally 1–2 times, max 3. Use wrapper check --workspace --all-targets and targeted failing tests before the final gate. Tests stay offline; MiniLM remains ignored. Storage tests run via test_surreal.py; skipped checks are not passes.
- **Storage:** SurrealDB only, defaults embedded,nlp. RocksDB: ARGOS_HOME/storage/surreal (ARGOS_HOME defaults ~/.argos). Mode: env → storage/config.json → embedded. Server is opt-in; no rusqlite/lancedb.
- **TUI:** load argos-tui-verify; follow tui-design-spec, tui-components and tui-verification (Profile: profile-analytics-dashboard). Capture affected views/sizes with scripts/tui_review.py; open PNGs and report image/interaction/metric evidence separately.
- **Delivery:** update docs/diagrams with architecture/contracts. Route .github changes through Code Architect. Coordinator makes authorized Conventional Commits with exact staging and release_check.mjs message/versions checks. Release Please owns versions; no hand-bumps/Release-As.
