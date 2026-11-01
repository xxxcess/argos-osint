"use client";
import React, { useSyncExternalStore } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import { intelColors as c, intelMetricColors } from "./intel-theme";
import { intelFonts } from "./intel-fonts";

const subscribeHydration = () => () => {};
const clientSnapshot = () => true;
const serverSnapshot = () => false;

export type IntelMetrics = {
  cpuPercent?: number | null;
  memoryPercent?: number | null;
  diskPercent?: number | null;
  networkKBps?: number | null;
};
function metric(value: number | null | undefined, suffix: string) {
  return value != null && Number.isFinite(value) && value >= 0
    ? `${Math.round(value)}${suffix}`
    : "—";
}

/** Caller owns metrics and clock; missing telemetry never implies a healthy value. */
export function IntelStatusBar({
  connected,
  activeApp,
  metrics,
  clock,
  onHome,
  onIntel,
}: {
  connected: boolean;
  activeApp: boolean;
  metrics?: IntelMetrics;
  clock?: string;
  onHome?: () => void;
  onIntel?: () => void;
}) {
  const dimensions = useWindowDimensions();
  // RN Web reads browser dimensions before hydration. The initial tree must
  // instead match the zero-width static export; adapt after hydration commits.
  const hydrated = useSyncExternalStore(
    subscribeHydration,
    clientSnapshot,
    serverSnapshot,
  );
  const width = hydrated ? dimensions.width : 0;
  const wide = width >= 1100;
  const networkValue = metric(metrics?.networkKBps, "");
  const networkAvailable = networkValue !== "—";
  return (
    <View nativeID="intel-status-bar" style={s.bar}>
      <View style={[s.cluster, s.navCluster]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Desktop"
          onPress={onHome}
          disabled={!onHome}
          style={s.home}
        >
          <View
            accessibilityElementsHidden
            importantForAccessibility="no-hide-descendants"
            style={s.server}
          >
            <View style={s.serverUnit}>
              <View style={s.serverLight} />
            </View>
            <View style={s.serverUnit}>
              <View style={s.serverLight} />
            </View>
          </View>
        </Pressable>
        <View style={s.divider} />
        {activeApp ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Bulletin"
            aria-pressed={activeApp}
            onPress={onIntel}
            disabled={!onIntel}
            style={s.app}
          >
            <View style={s.appLabel}>
              <View
                accessibilityElementsHidden
                importantForAccessibility="no-hide-descendants"
                style={s.globe}
              >
                <View style={s.meridian} />
                <View style={s.equator} />
              </View>
              <Text
                nativeID="intel-status-app-label"
                style={[s.appText, wide && s.appTextWide]}
              >
                {width < 600 ? "Intel" : "intelligence"}
              </Text>
            </View>
          </Pressable>
        ) : null}
      </View>
      <View nativeID="intel-status-details" style={s.cluster}>
        {wide ? (
          <>
            <Text nativeID="intel-status-metric-cpu" style={s.telemetry}>
              CPU{" "}
              <Text style={{ color: intelMetricColors.cpu }}>
                {metric(metrics?.cpuPercent, "%")}
              </Text>
            </Text>
            <Text nativeID="intel-status-metric-memory" style={s.telemetry}>
              MEM{" "}
              <Text style={{ color: intelMetricColors.memory }}>
                {metric(metrics?.memoryPercent, "%")}
              </Text>
            </Text>
            <Text nativeID="intel-status-metric-disk" style={s.telemetry}>
              DSK{" "}
              <Text style={{ color: intelMetricColors.disk }}>
                {metric(metrics?.diskPercent, "%")}
              </Text>
            </Text>
            <View
              nativeID="intel-status-metric-network"
              role="group"
              aria-label={`NET ${networkValue}${networkAvailable ? " KB/s" : ""}`}
              style={s.networkMetric}
            >
              <Text style={s.telemetry}>NET</Text>
              <View>
                <Text
                  style={[s.telemetry, { color: intelMetricColors.network }]}
                >
                  {networkValue}
                </Text>
                {networkAvailable ? (
                  <Text
                    style={[s.telemetry, { color: intelMetricColors.network }]}
                  >
                    KB/s
                  </Text>
                ) : null}
              </View>
            </View>
            <View nativeID="intel-status-metric-divider" style={s.divider} />
          </>
        ) : null}
        <Text
          nativeID="intel-status-api"
          accessibilityLabel={connected ? "API connected" : "API offline"}
          style={[
            s.api,
            { fontSize: width >= 600 ? 18 : 12 },
            { color: connected ? c.accent : c.danger },
          ]}
        >
          ● {connected ? "API_OK" : "OFFLINE"}
        </Text>
        {wide ? (
          <Text nativeID="intel-status-clock" style={s.clock}>
            {clock || "—"}
          </Text>
        ) : null}
      </View>
    </View>
  );
}

export function IntelScrollFab({
  remaining,
  onPress,
}: {
  remaining: number;
  onPress: () => void;
}) {
  if (!Number.isFinite(remaining) || remaining <= 200) return null;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Scroll to bottom"
      onPress={onPress}
      style={({ pressed }) => [s.fab, pressed && { borderColor: c.select }]}
    >
      <Text style={s.arrow}>↓</Text>
    </Pressable>
  );
}

const s = StyleSheet.create({
  bar: {
    height: 64,
    flexShrink: 0,
    paddingHorizontal: 16,
    backgroundColor: c.panel,
    borderTopWidth: 1,
    borderColor: c.border,
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 8,
  },
  cluster: { flexDirection: "row", alignItems: "center", gap: 16, minWidth: 0 },
  navCluster: { height: "100%" },
  home: {
    minWidth: 40,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  server: { gap: 4, width: 28 },
  serverUnit: {
    height: 12,
    borderWidth: 2,
    borderColor: c.accent,
    borderRadius: 3,
    justifyContent: "center",
    paddingLeft: 3,
  },
  serverLight: {
    width: 3,
    height: 3,
    borderRadius: 2,
    backgroundColor: c.accent,
  },
  appLabel: { flexDirection: "row", alignItems: "center", gap: 8 },
  globe: {
    width: 18,
    height: 18,
    borderWidth: 1.5,
    borderColor: c.accent,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
  },
  meridian: {
    position: "absolute",
    width: 8,
    height: 16,
    borderWidth: 1,
    borderColor: c.accent,
    borderRadius: 8,
  },
  equator: { width: 16, height: 1, backgroundColor: c.accent },
  divider: { width: 1, height: 24, backgroundColor: c.borderStrong },
  app: {
    height: "100%",
    justifyContent: "center",
    paddingHorizontal: 12,
    backgroundColor: c.inset,
    borderBottomWidth: 2,
    borderBottomColor: c.accent,
  },
  appText: {
    fontFamily: intelFonts.mono,
    fontSize: 16,
    fontWeight: "700",
    color: c.accent,
  },
  appTextWide: { fontSize: 20 },
  telemetry: {
    fontFamily: intelFonts.mono,
    fontSize: 18,
    color: c.dim,
    fontVariant: ["tabular-nums"],
  },
  api: { fontFamily: intelFonts.mono, fontSize: 12, fontWeight: "600" },
  networkMetric: { flexDirection: "row", alignItems: "center", gap: 8 },
  clock: {
    fontFamily: intelFonts.mono,
    fontSize: 18,
    color: c.dim,
    fontVariant: ["tabular-nums"],
  },
  fab: {
    position: "absolute",
    bottom: 88,
    alignSelf: "center",
    width: 56,
    height: 56,
    borderRadius: 28,
    backgroundColor: c.fab,
    borderWidth: 1,
    borderColor: c.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  arrow: { color: c.text, fontSize: 28 },
});
