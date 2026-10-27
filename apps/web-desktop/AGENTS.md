<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->

For React UI changes, load `argos-react-ui` and, for components/state/performance, `argos-react-patterns`. Follow [shared UI rules](../../docs/wiki/code/react-ui.md) and preserve the generated Next guidance above.

Keep the React Native Web alias/shared transpilation and React Compiler enabled in Next. `pnpm lint` enforces Hooks/compiler diagnostics with zero warnings; fix violations without disabling rules or adding blanket compiler opt-outs.

Launch Next with this app directory as its working directory; an app path argument alone does not fix Babel preset resolution from the repo root. Compiler/launcher changes require export and development browser checks.

Use [CI workflow contracts](../../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml` for frontend verification; async UI changes must cover pending, refresh, empty/zero, failure and cursor-page behavior.
