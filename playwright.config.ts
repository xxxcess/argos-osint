import { defineConfig } from "@playwright/test";
const development = process.env.ARGOS_WEB_PROFILE === "dev";
const port = Number(process.env.ARGOS_WEB_TEST_PORT || 3100);
export default defineConfig({
  testDir: "tests/web",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1440, height: 1000 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: development
      ? `pnpm --dir apps/web-desktop dev --port ${port}`
      : `node scripts/serve_web.mjs --port ${port}`,
    timeout: 120_000,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
  },
  reporter: "list",
});
