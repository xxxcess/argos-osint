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
// One phase drives every mounted skeleton, so bands sweep together instead of flashing
// independently. The loop runs while any skeleton is mounted, motion is allowed and the page
// is visible; the last skeleton to unmount stops it.
const phase = {
  progress: new Animated.Value(0),
  users: 0,
  reduced: true, // until the preference is known, stay still
  animation: undefined as Animated.CompositeAnimation | undefined,
  stop: undefined as (() => void) | undefined,
};
const pageHidden = () =>
  typeof document !== "undefined" && document.visibilityState === "hidden";
function syncPhase() {
  phase.animation?.stop();
  phase.animation = undefined;
  phase.progress.setValue(0);
  if (phase.users > 0 && !phase.reduced && !pageHidden()) {
    phase.animation = Animated.loop(
      Animated.sequence([
        Animated.timing(phase.progress, {
          toValue: 1,
          duration: SWEEP_MS,
          easing: Easing.linear,
          useNativeDriver: true,
        }),
        Animated.delay(PAUSE_MS),
      ]),
    );
    phase.animation.start();
  }
}
function acquirePhase(): () => void {
  if (phase.users++ === 0) {
    let live = true;
    phase.reduced = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((value) => {
      if (live) {
        phase.reduced = value;
        syncPhase();
      }
    });
    const subscription = AccessibilityInfo.addEventListener(
      "reduceMotionChanged",
      (value: boolean) => {
        phase.reduced = value;
        syncPhase();
      },
    );
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", syncPhase);
    }
    phase.stop = () => {
      live = false;
      phase.animation?.stop();
      phase.animation = undefined;
      subscription.remove();
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", syncPhase);
      }
    };
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    if (--phase.users === 0) phase.stop?.();
  };
}
/** Decorative loading geometry. The caller owns the labeled status region. */
export function Skeleton({
  lines = 1,
  width = "100%",
  height = 16,
  radius = 4,
  fill = false,
  baseColor = SKELETON_BASE,
}: {
  lines?: number;
  width?: DimensionValue;
  height?: number;
  radius?: number;
  /** Steady fill for a placeholder inheriting its surface context. */
  baseColor?: string;
  /** Stretch one block to its parent. Used for the topology graph square. */
  fill?: boolean;
}) {
  // Steady base, constant opacity. One cool sheen band crosses the clipped region in a shared
  // 1.6 s cycle: a 1.2 s linear sweep, then 0.4 s outside the region. Nothing pulses.
  const progress = phase.progress;
  const [regionWidth, setRegionWidth] = useState(0);
  useEffect(() => acquirePhase(), []);
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
            backgroundColor: baseColor,
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
