import React from "react";
import { View, Text, useWindowDimensions } from "react-native";
import type { Article, Brief } from "@argos/api-client";
import type { JobState } from "../../shell/jobState";
import { JobStatusBeacon } from "../../shell/JobStatusBeacon";
import { intelColors as c } from "../../shell/intel-theme";
import { BriefAction } from "./BriefAction";
import { briefStyles as s } from "./briefStyles";
export function BriefHeader({
  article,
  brief,
  jobState,
  onBack,
}: {
  article: Article;
  brief?: Brief;
  jobState: JobState;
  onBack: () => void;
}) {
  const narrow = useWindowDimensions().width < 900;
  const candidate = brief?.confidence?.score;
  const score =
    candidate != null && Number.isFinite(candidate) ? candidate : null;
  const grades = [
    ...new Set(
      (brief?.claims ?? [])
        .map((claim) => claim.admiralty.trim())
        .filter((grade) => grade.length > 0 && grade.toLowerCase() !== "unassessed"),
    ),
  ];
  const date = article.published_at.slice(0, 10) || "Date unavailable";
  const confidenceColor =
    score == null
      ? c.muted
      : score >= 70
        ? c.accent
        : score >= 40
          ? c.warning
          : c.danger;
  return (
    <View style={s.header}>
      <View style={[s.toolbar, narrow && { alignItems: "flex-start" }]}>
        <BriefAction variant="back" label="Return to Bulletin" onPress={onBack}>
          ‹ Bulletin
        </BriefAction>
        <View style={s.metaLine}>
          <Text style={s.headerMeta}>{date}</Text>
          {grades.map((grade) => (
            <Text key={grade} style={s.headerMeta}>
              {grade}
            </Text>
          ))}
          {score != null ? (
            <Text style={[s.headerMeta, { color: confidenceColor }]}>
              {score}%
            </Text>
          ) : grades.length === 0 ? (
            <Text style={s.headerMeta}>Not assessed</Text>
          ) : null}
          <JobStatusBeacon state={jobState} />
        </View>
      </View>
      <Text
        nativeID="intel-brief-title"
        accessibilityRole="header"
        numberOfLines={2}
        style={[s.title, narrow && { fontSize: 19, lineHeight: 25 }]}
      >
        {article.title}
      </Text>
    </View>
  );
}
