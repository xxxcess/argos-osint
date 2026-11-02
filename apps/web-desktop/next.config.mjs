import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const nativeWeb = require.resolve("react-native-web");

/** @type {import('next').NextConfig} */
export default {
  // The dev badge is a nextjs-portal overlay. It is not part of the Intel desk.
  devIndicators: false,
  // Browser verification must never claim the launcher's .next/dev lock/cache.
  distDir:
    process.env.ARGOS_WEB_TEST_DEV === "1"
      ? "../../.agent-scratch/next-web-tests"
      : ".next",
  reactCompiler: {
    panicThreshold: "all_errors",
    environment: { enablePreserveExistingMemoizationGuarantees: true },
  },
  // The pinned force-graph drag-cancel guard is applied via pnpm patchedDependencies.
  transpilePackages: [
    "@argos/ui",
    "@argos/intel",
    "@argos/api-client",
    "react-native-web",
  ],
  webpack(config) {
    config.resolve.alias = {
      ...config.resolve.alias,
      "react-native$": nativeWeb,
      "react-native": nativeWeb,
    };
    return config;
  },
};
