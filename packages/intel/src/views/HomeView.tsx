"use client";
import React, { useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { Globe } from "./desk-ui";
import { intelColors as c, intelRadius, intelType } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

export function HomeView({ onOpen }: { onOpen: () => void }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  return (
    <View nativeID="intel-home" style={s.home}>
      <Pressable
        nativeID="intel-home-launcher"
        accessibilityRole="button"
        accessibilityLabel="Open Intel"
        onPress={onOpen}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        style={s.launcher}
      >
        {({ pressed }) => (
          <>
            <View
              testID="intel-launcher-tile"
              style={[
                s.tile,
                (hovered || focused) && {
                  borderColor: c.accentBorder,
                  transform: [{ translateY: -2 }],
                },
                pressed && { transform: [{ scale: 0.98 }] },
              ]}
            >
              <Globe size={84} />
            </View>
            <Text style={s.label}>INTEL</Text>
          </>
        )}
      </Pressable>
    </View>
  );
}
const s = StyleSheet.create({
  home: {
    flex: 1,
    minHeight: 320,
    justifyContent: "center",
    alignItems: "center",
    alignContent: "center",
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 48,
  },
  launcher: { alignItems: "center", gap: 24, padding: 4 },
  tile: {
    width: 204,
    height: 204,
    borderRadius: intelRadius.card,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.inset,
    alignItems: "center",
    justifyContent: "center",
  },
  label: { ...intelType.launcher, fontFamily: intelFonts.body, color: c.text },
});
