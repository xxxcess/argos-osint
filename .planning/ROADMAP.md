# ROADMAP.md — argos-osint

## Milestone 1 — Core Onboarding (Current)

### Objectives

- [x] Codebase map: evidence-backed map of crate structure, key modules, data flow
- [x] Docs ingest: architecture.md and providers.md ingested and verified
- [x] Project initialization: PROJECT.md, REQUIREMENTS.md established
- [x] Onboarding summary: SUMMARY.md created

### Deliverables

- `.planning/codebase/map.md` — Codebase structure map
- `.planning/ingest/ingested-docs.md` — Ingested documentation summary
- `.planning/PROJECT.md` — Project purpose and components
- `.planning/REQUIREMENTS.md` — Functional and non-functional requirements
- `.planning/onboarding/SUMMARY.md` — Onboarding index (pending)

### Dependencies

- Rust 1.94.0 toolchain
- Vendored protoc via `protoc-bin-vendored`
- C toolchain for LanceDB build dependencies

### Success Criteria for Milestone 1

- All planning artifacts present and verified
- Codebase map accurately reflects crate structure
- Documentation ingest verified against codebase
- Project setup files complete and consistent

## Future Milestones (Planned)

### Milestone 2 — CLI Verification

- Verify all CLI commands function per specification
- Run `cargo clippy --workspace --all-targets -- -D warnings`
- Ensure `cargo build --locked` succeeds

### Milestone 3 — Test Coverage

- Run workspace tests (`cargo test --workspace`)
- Run MiniLM embedding tests with `ARGOS_EMBED=1`
- Ensure coverage tests pass (every tool input maps to binding kind)

### Milestone 4 — Integration & Ship

- Cross-phase integration verification
- Create PR branch with `.pr-branch` filtering
- Prepare for merge after verification passes