"use client";
import React, { useState, useSyncExternalStore } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import {
  intelColors as c,
  intelMetricColors,
  intelStatus,
} from "./intel-theme";
import { intelFonts } from "./intel-fonts";
import { FleetworksMark } from "@argos/ui";
import type { JobState } from "./jobState";

const subscribeHydration = () => () => {};
const clientSnapshot = () => true;
const serverSnapshot = () => false;

export type StatusJob = {
  id: string;
  label: string;
  detail?: string;
  state: JobState;
};

export type IntelMetrics = {
  cpuPercent?: number | null;
  memoryPercent?: number | null;
  diskPercent?: number | null;
  networkKBps?: number | null;
  platform?: string | null;
  chipset?: string | null;
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
  job,
  hideConnectionIndicator = false,
}: {
  job?: React.ReactNode;
  hideConnectionIndicator?: boolean;
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
  const navSize = width >= 600 ? 18 : 13;
  const networkValue = metric(metrics?.networkKBps, "");
  const networkAvailable = networkValue !== "—";
  const platform = metrics?.platform?.trim();
  const platformShort = platform?.split(/\s+/)[0] || "server";
  const chipset = metrics?.chipset?.trim() || "";
  const serverLabel = chipset
    ? `${platform || "server"}, ${chipset}`
    : platform || "server";
  return (
    <View nativeID="intel-status-bar" style={s.bar}>
      <View style={[s.cluster, s.navCluster]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={serverLabel}
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
          <Text numberOfLines={1} style={[s.serverName, { fontSize: navSize }]}>
            {platformShort}
          </Text>
          {chipset ? (
            <Text style={[s.chipBadge, { fontSize: navSize }]}>{chipset}</Text>
          ) : null}
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
              <FleetworksMark size={18} />
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
              <Text numberOfLines={1} style={s.telemetry}>
                NET
              </Text>
              <Text
                numberOfLines={1}
                style={[
                  s.telemetry,
                  { color: intelMetricColors.network, flexShrink: 0 },
                ]}
              >
                {networkValue}
                {networkAvailable ? " KB/s" : ""}
              </Text>
            </View>
            <View nativeID="intel-status-metric-divider" style={s.divider} />
          </>
        ) : null}
        {job}
        {!hideConnectionIndicator ? (
          <Text
            nativeID="intel-status-api"
            accessibilityLabel={connected ? "API connected" : "API offline"}
            style={[
              s.api,
              { fontSize: navSize },
              { color: connected ? c.success : c.danger },
            ]}
          >
            ● {connected ? "API_OK" : "OFFLINE"}
          </Text>
        ) : null}
        {wide ? (
          <Text nativeID="intel-status-clock" style={s.clock}>
            {clock || "—"}
          </Text>
        ) : null}
      </View>
    </View>
  );
}

const rank: Record<JobState, number> = {
  failed: 5,
  stale: 4,
  running: 3,
  queued: 2,
  succeeded: 1,
  idle: 0,
};

export function JobDropUp({ jobs }: { jobs: StatusJob[] }) {
  const [open, setOpen] = useState(false);
  const navSize = useWindowDimensions().width >= 600 ? 18 : 13;
  const lead = jobs.reduce<StatusJob | undefined>((best, job) => {
    if (!best || rank[job.state] > rank[best.state]) return job;
    return best;
  }, undefined);
  const state = lead?.state ?? "idle";
  const tone = intelStatus[state].color;
  return (
    <View style={s.jobs}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Background jobs"
        accessibilityState={{ expanded: open }}
        aria-expanded={open}
        onPress={() => setOpen((value) => !value)}
        style={s.jobTrigger}
      >
        <View
          style={[
            s.jobDot,
            {
              backgroundColor: tone,
              boxShadow: "none",
            },
          ]}
        />
        <Text style={[s.navText, { color: tone, fontSize: navSize }]}>
          Jobs
        </Text>
      </Pressable>
      {open ? (
        <View
          nativeID="intel-job-menu"
          accessibilityRole="menu"
          style={s.jobMenu}
        >
          <Text style={s.jobHeading}>Background</Text>
          {jobs.length === 0 ? (
            <Text style={s.jobDetail}>No background work</Text>
          ) : (
            jobs.map((job) => (
              <View key={job.id} style={s.jobRow}>
                <View
                  style={[
                    s.jobDot,
                    { backgroundColor: intelStatus[job.state].color },
                  ]}
                />
                <View style={{ flex: 1, minWidth: 0 }}>
                  <Text style={s.jobLabel}>{job.label}</Text>
                  {job.detail ? (
                    <Text style={s.jobDetail}>{job.detail}</Text>
                  ) : null}
                </View>
              </View>
            ))
          )}
        </View>
      ) : null}
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
      <Text style={s.arrow}>End</Text>
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
    overflow: "visible",
    zIndex: 8,
  },
  jobs: { position: "relative", zIndex: 12, overflow: "visible" },
  jobTrigger: {
    minHeight: 44,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  jobDot: { width: 8, height: 8, borderRadius: 4 },
  jobMenu: {
    position: "absolute",
    bottom: 48,
    right: 0,
    width: 260,
    maxWidth: 280,
    zIndex: 30,
    gap: 8,
    padding: 12,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.panelRaised,
  },
  jobHeading: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.muted,
  },
  jobRow: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  jobLabel: {
    fontFamily: intelFonts.mono,
    fontSize: 14,
    color: c.text,
  },
  jobDetail: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.dim,
  },
  cluster: { flexDirection: "row", alignItems: "center", gap: 16, minWidth: 0 },
  navCluster: { height: "100%", flexShrink: 1 },
  home: {
    flexShrink: 1,
    minWidth: 40,
    minHeight: 44,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  navText: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.dim,
  },
  serverName: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.dim,
    flexShrink: 1,
    maxWidth: 120,
  },
  chipBadge: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    fontWeight: "700",
    color: c.accent,
    backgroundColor: c.inset,
    borderWidth: 1,
    borderColor: c.borderStrong,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
    overflow: "hidden",
    flexShrink: 0,
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
    fontSize: 17,
    fontWeight: "700",
    color: c.accent,
  },
  appTextWide: { fontSize: 21 },
  telemetry: {
    fontFamily: intelFonts.mono,
    fontSize: 18,
    color: c.dim,
    fontVariant: ["tabular-nums"],
  },
  api: { fontFamily: intelFonts.mono, fontSize: 13, fontWeight: "600" },
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
    minWidth: 56,
    height: 40,
    paddingHorizontal: 12,
    borderRadius: 8,
    backgroundColor: c.fab,
    borderWidth: 1,
    borderColor: c.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  arrow: { color: c.text, fontFamily: intelFonts.mono, fontSize: 14 },
});
