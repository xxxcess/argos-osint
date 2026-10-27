"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  ActivityIndicator,
} from "react-native";

// Web translation of crates/argos-osint-bin/src/tui/theme.rs.
export const colors = {
  bg: "#141414",
  panel: "#1e1e1e",
  raised: "#303030",
  border: "#535353",
  text: "#e8e8e8",
  dim: "#b5b5b5",
  muted: "#919191",
  accent: "#87bfff",
  success: "#98c379",
  warning: "#e5c07b",
  danger: "#f08a8a",
};
export const fonts = { mono: "monospace", body: "Arial" };

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
    borderColor: colors.border,
    borderRadius: 2,
    backgroundColor: "transparent",
  },
  buttonText: {
    fontFamily: fonts.mono,
    color: colors.text,
    fontSize: 12,
    lineHeight: 19,
    flexShrink: 1,
  },
  quiet: { borderColor: "transparent" },
  quietText: { color: colors.dim },
  hovered: { backgroundColor: colors.raised },
  active: { borderColor: colors.accent, backgroundColor: colors.accent },
  activeText: { color: colors.bg, fontWeight: "700" },
  disabled: { opacity: 0.45 },
  panel: { backgroundColor: colors.bg, padding: 20, gap: 16, minWidth: 0 },
  panelTitle: {
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
    backgroundColor: colors.panel,
    minWidth: 0,
  },
  noticeText: {
    fontFamily: fonts.body,
    fontSize: 14,
    lineHeight: 22,
    color: colors.danger,
  },
});
