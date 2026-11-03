"use client";
import React, { useEffect, useState } from "react";
import { Animated, View } from "react-native";
import { usePresence, useReducedMotion } from "../shell/motion";
import { intelMotion } from "../shell/intel-theme";
export function InspectorPresence({
  visible,
  children,
}: {
  visible: boolean;
  children: React.ReactNode;
}) {
  const reduced = useReducedMotion();
  const [snapshot, setSnapshot] = useState({
    visible,
    content: children,
    exiting: false,
  });
  if (visible && (snapshot.content !== children || !snapshot.visible))
    setSnapshot({ visible, content: children, exiting: false });
  else if (!visible && snapshot.visible)
    setSnapshot({ ...snapshot, visible: false, exiting: !reduced });
  const presence = usePresence(visible, intelMotion.inspector);
  useEffect(() => {
    if (!snapshot.exiting) return;
    const timer = setTimeout(
      () => setSnapshot((current) => ({ ...current, exiting: false })),
      reduced ? 0 : intelMotion.inspector,
    );
    return () => clearTimeout(timer);
  }, [snapshot.exiting, reduced]);
  if (!visible && (!snapshot.exiting || reduced))
    return <View>{children}</View>;
  return (
    <Animated.View
      aria-hidden={!visible}
      pointerEvents={visible ? "auto" : "none"}
      style={{
        opacity: presence.opacity,
        transform: [
          {
            translateX: presence.opacity.interpolate({
              inputRange: [0, 1],
              outputRange: [16, 0],
            }),
          },
        ],
      }}
    >
      {visible ? children : snapshot.content}
    </Animated.View>
  );
}
