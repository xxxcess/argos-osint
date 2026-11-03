"use client";
import React, { useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { intelColors as c, intelRadius } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

export function Globe({ size = 36 }: { size?: number }) {
  const stroke = 2.5;
  return (
    <View
      testID="intel-globe"
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        borderWidth: stroke,
        borderColor: c.accent,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <View
        style={{
          position: "absolute",
          width: size * 0.4,
          height: size - 4,
          borderRadius: size / 2,
          borderWidth: stroke,
          borderColor: c.accent,
        }}
      />
      <View
        style={{ width: size - 4, height: stroke, backgroundColor: c.accent }}
      />
    </View>
  );
}

export function DeskButton({
  children,
  onPress,
  label,
  active = false,
  disabled = false,
}: {
  children: React.ReactNode;
  onPress: () => void;
  label?: string;
  active?: boolean;
  disabled?: boolean;
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
        active && s.active,
        hovered && !disabled && { borderColor: c.accentBorder },
        pressed && { backgroundColor: c.panelRaised },
      ]}
    >
      <Text
        style={[
          s.buttonText,
          active && { color: c.accent },
          disabled && { color: c.dim },
        ]}
      >
        {children}
      </Text>
    </Pressable>
  );
}

/** An actual retained identifier, never an invented sequential REF number. */
export function ReferenceChip({ id }: { id: string }) {
  return <Text style={s.chip}>{id}</Text>;
}

const s = StyleSheet.create({
  button: {
    minHeight: 44,
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderRadius: intelRadius.control,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.inset,
    justifyContent: "center",
    alignItems: "center",
  },
  buttonText: {
    fontFamily: intelFonts.mono,
    fontSize: 14,
    color: c.dim,
    flexShrink: 1,
  },
  active: { borderColor: c.accentBorder, backgroundColor: c.accentSoft },
  chip: {
    fontFamily: intelFonts.mono,
    fontSize: 19,
    color: c.accent,
    backgroundColor: c.accentSoft,
    borderColor: c.accentBorder,
    borderWidth: 1,
    borderRadius: intelRadius.chip,
    paddingHorizontal: 12,
    paddingVertical: 4,
    flexShrink: 1,
    minWidth: 0,
    maxWidth: "100%",
  },
});
