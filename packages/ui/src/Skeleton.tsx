"use client";
import React, { useEffect, useState } from "react";
import {
  AccessibilityInfo,
  Animated,
  Easing,
  View,
  type DimensionValue,
} from "react-native";

/** Steady placeholder base from the matte canvas theme. */
export const SKELETON_BASE = "#27313B";
const SHEEN_WIDTH = 0.2;
const SWEEP_MS = 1200;
const PAUSE_MS = 400;
const SHEEN = "169,200,217";
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
  // Steady base, constant opacity. One cool sheen band crosses the clipped region in a shared
  // 1.6 s cycle: a 1.2 s linear sweep, then 0.4 s outside the region. Nothing pulses.
  const [progress] = useState(() => new Animated.Value(0));
  const [regionWidth, setRegionWidth] = useState(0);
  useEffect(() => {
    let active = true;
    let animation: Animated.CompositeAnimation | undefined;
    const update = (reduced: boolean) => {
      animation?.stop();
      progress.setValue(0);
      if (!reduced && active) {
        animation = Animated.loop(
          Animated.sequence([
            Animated.timing(progress, {
              toValue: 1,
              duration: SWEEP_MS,
              easing: Easing.linear,
              useNativeDriver: true,
            }),
            Animated.delay(PAUSE_MS),
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
  }, [progress]);
  const bandWidth = regionWidth * SHEEN_WIDTH;
  const translateX = progress.interpolate({
    inputRange: [0, 1],
    outputRange: [-bandWidth, regionWidth],
  });
  return (
    <View
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={
        fill ? { width: "100%", height: "100%", flex: 1 } : { gap: 8, width }
      }
    >
      {Array.from({ length: fill ? 1 : lines }, (_, i) => (
        <View
          testID="skeleton-line"
          key={i}
          onLayout={(event) => setRegionWidth(event.nativeEvent.layout.width)}
          style={{
            height: fill ? "100%" : height,
            flex: fill ? 1 : undefined,
            borderRadius: radius,
            backgroundColor: SKELETON_BASE,
            overflow: "hidden",
          }}
        >
          {regionWidth > 0 ? (
            <Animated.View
              style={
                {
                  position: "absolute",
                  top: 0,
                  bottom: 0,
                  width: bandWidth,
                  backgroundImage: `linear-gradient(90deg, rgba(${SHEEN},0) 0%, rgba(${SHEEN},0.06) 50%, rgba(${SHEEN},0) 100%)`,
                  transform: [{ translateX }],
                } as object
              }
            />
          ) : null}
        </View>
      ))}
    </View>
  );
}
