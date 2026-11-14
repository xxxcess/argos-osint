import { defineConfig } from "@playwright/test";
const development = process.env.ARGOS_WEB_PROFILE === "dev";
const port = Number(process.env.ARGOS_WEB_TEST_PORT || 3100);
export default defineConfig({
  testDir: "tests/web",
  // Fonts, locale and time are fixed by visual fixtures; share inspected Chromium baselines across hosts.
  snapshotPathTemplate: "{testDir}/{testFilePath}-snapshots/{arg}{ext}",
  fullyParallel: false,
  workers: 1,
  use: {
    // Linux otherwise rasterizes small text with LCD color fringes while macOS
    // uses grayscale. Keep shared screenshot baselines on the same text path.
    launchOptions: {
      args: ["--disable-lcd-text", "--font-render-hinting=none"],
    },
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1440, height: 1000 },
    trace: "retain-on-failure",
  },
  webServer: {
    // Local production fixtures can share the isolated build mapping; hosted
    // production runs retain their normal output unless explicitly requested.
    env: {
      ARGOS_WEB_TEST_DEV:
        development || process.env.ARGOS_WEB_TEST_DEV === "1" ? "1" : "0",
    },
    command: `node apps/web-desktop/__tests__/serve-connect-fixture.mjs --port ${port}${development ? " --dev" : ""}`,
    timeout: 120_000,
    url: `http://127.0.0.1:${port}/connect`,
    reuseExistingServer: false,
  },
  reporter: "list",
});
