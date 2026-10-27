"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  ActivityIndicator,
} from "react-native";

// Dark HUD + glass tokens. Colors are the core TUI palette from
// crates/argos-osint-bin/src/tui/theme.rs; glass/glow is the web treatment (see docs/wiki/code/react-ui.md).
export const colors = {
  bg: "#141414",
  panel: "#1e1e1e",
  raised: "#303030",
  border: "#535353",
  text: "#e8e8e8",
  dim: "#b5b5b5",
  muted: "#919191",
  accent: "#87bfff",
  accent2: "#c678dd",
  success: "#98c379",
  warning: "#e5c07b",
  danger: "#f08a8a",
  glass: "rgba(30, 30, 30, 0.55)",
  glassStrong: "rgba(48, 48, 48, 0.72)",
  glassBorder: "rgba(83, 83, 83, 0.6)",
  accentSoft: "rgba(135, 191, 255, 0.14)",
};
export const fonts = {
  mono: 'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace',
  body: 'system-ui, -apple-system, "Segoe UI", Roboto, Arial, sans-serif',
};
// Web-only style props (RN Web supports them); typed loosely for RN.
type WebStyle = Record<string, unknown>;
export const glass: WebStyle = {
  backgroundColor: colors.glass,
  borderWidth: 1,
  borderColor: colors.glassBorder,
  backdropFilter: "blur(12px)",
  WebkitBackdropFilter: "blur(12px)",
};
export const glow = (color: string): WebStyle => ({
  boxShadow: `0 0 12px ${color}55, inset 0 0 0 1px ${color}33`,
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
/** Glass container with corner-accent HUD edge; `tone` tints the top rule. */
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
  variant?: "default" | "quiet";
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
        !disabled && (hovered || pressed) && s.hovered,
        active && s.active,
        disabled && s.disabled,
      ]}
    >
      <Text
        style={[
          s.buttonText,
          variant === "quiet" && s.quietText,
          active && s.activeText,
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
    fontSize: 15,
    lineHeight: 23,
    fontWeight: "600",
  },
  label: {
    fontFamily: fonts.mono,
    fontSize: 11,
    lineHeight: 18,
    color: colors.muted,
    textTransform: "uppercase",
    letterSpacing: 1.2,
  },
  copy: {
    fontFamily: fonts.body,
    color: colors.dim,
    fontSize: 14,
    lineHeight: 23,
  },
  button: {
    minHeight: 40,
    maxWidth: "100%",
    justifyContent: "center",
    paddingVertical: 9,
    paddingHorizontal: 12,
    borderWidth: 1,
    borderRadius: 2,
    backgroundColor: colors.glass,
    borderColor: colors.glassBorder,
  },
  buttonText: {
    textTransform: "uppercase",
    letterSpacing: 0.8,
    fontFamily: fonts.mono,
    color: colors.text,
    fontSize: 12,
    lineHeight: 19,
    flexShrink: 1,
  },
  quiet: { borderColor: "transparent" },
  quietText: { color: colors.dim },
  hovered: {
    backgroundColor: colors.accentSoft,
    borderColor: colors.accent,
  },
  active: {
    borderColor: colors.accent,
    backgroundColor: colors.accent,
    ...(glow(colors.accent) as object),
  },
  activeText: { color: colors.bg, fontWeight: "700" },
  disabled: { opacity: 0.45 },
  panel: { ...(glass as object), padding: 20, gap: 16, minWidth: 0 },
  frame: { padding: 16, gap: 12, minWidth: 0 },
  tag: {
    fontFamily: fonts.mono,
    fontSize: 11,
    lineHeight: 16,
    letterSpacing: 1,
    textTransform: "uppercase",
    fontWeight: "700",
  },
  panelTitle: {
    textTransform: "uppercase",
    letterSpacing: 1.5,
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 20,
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
    fontSize: 14,
    lineHeight: 22,
    color: colors.danger,
  },
});
