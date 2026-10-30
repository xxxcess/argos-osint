import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";
import { createRequire } from "node:module";

// React Native primitives resolve to react-native-web (same alias the Next
// static export applies in apps/web-desktop/next.config.mjs).
const require = createRequire(import.meta.url);
const nativeWeb = require.resolve("react-native-web", {
  paths: [path.join(__dirname, "apps/web-desktop")],
});

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.tsx"],
    include: [
      "packages/**/*.test.{ts,tsx}",
      "apps/**/*.test.{ts,tsx}",
    ],
    coverage: {
      provider: "v8",
      reporter: ["text", "json", "html"],
      exclude: [
        "node_modules/",
        "**/*.test.{ts,tsx}",
        "**/__tests__/**",
        "vitest.config.ts",
        "vitest.setup.tsx",
      ],
    },
    // Transform setup files with Vite plugins
    transformMode: {
      web: [/\.[jt]sx?$/],
    },
    // Run from workspace root to resolve aliases correctly
    root: ".",
  },
  resolve: {
    alias: {
      "@argos/ui": path.resolve(__dirname, "./packages/ui/src"),
      "@argos/intel": path.resolve(__dirname, "./packages/intel/src"),
      "@argos/api-client": path.resolve(__dirname, "./packages/api-client/src"),
      "@argos/web-desktop": path.resolve(__dirname, "./apps/web-desktop"),
      // Test the real React Native components against react-native-web, like the app build does.
      "react-native": nativeWeb,
      "react-native$": nativeWeb,
    },
  },
});