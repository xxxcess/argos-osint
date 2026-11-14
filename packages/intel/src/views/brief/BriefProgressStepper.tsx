"use client";
import React from "react";
import { View, Text, StyleSheet } from "react-native";
import type { BriefStage } from "@argos/api-client";
import type { JobState } from "../../shell/jobState";
import {
  intelColors as c,
  intelSpace,
  intelRadius,
} from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

type StepState = "complete" | "active" | "failed" | "pending";
function stateOf(stage: BriefStage): StepState {
  if (/^(failed|error|blocked)$/i.test(stage.state)) return "failed";
  if (/^(completed|succeeded|done)$/i.test(stage.state)) return "complete";
  if (stage.active || /^(running|extracting|processing)$/i.test(stage.state))
    return "active";
  return "pending";
}
const headings: Record<JobState, string> = {
  idle: "Brief idle",
  queued: "Brief queued",
  running: "Brief in progress",
  succeeded: "Brief complete",
  failed: "Brief extraction failed",
  stale: "Brief updates interrupted",
};
/** Live stage receipts determine each step independently, including parallel work. */
export function BriefProgressStepper({
  stages,
  state,
  connected = true,
}: {
  stages: BriefStage[];
  state: JobState;
  connected?: boolean;
}) {
  const attention = stages.filter((stage) =>
    ["active", "failed"].includes(stateOf(stage)),
  );
  return (
    <View
      role="status"
      aria-live="polite"
      accessibilityLabel="Brief progress"
      style={s.panel}
    >
      <Text
        style={[s.heading, { color: state === "failed" ? c.danger : c.text }]}
      >
        {headings[state]}
      </Text>
      <View
        role="list"
        accessibilityLabel="Brief progress steps"
        style={s.steps}
      >
        {stages.map((stage, index) => {
          const status = stateOf(stage);
          const color =
            status === "failed"
              ? c.danger
              : status === "complete"
                ? c.success
                : status === "active"
                  ? c.cool
                  : c.muted;
          const label =
            status === "active" && !connected
              ? "Updates interrupted"
              : status === "complete"
                ? "Complete"
                : status === "pending"
                  ? stage.state.replaceAll("_", " ") || "Pending"
                  : status === "failed"
                    ? "Failed"
                    : "In progress";
          return (
            <View
              role="listitem"
              key={stage.operation}
              accessibilityLabel={`${stage.label}: ${label}`}
              style={s.step}
            >
              <View style={s.track} aria-hidden>
                <View
                  style={[
                    s.connector,
                    {
                      backgroundColor:
                        index === 0
                          ? "transparent"
                          : status === "complete"
                            ? c.success
                            : c.border,
                    },
                  ]}
                />
                <View
                  style={[
                    s.circle,
                    {
                      borderColor: color,
                      backgroundColor:
                        status === "complete" ? c.success : c.panelRaised,
                    },
                  ]}
                >
                  <Text
                    style={[
                      s.number,
                      { color: status === "complete" ? c.panel : color },
                    ]}
                  >
                    {status === "complete"
                      ? "✓"
                      : status === "failed"
                        ? "!"
                        : index + 1}
                  </Text>
                </View>
                <View
                  style={[
                    s.connector,
                    {
                      backgroundColor:
                        index === stages.length - 1
                          ? "transparent"
                          : status === "complete"
                            ? c.success
                            : c.border,
                    },
                  ]}
                />
              </View>
              <Text style={[s.label, { color }]}>{stage.label}</Text>
              <Text style={s.status}>{label}</Text>
            </View>
          );
        })}
      </View>
      {attention.map((stage) => (
        <Text
          key={stage.operation}
          style={[s.detail, stateOf(stage) === "failed" && { color: c.danger }]}
        >
          {stage.label} · {stage.detail || stage.row || stage.state}
        </Text>
      ))}
      {!connected && state !== "succeeded" ? (
        <Text style={[s.detail, { color: c.warning }]}>
          Live updates interrupted. Reconnecting…
        </Text>
      ) : null}
    </View>
  );
}
const s = StyleSheet.create({
  panel: {
    padding: intelSpace.card,
    gap: intelSpace.gutter,
    minWidth: 0,
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: intelRadius.control,
    backgroundColor: c.panelRaised,
  },
  heading: { fontFamily: intelFonts.mono, fontSize: 13, fontWeight: "700" },
  steps: { flexDirection: "row", width: "100%", minWidth: 0 },
  step: { flex: 1, minWidth: 0, gap: intelSpace.small, alignItems: "stretch" },
  track: { flexDirection: "row", alignItems: "center", height: 28 },
  connector: { flex: 1, height: 1 },
  circle: {
    width: 26,
    height: 26,
    borderRadius: 13,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  number: { fontFamily: intelFonts.mono, fontSize: 13, fontWeight: "700" },
  label: {
    textAlign: "center",
    fontFamily: intelFonts.mono,
    fontSize: 13,
    flexShrink: 1,
    paddingHorizontal: 2,
  },
  status: {
    textAlign: "center",
    fontFamily: intelFonts.mono,
    fontSize: 12,
    color: c.dim,
    flexShrink: 1,
    paddingHorizontal: 2,
  },
  detail: {
    fontFamily: intelFonts.body,
    fontSize: 13,
    lineHeight: 19,
    color: c.dim,
  },
});
