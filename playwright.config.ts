import { defineConfig } from "@playwright/test";
const development = process.env.ARGOS_WEB_PROFILE === "dev";
export default defineConfig({
  testDir: "tests/web",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:3100",
    viewport: { width: 1440, height: 1000 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: development
      ? "pnpm --dir apps/web-desktop dev --port 3100"
      : "node scripts/serve_web.mjs --port 3100",
    timeout: 120_000,
    url: "http://127.0.0.1:3100",
    reuseExistingServer: false,
  },
  reporter: "list",
});
