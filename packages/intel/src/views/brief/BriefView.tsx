"use client";
import React from "react";
import { View, Text, useWindowDimensions } from "react-native";
import type { Article, Brief, Seed, BriefStage } from "@argos/api-client";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { briefStyles as s } from "./briefStyles";
import { BriefSection } from "./BriefSection";
export { BriefSection } from "./BriefSection";
export { RelatedBriefCard } from "./RelatedBriefCard";
import { ActorChips } from "./ActorChips";
import { ClaimList } from "./ClaimList";
import { InferenceList } from "./InferenceList";
import {
  BriefAction,
  extractedBriefSeed,
  extractedFocus,
  topologyTarget,
} from "./BriefAction";
import { BriefHeader } from "./BriefHeader";
import type { JobState } from "../../shell/jobState";

// @tag:intel-brief — canonical Brain identities gate read-only navigation.
function lines(value: unknown): string[] {
  if (Array.isArray(value)) return value.flatMap(lines);
  if (typeof value === "string") return value ? [value] : [];
  if (!value || typeof value !== "object") return [];
  const object = value as Record<string, unknown>;
  const own = [
    "text",
    "claim",
    "inference_text",
    "why",
    "verification_needed",
    "interpretation",
    "empty_reason",
  ].flatMap((key) =>
    typeof object[key] === "string" && object[key] ? [String(object[key])] : [],
  );
  return own.length
    ? own
    : Object.entries(object)
        .filter(([key]) =>
          ["claims", "inferences", "leads", "links"].includes(key),
        )
        .flatMap(([, nested]) => lines(nested));
}
export function BriefView({
  article,
  brief,
  pending,
  stages,
  onBack,
  onTopology,
  related,
  operations,
  notices,
  source,
  jobState = "idle",
}: {
  article: Article;
  brief?: Brief;
  pending: boolean;
  stages?: BriefStage[];
  onBack: () => void;
  onTopology: (seed: Seed, focus?: string) => void;
  related: React.ReactNode;
  operations: React.ReactNode;
  notices: React.ReactNode;
  source: React.ReactNode;
  jobState?: JobState;
}) {
  const { width } = useWindowDimensions();
  const narrow = width < 900;
  const shown = brief?.article || article;
  const loading = pending && !brief;
  const actors = (brief?.actors || []).filter(
    (actor, index, all) =>
      all.findIndex((other) => other.id === actor.id) === index,
  );
  const claims =
    brief?.claims.filter((claim) => claim.classification !== "inference") || [];
  const inferenceText = lines(
    brief?.output?.inferences ??
      brief?.publication?.[2] ??
      brief?.claims
        .filter((claim) => claim.classification === "inference")
        .map((claim) => ({ text: claim.text })),
  );
  const inferences =
    brief?.inferences ??
    inferenceText.map((text, index) => {
      const retained = brief?.claims.find(
        (claim) => claim.classification === "inference" && claim.text === text,
      );
      return {
        id: retained?.id || `display:${index}`,
        text,
        topology_seed: retained?.topology_seed,
      };
    });
  const extracted = extractedBriefSeed(article);
  const launch = topologyTarget(brief?.topology_seed, extracted);
  const sourceName =
    shown.source_name || shown.source_domain || "Source unavailable";
  return (
    <View nativeID="intel-brief" style={s.shell}>
      <BriefHeader
        article={shown}
        brief={brief}
        jobState={jobState}
        onBack={onBack}
      />
      <View
        style={{
          flexDirection: narrow ? "column" : "row",
          alignItems: "flex-start",
          minWidth: 0,
        }}
      >
        <View
          nativeID="intel-brief-reader"
          style={[
            s.body,
            { flex: 2 },
            narrow && { width: "100%" },
            narrow && s.narrowPadding,
          ]}
        >
          {notices}
          <Text style={[s.lede, narrow && { fontSize: 16, lineHeight: 23 }]}>
            {shown.description || "No description available."}
          </Text>
          <ActorChips
            actors={actors}
            pending={loading || !!brief?.actors_pending}
            brief={brief}
            extracted={extracted}
            onTopology={onTopology}
          />
          <ClaimList
            stage={stages?.find((s) => s.operation === "intel_brief_claims")}
            claims={claims}
            pending={loading}
            brief={brief}
            extracted={extracted}
            onTopology={onTopology}
          />
          <InferenceList
            stage={stages?.find(
              (s) => s.operation === "intel_brief_inferences",
            )}
            inferences={inferences}
            pending={loading}
            brief={brief}
            extracted={extracted}
            onTopology={onTopology}
          />
        </View>
        <View
          nativeID="intel-context-rail"
          style={{
            flex: 1,
            minWidth: 0,
            width: narrow ? "100%" : undefined,
            padding: 20,
            gap: 20,
          }}
        >
          {operations}
          {source}
          <BriefSection title="CORRELATED INTELLIGENCE REPORTS">
            <Text style={s.micro}>Related leads · not corroboration</Text>
            {related}
          </BriefSection>
        </View>
      </View>
      <View style={[s.foot, narrow && s.narrowPadding]}>
        <Text style={s.micro}>
          Source · {sourceName}
          {shown.author ? ` · ${shown.author}` : ""}
        </Text>
        <Text style={s.micro}>Actors</Text>
        {actors.length ? (
          actors.map((actor) => (
            <BriefAction
              key={actor.id}
              label={`Extracted actor ${actor.display_name}`}
              disabled={!topologyTarget(actor.topology_seed, extracted)}
              onPress={() => {
                const seed = topologyTarget(actor.topology_seed, extracted);
                const focus = seed
                  ? extractedFocus(seed, extracted, `actor:${actor.id}`)
                  : undefined;
                if (seed && focus) onTopology(seed, focus);
                else if (seed) onTopology(seed);
              }}
            >
              <Text style={{ color: kinds.actor }}>● </Text>
              {actor.display_name}
              {topologyTarget(actor.topology_seed, extracted)
                ? " · Topology"
                : ""}
            </BriefAction>
          ))
        ) : !brief?.actors_pending && (brief || !loading) ? (
          <Text style={s.micro}>
            {brief ? "No actors" : "Actors unavailable"}
          </Text>
        ) : null}
      </View>
      <View style={[s.lower, narrow && s.narrowPadding]}>
        <BriefAction
          variant="cta"
          label="Launch 3D topology"
          disabled={!launch}
          onPress={() => {
            if (launch) onTopology(launch);
          }}
        >
          {launch ? "Topology" : "Indexing"}
        </BriefAction>
      </View>
    </View>
  );
}
