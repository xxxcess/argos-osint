<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->

Follow the shared [agent workflow](../../docs/wiki/code/agent-workflow.md) for execution and evidence. Reuse authorized scoped checks, preserve the active host's roles, and suggest evidence-backed reversible process improvements within assigned ownership.

Keep work bounded by the shared [scope and finish line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line).

For React UI changes, load `argos-react-ui` and, for components/state/performance, `argos-react-patterns`. Follow [shared UI rules](../../docs/wiki/code/react-ui.md) and the [Argos look and feel](../../docs/wiki/code/react-ui.md#argos-look--feel-reference). Design components creatively inside the Argos theme. Preserve the generated Next guidance above.

Keep the React Native Web alias/shared transpilation and React Compiler enabled in Next. `pnpm lint` enforces Hooks/compiler diagnostics with zero warnings; fix violations without disabling rules or adding blanket compiler opt-outs.

Launch Next with this app directory as its working directory; an app path argument alone does not fix Babel preset resolution from the repo root. Compiler/launcher changes require export and development browser checks.

Use [CI workflow contracts](../../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml` for frontend verification; async UI changes must cover pending, refresh, empty/zero, failure and cursor-page behavior.

Follow [build cache hygiene](../../docs/wiki/code/contributing.md#build-cache-hygiene): check free space before/after builds, remove only verified obsolete inactive artifacts, and preserve the active Next output/cache, running dev server and retained Rust build.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
