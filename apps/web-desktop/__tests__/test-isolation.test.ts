import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { expect, it } from "vitest";

it("gives verification a Next cache/lock outside the workspace launcher's output", () => {
  const readConfig = (testing: boolean) =>
    JSON.parse(
      execFileSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          'const {default: config} = await import("./next.config.mjs"); process.stdout.write(JSON.stringify({distDir:config.distDir, compiler:config.reactCompiler}));',
        ],
        {
          cwd: resolve("apps/web-desktop"),
          env: { ...process.env, ARGOS_WEB_TEST_DEV: testing ? "1" : "0" },
          encoding: "utf8",
        },
      ),
    );
  const user = readConfig(false);
  const tests = readConfig(true);
  expect(user.distDir).toBe(".next");
  expect(tests.distDir).toBe("../../.agent-scratch/next-web-tests");
  expect(tests.distDir).not.toBe(user.distDir);
  expect(tests.compiler).toEqual(user.compiler);
  expect(tests.compiler.panicThreshold).toBe("all_errors");
});

it.each(["dev", "production"])(
  "the %s browser runner selects the matching output directory",
  (profile) => {
    // Load Playwright in a Node host, not jsdom (which enables browser-only side effects).
    const config = JSON.parse(
      execFileSync(
        process.execPath,
        [
          "--experimental-strip-types",
          "--input-type=module",
          "-e",
          'const {default: config} = await import("./playwright.config.ts"); process.stdout.write(JSON.stringify(config.webServer));',
        ],
        {
          env: { ...process.env, ARGOS_WEB_PROFILE: profile },
          encoding: "utf8",
        },
      ),
    );
    expect(config).toMatchObject({
      env: { ARGOS_WEB_TEST_DEV: profile === "dev" ? "1" : "0" },
      reuseExistingServer: false,
    });
  },
);
