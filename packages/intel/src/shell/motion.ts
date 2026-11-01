"use client";
import { useEffect, useState, useSyncExternalStore } from "react";
import { AccessibilityInfo, Animated, Platform, Easing } from "react-native";
let nativeReduced = true;
function subscribeNativeMotion(listener: () => void) {
  let active = true;
  void AccessibilityInfo.isReduceMotionEnabled().then((value) => {
    if (active) {
      nativeReduced = value;
      listener();
    }
  });
  const subscription = AccessibilityInfo.addEventListener(
    "reduceMotionChanged",
    (value) => {
      nativeReduced = value;
      listener();
    },
  );
  return () => {
    active = false;
    subscription.remove();
  };
}
export function useReducedMotion() {
  return useSyncExternalStore(
    Platform.OS === "web" ? subscribeReducedMotion : subscribeNativeMotion,
    Platform.OS === "web" ? reducedMotionSnapshot : () => nativeReduced,
    () => true,
  );
}
import {
  reducedMotionSnapshot,
  subscribeReducedMotion,
} from "./motion-preference.web";
export function usePresence(
  visible: boolean,
  ms = 200,
  initiallyVisible = false,
) {
  const reduced = useReducedMotion();
  const [opacity] = useState(
    () => new Animated.Value(initiallyVisible ? 1 : 0),
  );
  useEffect(() => {
    const animation = Animated.timing(opacity, {
      toValue: visible ? 1 : 0,
      duration: reduced ? 0 : ms,
      easing: Easing.bezier(0.2, 0.7, 0.2, 1),
      useNativeDriver: true,
    });
    animation.start();
    return () => animation.stop();
  }, [visible, ms, reduced, opacity]);
  return {
    opacity,
    transform: [
      {
        translateY: opacity.interpolate({
          inputRange: [0, 1],
          outputRange: [8, 0],
        }),
      },
    ],
  };
}
export function fadeSlide(ms: number) {
  return {
    transitionDuration: `${ms}ms`,
    transitionProperty: "opacity, transform",
  };
}
