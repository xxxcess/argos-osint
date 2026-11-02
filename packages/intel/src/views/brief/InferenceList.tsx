"use client";
import React from "react";
import { View, Text, Pressable } from "react-native";
import type { Brief, Seed, BriefStage } from "@argos/api-client";
import { EmptyState } from "@argos/ui";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { BriefSection } from "./BriefSection";
import { BriefSkeleton } from "./BriefSkeleton";
import { extractedFocus, topologyTarget } from "./BriefAction";
import { StageContent } from "./StageContent";
import { briefStyles as s } from "./briefStyles";
import { displayEvidence } from "./displayEvidence";
export function InferenceList({
  inferences,
  pending,
  stage,
  brief,
  extracted = null,
  onTopology,
}: {
  inferences: NonNullable<Brief["inferences"]>;
  pending: boolean;
  stage?: BriefStage;
  brief?: Brief;
  extracted?: Seed | null;
  onTopology: (seed: Seed, focus?: string) => void;
}) {
  return (
    <>
      <View testID="reader-inferences">
        <BriefSection
          title="ANALYTICAL INFERENCES"
          color={kinds.inference}
          icon="inference"
          count={
            stage && stage.state !== "completed" ? undefined : inferences.length
          }
        >
          <View style={s.evidenceBox}>
            <StageContent stage={stage} kind="inferences">
              {inferences.length ? (
                inferences.map((item, index) => {
                  const text = displayEvidence(item.text);
                  return (
                    <Pressable
                      key={item.id}
                      style={{ gap: 12 }}
                      accessibilityRole="button"
                      accessibilityLabel={`Explore inference: ${text}`}
                      disabled={!topologyTarget(item.topology_seed, extracted)}
                      onPress={() => {
                      const seed = topologyTarget(
                        item.topology_seed,
                        extracted,
                      );
                      const focus = seed
                        ? extractedFocus(
                            seed,
                            extracted,
                            `inference:${item.id}`,
                          )
                        : undefined;
                      if (seed && focus) onTopology(seed, focus);
                      else if (seed) onTopology(seed);
                      }}
                    >
                      <View style={s.evidenceRow}>
                        <Text style={[s.marker, { color: kinds.inference }]}>
                          [{index + 1}]
                        </Text>
                        <Text style={s.evidenceText}>{text}</Text>
                      </View>
                    </Pressable>
                  );
                })
              ) : pending && !brief ? (
                <BriefSkeleton
                  variant="inferences"
                  label="Loading inferences…"
                />
              ) : (
                <EmptyState
                  title={
                    brief ? "No retained output" : "Inferences unavailable"
                  }
                />
              )}
            </StageContent>
          </View>
        </BriefSection>
      </View>
    </>
  );
}
