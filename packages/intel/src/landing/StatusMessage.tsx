"use client";
import React, { useEffect, useState } from "react";
import { Animated, Text, View, Platform } from "react-native";
import { intelFonts } from "../shell/intel-fonts";
/** 600ms in, 1800ms readable, 600ms out: one three-second message cycle. */
export function StatusMessage({
  message,
  color,
  reduced,
}: {
  message: string;
  color: string;
  reduced: boolean;
}) {
  const [opacity] = useState(() => new Animated.Value(reduced ? 1 : 0));
  useEffect(() => {
    if (Platform.OS === "web") return;
    if (reduced) {
      opacity.setValue(1);
      return;
    }
    const animation = Animated.loop(
      Animated.sequence([
        Animated.timing(opacity, {
          toValue: 1,
          duration: 600,
          useNativeDriver: true,
        }),
        Animated.delay(1800),
        Animated.timing(opacity, {
          toValue: 0,
          duration: 600,
          useNativeDriver: true,
        }),
      ]),
    );
    animation.start();
    return () => animation.stop();
  }, [opacity, reduced]);
  const Wrapper = Platform.OS === "web" ? View : Animated.View;
  return (
    <Wrapper
      nativeID="intel-landing-status"
      style={Platform.OS === "web" ? undefined : { opacity }}
    >
      <Text
        role="status"
        aria-live="polite"
        accessibilityLabel={message}
        style={{
          color,
          fontFamily: intelFonts.mono,
          fontSize: 12,
          textAlign: "center",
        }}
      >
        {message}
      </Text>
    </Wrapper>
  );
}
