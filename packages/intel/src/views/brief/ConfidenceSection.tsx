"use client";
import React from "react";
import { View, Text } from "react-native";
import type { Brief } from "@argos/api-client";
import { BriefSection } from "./BriefSection";
import { gradeArt } from "./gradeArt";
import { briefStyles as s } from "./briefStyles";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

export function ConfidenceSection({
  brief,
  claimCount,
  inferenceCount,
}: {
  brief?: Brief;
  pending: boolean;
  claimCount?: number;
  inferenceCount?: number;
}) {
  const confidence = brief?.confidence;
  const grade = /^[A-F][1-6]$/.test(brief?.confidence_grade || "")
    ? brief!.confidence_grade!
    : "F6";
  return (
    <BriefSection title="ARTICLE CONFIDENCE" color={c.accent}>
      <View
        style={{
          flexDirection: "row",
          flexWrap: "wrap",
          alignItems: "center",
          gap: 20,
        }}
      >
        <Text
          accessibilityLabel={`Admiralty grade ${grade}`}
          style={{
            fontFamily: intelFonts.mono,
            fontSize: 12,
            lineHeight: 14,
            color: c.accent,
          }}
        >
          {gradeArt(grade)}
        </Text>
        <View style={{ gap: 8, flexShrink: 1, minWidth: 0 }}>
          <Text style={s.evidenceText}>
            Source {grade[0]} · Information {grade[1]}
          </Text>
          {brief ? (
            <Text style={s.evidenceText}>
              {claimCount ?? confidence?.claims ?? 0} claims ·{" "}
              {inferenceCount ?? confidence?.inferences ?? 0} inferences
            </Text>
          ) : null}
          {brief?.confidence_reason ? (
            <Text style={s.evidenceText}>{brief.confidence_reason}</Text>
          ) : null}
        </View>
      </View>
    </BriefSection>
  );
}
