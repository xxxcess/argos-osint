"use client";
import React from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
export const colors = {
  bg: "#0a0f14",
  panel: "#101820",
  raised: "#17232d",
  border: "#293740",
  text: "#dce5eb",
  muted: "#8c9da9",
  accent: "#80d7b7",
  danger: "#ffae94",
};
export function Label({ children }: { children: React.ReactNode }) {
  return <Text style={s.label}>{children}</Text>;
}
export function Copy({ children }: { children: React.ReactNode }) {
  return <Text style={s.copy}>{children}</Text>;
}
export function Button({
  children,
  onPress,
  active,
  disabled,
  label,
}: {
  children: React.ReactNode;
  onPress: () => void;
  active?: boolean;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      disabled={disabled}
      onPress={onPress}
      style={[s.button, active && s.active, disabled && { opacity: 0.4 }]}
    >
      <Text
        style={{ color: active ? colors.accent : colors.text, fontSize: 13 }}
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
      {title && <Label>{title}</Label>}
      {children}
    </View>
  );
}
export function Notice({ children }: { children: React.ReactNode }) {
  return (
    <View accessibilityRole="alert" style={s.notice}>
      <Text style={{ color: colors.danger }}>{children}</Text>
    </View>
  );
}
const s = StyleSheet.create({
  label: {
    fontSize: 10,
    letterSpacing: 2,
    color: colors.muted,
    textTransform: "uppercase",
    marginBottom: 15,
    fontWeight: "700",
  },
  copy: { color: colors.text, fontSize: 14, lineHeight: 23 },
  button: {
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderWidth: 1,
    borderColor: colors.border,
    borderRadius: 4,
    backgroundColor: colors.panel,
  },
  active: { borderColor: colors.accent, backgroundColor: "#152b28" },
  panel: { backgroundColor: colors.panel, padding: 24, gap: 14, minWidth: 0 },
  notice: {
    padding: 16,
    borderWidth: 1,
    borderColor: "#765345",
    backgroundColor: "#2b201b",
    borderRadius: 4,
  },
});
