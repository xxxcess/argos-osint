import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Check the produced build, not just the source flag. The framework runtime
// contains memoization symbols even when application compilation is disabled.
const app = process.argv[2]
  ? resolve(process.argv[2])
  : fileURLToPath(new URL("../apps/web-desktop/", import.meta.url));
// Match Next's verification output without reading the launcher's .next.
const dist = resolve(
  app,
  process.env.ARGOS_WEB_TEST_DEV === "1"
    ? "../../.agent-scratch/next-web-tests"
    : ".next",
);
const { config } = JSON.parse(
  await readFile(resolve(dist, "required-server-files.json"), "utf8"),
);
assert.notEqual(
  config.output,
  "export",
  "The Next API proxy needs a running server",
);
assert.equal(
  config.reactCompiler?.panicThreshold,
  "all_errors",
  "React Compiler must be enabled with strict compilation failures",
);
assert.equal(
  config.reactCompiler?.environment
    ?.enablePreserveExistingMemoizationGuarantees,
  true,
  "Keep existing memoization guarantees enabled",
);
for (const name of [
  "@argos/ui",
  "@argos/intel",
  "@argos/api-client",
  "react-native-web",
])
  assert.ok(
    config.transpilePackages?.includes(name),
    `Missing shared transpilation: ${name}`,
  );

await readFile(resolve(dist, "server/app/api/v1/[...path]/route.js"));
await readFile(resolve(app, "server.mjs"));
await readFile(resolve(app, "live-relay.mjs"));
const chunks = resolve(dist, "static/chunks/app");
const pages = (await readdir(chunks)).filter((name) =>
  /^(page|layout)-.*\.js$/.test(name),
);
assert.ok(
  pages.length,
  "No compiled application chunks found; run pnpm build first",
);
const sources = await Promise.all(
  pages.map((name) => readFile(resolve(chunks, name), "utf8")),
);
assert.ok(
  sources.some((source) => source.includes("react.memo_cache_sentinel")),
  "No React Compiler memoization in compiled app chunks",
);
assert.ok(
  sources.every(
    (source) =>
      !source.includes("ARGOS_RUST_API_ORIGIN") &&
      !source.includes("http://127.0.0.1:8080"),
  ),
  "Rust server configuration leaked into the browser chunks",
);
console.log(
  "Verified Next API server, RN Web/shared transpilation and compiled app memoization.",
);
