"use client";
import React from "react";
import { View, Text } from "react-native";
import type { Brief } from "@argos/api-client";
import { BriefSection } from "./BriefSection";
import { gradeArt } from "./gradeArt";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

export function ConfidenceSection({
  brief,
  pending,
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
        <View style={{ gap: 8, flexShrink: 1 }}>
          <Text
            style={{
              color: c.text,
              fontFamily: intelFonts.mono,
              fontSize: 16,
              fontWeight: "700",
            }}
          >
            Brief rating{" "}
            {confidence?.score != null ? `${confidence.score}/100` : "N/A"}
          </Text>
          <Text
            style={{ color: c.dim, fontFamily: intelFonts.mono, fontSize: 13 }}
          >
            {pending
              ? "Assessment pending"
              : confidence?.score == null
                ? "Assessment unavailable"
                : brief?.confidence_basis === "publisher_prior"
                  ? "Provisional · publisher prior only"
                  : confidence.provisional
                    ? "Provisional assessment"
                    : "Assessed"}
          </Text>
          <Text
            style={{ color: c.dim, fontFamily: intelFonts.mono, fontSize: 13 }}
          >
            Source {grade[0]} · Information {grade[1]}
          </Text>
          {brief ? (
            <Text
              style={{
                color: c.dim,
                fontFamily: intelFonts.mono,
                fontSize: 13,
              }}
            >
              {claimCount ?? confidence?.claims ?? 0} claims ·{" "}
              {inferenceCount ?? confidence?.inferences ?? 0} inferences
            </Text>
          ) : null}
          {brief?.confidence_reason ? (
            <Text
              style={{
                color: c.dim,
                fontFamily: intelFonts.body,
                fontSize: 13,
              }}
            >
              {brief.confidence_reason}
            </Text>
          ) : null}
          <Text
            style={{
              color: c.muted,
              fontFamily: intelFonts.body,
              fontSize: 13,
            }}
          >
            Evidence assessment · not a probability of truth
          </Text>
        </View>
      </View>
    </BriefSection>
  );
}
