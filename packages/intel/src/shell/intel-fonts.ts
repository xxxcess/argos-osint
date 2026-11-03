import { Platform } from "react-native";

/** CSS variables/font stacks belong to RN Web, never native fontFamily values. */
export function fontFamilies(platform: typeof Platform.OS) {
  if (platform === "web") {
    return {
      mono: 'var(--intel-mono, "JetBrains Mono"), "SF Mono", ui-monospace, monospace',
      body: "var(--intel-body, Inter), system-ui, -apple-system, sans-serif",
    };
  }
  // Native hosts have not bundled Inter/JetBrains Mono; use installed system faces.
  return {
    mono: platform === "ios" ? "Menlo" : "monospace",
    body: platform === "ios" ? "System" : "sans-serif",
  };
}

export const intelFonts = fontFamilies(Platform.OS);
