"use client";
import React, { lazy, Suspense, useState, useSyncExternalStore } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import type { Health } from "@argos/api-client";
import { FleetworksMark } from "@argos/ui";
import { StatusMessage } from "./StatusMessage";
import { useConnectionSequence } from "./sequence";
import { useReducedMotion } from "../shell/motion";
import {
  landingColors,
  intelColors,
  topologyColors,
} from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";
import { connectionChecks } from "./checks";
import { webGL } from "./webgl.web";
const Canvas = lazy(() => import("./OrbitCanvas"));
const subscribe = () => () => {};
export function OrbitFallback({
  coreColor = landingColors.core,
}: {
  coreColor?: string;
}) {
  return (
    <View testID="landing-fallback" style={s.fallback}>
      <View
        testID="landing-core"
        style={[s.halo, { boxShadow: `0 0 80px ${coreColor}` }]}
      />
      <View testID="landing-mark" style={s.mark}>
        <FleetworksMark size={52} />
      </View>
    </View>
  );
}
export function OrbitScene({
  ready = false,
  health,
  connecting = true,
  error,
  onRetry,
}: {
  ready?: boolean;
  health?: Health;
  connecting?: boolean;
  error?: string;
  onRetry?: () => void;
}) {
  const reduced = useReducedMotion();
  const hydrated = useSyncExternalStore(
    subscribe,
    () => true,
    () => false,
  );
  const supported = useSyncExternalStore(subscribe, webGL, () => false);
  const [box, setBox] = useState({ width: 0, height: 0 });
  const animated = hydrated && supported && !reduced;
  const sequence = useConnectionSequence(
    connectionChecks(health),
    ready,
    animated,
  );
  const coreColor = {
    offline: intelColors.danger,
    checking: landingColors.core,
    healthy: intelColors.success,
    ready: intelColors.cool,
  }[sequence.phase];
  const message = sequence.active
    ? sequence.active.label
    : sequence.phase === "healthy"
      ? "All connections healthy"
      : ready
        ? "Workspace ready"
        : connecting
          ? "Checking local API…"
          : error
            ? "Local API unavailable"
            : "Waiting for workspace readiness";
  return (
    <View
      nativeID="intel-landing"
      onLayout={(event) =>
        setBox({
          width: event.nativeEvent.layout.width,
          height: event.nativeEvent.layout.height,
        })
      }
      style={{
        flex: 1,
        minHeight: 0,
        width: "100%",
        backgroundColor: topologyColors.bg,
        overflow: "hidden",
      }}
    >
      <View
        aria-hidden
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        style={StyleSheet.absoluteFill}
      >
        {supported && box.width > 0 && box.height > 0 ? (
          <OrbitBoundary coreColor={coreColor}>
            <Suspense fallback={<OrbitFallback coreColor={coreColor} />}>
              <Canvas
                reduced={reduced}
                width={box.width}
                height={box.height}
                coreColor={coreColor}
              />
            </Suspense>
          </OrbitBoundary>
        ) : (
          <OrbitFallback coreColor={coreColor} />
        )}
      </View>
      <View
        style={{
          position: "absolute",
          top: "50%",
          left: 16,
          right: 16,
          paddingTop: 40,
          alignItems: "center",
          gap: 8,
        }}
      >
        <StatusMessage
          key={message}
          message={message}
          color={
            sequence.phase === "healthy" || sequence.phase === "ready"
              ? coreColor
              : intelColors.dim
          }
          reduced={reduced}
        />
        <View
          role="list"
          accessibilityLabel="Connection checks"
          style={{
            position: "absolute",
            width: 1,
            height: 1,
            overflow: "hidden",
          }}
        >
          {sequence.revealed.map((check) => (
            <Text key={check.id} role="listitem">
              {check.label} passed
            </Text>
          ))}
        </View>
        {!connecting && !ready && onRetry ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Retry connection checks"
            onPress={onRetry}
            style={{
              padding: 12,
              borderWidth: 1,
              borderColor: intelColors.borderStrong,
              borderRadius: 4,
            }}
          >
            <Text
              style={{
                color: intelColors.accent,
                fontFamily: intelFonts.mono,
                fontSize: 13,
              }}
            >
              Retry connection checks
            </Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}
class OrbitBoundary extends React.Component<
  {
    children: React.ReactNode;
    coreColor: string;
  },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <OrbitFallback coreColor={this.props.coreColor} />
    ) : (
      this.props.children
    );
  }
}

const s = StyleSheet.create({
  fallback: {
    ...StyleSheet.absoluteFillObject,
    alignItems: "center",
    justifyContent: "center",
  },
  halo: {
    width: 28,
    height: 28,
    borderRadius: 14,
    backgroundColor: landingColors.core,
  },
  // Align the SVG's warm center (at y=61 of 176) with the halo.
  mark: { position: "absolute", transform: [{ translateY: 10 }] },
});
