# Acceptance — Intel Nexus Desk

Updated 2026-11-01 for the final implementation. Evidence below is local; no new hosted result is claimed.

## Completed checks

- `pnpm exec vitest run --no-file-parallelism`: 27 files, 180 tests passed.
- TypeScript, React Compiler/Hooks lint, Next production build and `pnpm check:web-build` passed.
- Offline production browser checks: metrics push, quiet Bulletin/completed Brief, pending progress/reconnect and camera/drag regressions passed (4 cases). Dev transport fixtures passed (3 cases).
- Topology browser checks at 1280, 832 and 320 pixels plus manual camera/drag passed (4 cases); captured screenshots were inspected.
- `pnpm test:live`: all 5 offline Node relay/runtime tests passed, including isolated dev HMR and a stateless production restart.
- `node --test .opencode/tests/*.test.mjs`: all 22 agent and release-policy tests passed.
- Scoped Rust API Clippy passed. Six library tests passed for metrics freshness/zero values and ongoing/terminal work. The isolated live socket integration previously passed; the final combined integration rerun was interrupted, so it is not claimed as final verification.

## Remaining verification limits

- The full embedded gate passed formatting and workspace Clippy, then was interrupted during workspace test compilation under heavy linker/resource load. The full Rust gate is incomplete.
- The latest stack-launcher source has not been verified through a newly built CLI process smoke.
- Exact pixel equivalence to the initial six reference images, a 200-node frame-rate measurement and WebGL context counts remain unverified.
- Local Rust mutation execution was deliberately skipped under repository policy. Hosted mutation evidence is separate.

Generated screenshots, logs and operational session plans stay local and are not committed.
