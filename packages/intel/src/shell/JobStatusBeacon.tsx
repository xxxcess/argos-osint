"use client";
import React, { useEffect, useState } from "react";
import { View, Text, Animated } from "react-native";
import { intelStatus } from "./intel-theme";
import { intelFonts } from "./intel-fonts";
import { useReducedMotion } from "./motion";
import type { JobState } from "./jobState";
const labels = {
  idle: "Brief idle",
  queued: "Brief queued",
  running: "Brief running",
  succeeded: "Brief complete",
  failed: "Brief failed",
  stale: "Brief updates interrupted",
};
export function JobStatusBeacon({ state }: { state: JobState }) {
  const token = intelStatus[state];
  const reduced = useReducedMotion();
  const [opacity] = useState(() => new Animated.Value(1));
  useEffect(() => {
    opacity.setValue(1);
    if (reduced || (state !== "queued" && state !== "running")) return;
    const pulse = Animated.loop(
      Animated.sequence([
        Animated.timing(opacity, {
          toValue: 0.45,
          duration: 900,
          useNativeDriver: true,
        }),
        Animated.timing(opacity, {
          toValue: 1,
          duration: 900,
          useNativeDriver: true,
        }),
      ]),
    );
    pulse.start();
    return () => pulse.stop();
  }, [state, reduced, opacity]);
  return (
    <View
      role="status"
      aria-live="polite"
      style={{ flexDirection: "row", alignItems: "center", gap: 8 }}
    >
      <Animated.View
        aria-hidden
        style={{
          width: 8,
          height: 8,
          borderRadius: 4,
          backgroundColor: token.color,
          boxShadow: token.glow,
          opacity,
        }}
      />
      <Text
        style={{
          fontFamily: intelFonts.mono,
          fontSize: 12,
          color: token.color,
        }}
      >
        {labels[state]}
      </Text>
    </View>
  );
}
