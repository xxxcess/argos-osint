<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->

Follow the [agent workflow](../../docs/wiki/code/agent-workflow.md) ([scope and finish line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line)), [shared UI rules](../../docs/wiki/code/react-ui.md) ([Fleetworks look and feel](../../docs/wiki/code/react-ui.md#fleetworks-look-and-feel)), [Glass Canvas theme](../../docs/design/matte-canvas/theme.md), and [interaction-state contract](../../docs/design/matte-canvas/interaction-state.md). Preserve the generated Next block above.

- **Scope & skills:** Load `argos-react-ui` for UI work and `argos-react-patterns` for components/state/perf. Preserve web routes, typography, composition, and topology/list behavior (never graft native setup/service workflows onto Intel; mockups are not runtime proof). Connect v2 uses an opaque workstation ID (optional IP metadata never defines identity); reject legacy workspace contexts/cookies/query narrowing and keep authoritative session/revision/epoch fences.
- **Next & Compiler:** Keep React Native Web aliasing/transpilation and React Compiler enabled. `pnpm lint` enforces zero-warning Hooks/Compiler diagnostics (fix rather than suppress or opt out). Always launch Next with `apps/web-desktop` as CWD for Babel preset resolution; compiler/launcher changes require both export and dev browser checks.
- **Verification & visuals:** In Nightly GSD, run Web checks, builds, and captures in [GitHub Actions](../../.agents/skills/gsd/SKILL.md#github-ci-first) per [CI contracts](../../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml`; cover pending, refresh, empty/zero, error, and cursor-page states. For UI deltas, dispatch [UI visual review](../../.github/workflows/ui-visual-review.yml) ([recipe](../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review)) at exact PR head with `scope=web` and affected `web_surface` (or use existing CI artifacts until the workflow lands on the default branch); inspect Playwright images, retain the artifact, attach selected images inline, and record durable refs.
- **Hygiene & restrictions:** Follow [build cache hygiene](../../docs/wiki/code/contributing.md#build-cache-hygiene) while preserving active Next output/cache, running dev server, and retained Rust cache. Do not run Rust mutation tests (`cargo mutants`) locally; use hosted `mutation.yml` and report local mutation as skipped.
