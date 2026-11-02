"use client";
import React, { useEffect, useState } from "react";
import { Animated, Text, View } from "react-native";
import { intelColors as c } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";
import { useReducedMotion } from "../shell/motion";

/** Article heat runs from 0 (cold) to 1 (hot), in four equal bands. */
export function temperatureLabel(
  value: number | null | undefined,
): "Hot" | "Warm" | "Cool" | "Cold" | null {
  if (value == null || !Number.isFinite(value)) return null;
  if (value >= 0.75) return "Hot";
  if (value >= 0.5) return "Warm";
  if (value >= 0.25) return "Cool";
  return "Cold";
}

const color = {
  Hot: c.danger,
  Warm: c.warning,
  Cool: c.cool,
  Cold: c.dim,
} as const;

function HotTag() {
  const reduced = useReducedMotion();
  const [glow] = useState(() => new Animated.Value(0.35));
  useEffect(() => {
    if (reduced) {
      glow.setValue(0.4);
      return;
    }
    const pulse = Animated.loop(
      Animated.sequence([
        Animated.timing(glow, {
          toValue: 1,
          duration: 1400,
          useNativeDriver: true,
        }),
        Animated.timing(glow, {
          toValue: 0.4,
          duration: 1400,
          useNativeDriver: true,
        }),
      ]),
    );
    pulse.start();
    return () => pulse.stop();
  }, [reduced, glow]);
  return (
    <View style={{ alignSelf: "center", justifyContent: "center" }}>
      <Animated.View
        pointerEvents="none"
        style={{
          position: "absolute",
          top: -1,
          bottom: -1,
          left: -1,
          right: -1,
          borderRadius: 999,
          backgroundColor: c.danger,
          opacity: glow.interpolate({
            inputRange: [0.4, 1],
            outputRange: [0.08, 0.22],
          }),
        }}
      />
      <Text
        accessibilityLabel="Temperature Hot"
        style={{
          fontFamily: intelFonts.mono,
          fontSize: 12,
          fontWeight: "600",
          lineHeight: 16,
          color: c.danger,
          borderWidth: 1,
          borderColor: c.danger,
          borderRadius: 999,
          paddingHorizontal: 8,
          paddingVertical: 1,
        }}
      >
        Hot
      </Text>
    </View>
  );
}

export function TemperatureTag({
  value,
}: {
  value: number | null | undefined;
}) {
  const label = temperatureLabel(value);
  if (!label) return null;
  if (label === "Hot") return <HotTag />;
  return (
    <Text
      accessibilityLabel={`Temperature ${label}`}
      style={{
        alignSelf: "center",
        fontFamily: intelFonts.mono,
        fontSize: 12,
        fontWeight: "600",
        lineHeight: 16,
        color: color[label],
        borderWidth: 1,
        borderColor: color[label],
        borderRadius: 999,
        paddingHorizontal: 8,
        paddingVertical: 1,
        overflow: "hidden",
      }}
    >
      {label}
    </Text>
  );
}
