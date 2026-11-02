"use client";
import React, { useEffect, useState } from "react";
import {
  AccessibilityInfo,
  Animated,
  View,
  type DimensionValue,
} from "react-native";
/** Decorative loading geometry. The caller owns the labeled status region. */
export function Skeleton({
  lines = 1,
  width = "100%",
  height = 16,
  radius = 4,
  fill = false,
}: {
  lines?: number;
  width?: DimensionValue;
  height?: number;
  radius?: number;
  /** Stretch one block to its parent. Used for the topology graph square. */
  fill?: boolean;
}) {
  const [opacity] = useState(() => new Animated.Value(0.55));
  useEffect(() => {
    let active = true;
    let animation: Animated.CompositeAnimation | undefined;
    const update = (reduced: boolean) => {
      animation?.stop();
      opacity.setValue(0.55);
      if (!reduced && active) {
        animation = Animated.loop(
          Animated.sequence([
            Animated.timing(opacity, {
              toValue: 1,
              duration: 900,
              useNativeDriver: true,
            }),
            Animated.timing(opacity, {
              toValue: 0.55,
              duration: 900,
              useNativeDriver: true,
            }),
          ]),
        );
        animation.start();
      }
    };
    void AccessibilityInfo.isReduceMotionEnabled().then((value) => {
      if (active) update(value);
    });
    const subscription = AccessibilityInfo.addEventListener(
      "reduceMotionChanged",
      update,
    );
    return () => {
      active = false;
      animation?.stop();
      subscription.remove();
    };
  }, [opacity]);
  return (
    <View
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={
        fill
          ? { width: "100%", height: "100%", flex: 1 }
          : { gap: 8, width }
      }
    >
      {Array.from({ length: fill ? 1 : lines }, (_, i) => (
        <Animated.View
          testID="skeleton-line"
          key={i}
          style={{
            height: fill ? "100%" : height,
            flex: fill ? 1 : undefined,
            borderRadius: radius,
            backgroundColor: "#34343A",
            opacity,
          }}
        />
      ))}
    </View>
  );
}
