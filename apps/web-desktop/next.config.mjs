import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const nativeWeb = require.resolve("react-native-web");

/** @type {import('next').NextConfig} */
export default {
  output: "export",
  reactCompiler: {
    panicThreshold: "all_errors",
    environment: { enablePreserveExistingMemoizationGuarantees: true },
  },
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
