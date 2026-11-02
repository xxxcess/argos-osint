"use client";
import React, {
  lazy,
  Suspense,
  useEffect,
  useCallback,
  useState,
  useSyncExternalStore,
} from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import type { Health } from "@argos/api-client";
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
  count = 0,
  coreColor = landingColors.core,
}: {
  count?: number;
  coreColor?: string;
}) {
  return (
    <View
      testID="landing-fallback"
      style={[
        StyleSheet.absoluteFill,
        { alignItems: "center", justifyContent: "center" },
      ]}
    >
      <View
        testID="landing-core"
        style={{
          width: 28,
          height: 28,
          borderRadius: 14,
          backgroundColor: coreColor,
          boxShadow: `0 0 80px ${coreColor}`,
        }}
      />
      {Array.from({ length: count }, (_, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: "50%",
            top: "50%",
            width: 8,
            height: 8,
            borderRadius: 4,
            backgroundColor:
              i % 2 ? landingColors.orbiterAlt : landingColors.orbiter,
            transform: [
              { translateX: Math.cos(i * 2.399) * 110 },
              { translateY: Math.sin(i * 2.399) * 110 },
            ],
            boxShadow: `0 0 12px ${landingColors.orbiter}`,
          }}
        />
      ))}
    </View>
  );
}
export function OrbitScene({
  onComplete,
  ready = false,
  health,
  connecting = true,
  error,
  onRetry,
}: {
  onComplete?: () => void;
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
  const revealKey = sequence.revealed.map((check) => check.id).join(":");
  const [settled, setSettled] = useState("");
  const markSettled = useCallback(() => setSettled(revealKey), [revealKey]);
  useEffect(() => {
    if (!hydrated || !ready) return;
    if (!animated || (sequence.finished && settled === revealKey))
      onComplete?.();
  }, [
    hydrated,
    ready,
    animated,
    sequence.finished,
    settled,
    revealKey,
    onComplete,
  ]);
  const coreColor = {
    offline: intelColors.danger,
    checking: landingColors.core,
    healthy: intelColors.accent,
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
          <OrbitBoundary
            onComplete={markSettled}
            count={sequence.revealed.length}
            coreColor={coreColor}
          >
            <Suspense
              fallback={
                <OrbitFallback
                  count={sequence.revealed.length}
                  coreColor={coreColor}
                />
              }
            >
              <Canvas
                reduced={reduced}
                onComplete={markSettled}
                ready={
                  ready && sequence.revealed.length === sequence.passed.length
                }
                checks={sequence.revealed.map((check) => check.id)}
                width={box.width}
                height={box.height}
                coreColor={coreColor}
              />
            </Suspense>
          </OrbitBoundary>
        ) : (
          <OrbitFallback
            count={sequence.revealed.length}
            coreColor={coreColor}
          />
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
function CompletedFallback({
  onComplete,
  count,
  coreColor,
}: {
  onComplete?: () => void;
  count: number;
  coreColor: string;
}) {
  useEffect(() => onComplete?.(), [onComplete]);
  return <OrbitFallback count={count} coreColor={coreColor} />;
}
class OrbitBoundary extends React.Component<
  {
    children: React.ReactNode;
    onComplete?: () => void;
    count: number;
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
      <CompletedFallback
        onComplete={this.props.onComplete}
        count={this.props.count}
        coreColor={this.props.coreColor}
      />
    ) : (
      this.props.children
    );
  }
}
