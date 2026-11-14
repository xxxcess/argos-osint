"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  ActivityIndicator,
} from "react-native";

// Matte Fleetworks paint. Existing geometry and semantic colors remain separate.
export const colors = {
  bg: "#060709",
  panel: "#0B0C0F",
  raised: "#111317",
  inset: "#060709",
  border: "#898274",
  borderSubtle: "#3A3833",
  text: "#F3EEE4",
  dim: "#D6D2C4",
  muted: "#B8B4A8",
  accent: "#C6A56E",
  accentPressed: "#C4A265",
  accentHover: "#D8BD8C",
  onAccent: "#060709",
  selectionSurface: "#322C22",
  accent2: "#c678dd",
  success: "#98c379",
  warning: "#e5c07b",
  danger: "#f08a8a",
  // Compatibility names retain existing consumers with opaque matte fills.
  glass: "#0B0C0F",
  glassStrong: "#111317",
  glassBorder: "#898274",
  cardEdge: "rgba(243, 238, 228, 0.02)",
  satinControlEdge: "rgba(243, 238, 228, 0.03)",
  accentSoft: "rgba(198, 165, 110, 0.1)",
};
export const fonts = {
  mono: 'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace',
  body: 'system-ui, -apple-system, "Segoe UI", Roboto, Arial, sans-serif',
};
// Web-only style props (RN Web supports them); typed loosely for RN.
type WebStyle = Record<string, unknown>;
export const matteCard: WebStyle = {
  backgroundColor: colors.panel,
  borderWidth: 1,
  borderColor: colors.cardEdge,
};
export const matteDarkControl: WebStyle = {
  backgroundColor: colors.raised,
  borderWidth: 1,
  borderColor: colors.border,
};
export const mutedGoldPrimary: WebStyle = {
  backgroundColor: colors.accent,
  borderColor: colors.accent,
};
// Existing consumers retain their API while the visual treatment becomes matte.
export const glass = matteCard;
export const glow = (color: string): WebStyle => ({
  boxShadow: `inset 0 0 0 1px ${color}`,
});
export type Tone = "accent" | "success" | "warning" | "danger" | "purple";
export const toneColor: Record<Tone, string> = {
  accent: colors.accent,
  success: colors.success,
  warning: colors.warning,
  danger: colors.danger,
  purple: colors.accent2,
};

/** Bracketed uppercase telemetry tag, e.g. [ALERT: DEFCON 2]. */
export function StatusTag({
  children,
  tone = "accent",
}: {
  children: React.ReactNode;
  tone?: Tone;
}) {
  const color = toneColor[tone];
  return (
    <Text style={[s.tag, { color, borderColor: color + "66" }]}>
      [{children}]
    </Text>
  );
}
/** Matte container retaining the existing tone-tinted top rule. */
export function Frame({
  children,
  tone = "accent",
}: {
  children: React.ReactNode;
  tone?: Tone;
}) {
  return (
    <View
      style={[
        s.frame,
        glass as object,
        { borderTopColor: toneColor[tone], borderTopWidth: 2 },
      ]}
    >
      {children}
    </View>
  );
}

export function Label({ children }: { children: React.ReactNode }) {
  return <Text style={s.label}>{children}</Text>;
}
export function Copy({ children }: { children: React.ReactNode }) {
  return <Text style={s.copy}>{children}</Text>;
}
export function LoadingState({
  label,
  compact = false,
}: {
  label: string;
  compact?: boolean;
}) {
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityState={{ busy: true }}
      style={[s.loading, compact && { paddingVertical: 0 }]}
    >
      <ActivityIndicator
        size="small"
        color={colors.accent}
        aria-hidden={true}
      />
      <Text style={s.copy}>{label}</Text>
    </View>
  );
}
export function EmptyState({
  title,
  children,
}: {
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <View style={s.empty}>
      <Text accessibilityRole="header" style={s.emptyTitle}>
        {title}
      </Text>
      {children != null ? <Copy>{children}</Copy> : null}
    </View>
  );
}

export function Button({
  children,
  onPress,
  active,
  disabled = false,
  label,
  variant = "default",
}: {
  children: React.ReactNode;
  onPress: () => void;
  active?: boolean;
  disabled?: boolean;
  label?: string;
  variant?: "default" | "quiet" | "primary";
}) {
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      aria-pressed={active}
      disabled={disabled}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={({ pressed }) => [
        s.button,
        variant === "quiet" && s.quiet,
        variant === "primary" && s.primary,
        !disabled &&
          hovered &&
          !pressed &&
          variant === "primary" &&
          s.primaryHover,
        !disabled && hovered && !pressed && variant !== "primary" && s.hovered,
        !disabled && pressed && variant === "primary" && s.primaryPressed,
        !disabled && pressed && variant !== "primary" && s.pressed,
        active && s.active,
        disabled && s.disabled,
      ]}
    >
      <Text
        style={[
          s.buttonText,
          variant === "quiet" && s.quietText,
          variant === "primary" && s.primaryText,
          active && s.activeText,
          disabled && s.disabledText,
        ]}
      >
        {children}
      </Text>
    </Pressable>
  );
}
export function Panel({
  children,
  title,
}: {
  children: React.ReactNode;
  title?: string;
}) {
  return (
    <View style={s.panel}>
      {title ? (
        <Text accessibilityRole="header" style={s.panelTitle}>
          {title}
        </Text>
      ) : null}
      {children}
    </View>
  );
}
export function Notice({ children }: { children: React.ReactNode }) {
  return (
    <View accessibilityRole="alert" style={s.notice}>
      <Text style={s.noticeText}>{children}</Text>
    </View>
  );
}
export { ErrorBoundary } from "./ErrorBoundary";
const s = StyleSheet.create({
  loading: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 10,
    paddingVertical: 20,
    minWidth: 0,
  },
  empty: { paddingVertical: 24, gap: 8, minWidth: 0 },
  emptyTitle: {
    color: colors.text,
    fontSize: 16,
    lineHeight: 24,
    fontWeight: "600",
  },
  label: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 19,
    color: colors.muted,
    textTransform: "uppercase",
    letterSpacing: 1.2,
  },
  copy: {
    fontFamily: fonts.body,
    color: colors.dim,
    fontSize: 15,
    lineHeight: 24,
  },
  button: {
    minHeight: 40,
    maxWidth: "100%",
    justifyContent: "center",
    paddingVertical: 9,
    paddingHorizontal: 12,
    borderWidth: 1,
    borderRadius: 2,
    backgroundColor: colors.raised,
    borderColor: colors.border,
  },
  buttonText: {
    textTransform: "uppercase",
    letterSpacing: 0.8,
    fontFamily: fonts.mono,
    color: colors.text,
    fontSize: 13,
    lineHeight: 20,
    flexShrink: 1,
  },
  quiet: { borderColor: colors.border, backgroundColor: "transparent" },
  quietText: { color: colors.dim },
  primary: {
    borderColor: colors.accent,
    backgroundColor: colors.accent,
  },
  primaryText: { color: colors.onAccent },
  hovered: {
    backgroundColor: colors.accentSoft,
    borderColor: colors.accent,
  },
  primaryHover: {
    backgroundColor: colors.accentHover,
    borderColor: colors.accentHover,
  },
  pressed: {
    borderColor: colors.accent,
    backgroundColor: colors.glassStrong,
  },
  primaryPressed: {
    borderColor: colors.accentPressed,
    backgroundColor: colors.accentPressed,
  },
  active: {
    borderColor: colors.accent,
    backgroundColor: colors.accent,
  },
  activeText: { color: colors.onAccent, fontWeight: "700" },
  disabled: {
    backgroundColor: "transparent",
    borderColor: colors.glassBorder,
  },
  disabledText: { color: colors.muted },
  panel: { ...(glass as object), padding: 20, gap: 16, minWidth: 0 },
  frame: { padding: 16, gap: 12, minWidth: 0 },
  tag: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 17,
    letterSpacing: 1,
    textTransform: "uppercase",
    fontWeight: "700",
  },
  panelTitle: {
    textTransform: "uppercase",
    letterSpacing: 1.5,
    fontFamily: fonts.mono,
    fontSize: 13,
    lineHeight: 21,
    color: colors.accent,
    paddingBottom: 14,
    marginBottom: 4,
    borderBottomWidth: 1,
    borderColor: colors.border,
  },
  notice: {
    paddingVertical: 12,
    paddingHorizontal: 16,
    borderLeftWidth: 2,
    borderColor: colors.danger,
    backgroundColor: "rgba(240, 138, 138, 0.1)",
    minWidth: 0,
  },
  noticeText: {
    fontFamily: fonts.body,
    fontSize: 15,
    lineHeight: 23,
    color: colors.danger,
  },
});

export { FleetworksMark } from "./FleetworksMark";
export { Skeleton } from "./Skeleton";
