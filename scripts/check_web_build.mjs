import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Check the produced build, not just the source flag. The framework runtime
// contains memoization symbols even when application compilation is disabled.
const app = process.argv[2]
  ? resolve(process.argv[2])
  : fileURLToPath(new URL("../apps/web-desktop/", import.meta.url));
const { config } = JSON.parse(
  await readFile(resolve(app, ".next/required-server-files.json"), "utf8"),
);
assert.equal(config.output, "export", "Expected a Next static export");
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

await readFile(resolve(app, "out/index.html"));
const chunks = resolve(app, "out/_next/static/chunks/app");
const pages = (await readdir(chunks)).filter((name) =>
  /^page-.*\.js$/.test(name),
);
assert.ok(
  pages.length,
  "No exported application page chunks found; run pnpm build first",
);
const sources = await Promise.all(
  pages.map((name) => readFile(resolve(chunks, name), "utf8")),
);
assert.ok(
  sources.some((source) => source.includes("react.memo_cache_sentinel")),
  "No React Compiler memoization in exported app page chunks",
);
console.log(
  "Verified static export, RN Web/shared transpilation and compiled app memoization.",
);
