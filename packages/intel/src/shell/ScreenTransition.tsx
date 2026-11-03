"use client";
import React, { useEffect, useState } from "react";
import { Animated, View } from "react-native";
import { usePresence, useReducedMotion } from "./motion";
import { intelMotion } from "./intel-theme";
const order: Record<string, number> = {
  connection: -1,
  home: 0,
  bulletin: 1,
  brief: 2,
  actor: 3,
  topology: 4,
};
export function transitionDirection(from: string, to: string) {
  return (order[to] ?? 0) >= (order[from] ?? 0) ? 1 : -1;
}
type Snapshot = { screen: string; content: React.ReactNode };
/** Stable sibling keys retain the outgoing component, avoiding replayed effects. */
export function ScreenTransition({
  children,
  screen,
  fill = false,
}: {
  children: React.ReactNode;
  screen: string;
  fill?: boolean;
}) {
  const reduced = useReducedMotion();
  const [snapshot, setSnapshot] = useState({
    screen,
    content: children,
    previous: null as Snapshot | null,
    direction: 1,
  });
  if (snapshot.screen !== screen)
    setSnapshot({
      screen,
      content: children,
      previous: reduced
        ? null
        : { screen: snapshot.screen, content: snapshot.content },
      direction: transitionDirection(snapshot.screen, screen),
    });
  else if (snapshot.content !== children)
    setSnapshot({ ...snapshot, content: children });
  useEffect(() => {
    if (!snapshot.previous) return;
    const timer = setTimeout(
      () => setSnapshot((current) => ({ ...current, previous: null })),
      reduced ? 0 : intelMotion.page,
    );
    return () => clearTimeout(timer);
  }, [snapshot.previous, reduced]);
  const screens: {
    screen: string;
    content: React.ReactNode;
    exiting: boolean;
  }[] = [];
  if (!reduced && snapshot.previous && snapshot.previous.screen !== screen)
    screens.push({ ...snapshot.previous, exiting: true });
  screens.push({ screen, content: children, exiting: false });
  return (
    <View
      style={[
        { minWidth: 0, width: "100%" },
        fill && { flex: 1, minHeight: 0 },
      ]}
    >
      {screens.map((view) => (
        <Transition
          key={view.screen}
          exiting={view.exiting}
          fill={fill}
          direction={snapshot.direction}
        >
          {view.content}
        </Transition>
      ))}
    </View>
  );
}
function Transition({
  children,
  direction,
  exiting,
  fill,
}: {
  children: React.ReactNode;
  direction: number;
  exiting: boolean;
  fill: boolean;
}) {
  const presence = usePresence(!exiting, intelMotion.page);
  return (
    <Animated.View
      aria-hidden={exiting || undefined}
      importantForAccessibility={exiting ? "no-hide-descendants" : "auto"}
      pointerEvents={exiting ? "none" : "auto"}
      {...(exiting ? { inert: true } : {})}
      style={[
        { minWidth: 0, width: "100%" },
        fill && { flex: 1, minHeight: 0 },
        exiting && { position: "absolute", top: 0, left: 0, right: 0 },
        presence,
        {
          transform: [
            {
              translateY: presence.opacity.interpolate({
                inputRange: [0, 1],
                outputRange: [8 * direction, 0],
              }),
            },
          ],
        },
      ]}
    >
      {children}
    </Animated.View>
  );
}
