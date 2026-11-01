"use client";
import React from "react";
import { View, Text } from "react-native";
import type { Brief, Seed } from "@argos/api-client";
import { EmptyState } from "@argos/ui";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { BriefSection } from "./BriefSection";
import { BriefSkeleton } from "./BriefSkeleton";
import { BriefAction, topologyTarget } from "./BriefAction";
import { briefStyles as s } from "./briefStyles";
export function InferenceList({
  inferences,
  pending,
  brief,
  extracted = null,
  onTopology,
}: {
  inferences: NonNullable<Brief["inferences"]>;
  pending: boolean;
  brief?: Brief;
  extracted?: Seed | null;
  onTopology: (seed: Seed) => void;
}) {
  return (
    <>
      <View testID="reader-inferences">
        <BriefSection
          title="ANALYTICAL INFERENCES"
          color={kinds.inference}
          icon="inference"
          count={inferences.length}
        >
          <View style={s.evidenceBox}>
            {inferences.length ? (
              inferences.map((item, index) => (
                <View key={item.id} style={{ gap: 12 }}>
                  <View style={s.evidenceRow}>
                    <Text style={[s.marker, { color: kinds.inference }]}>
                      [{index + 1}]
                    </Text>
                    <Text style={s.evidenceText}>{item.text}</Text>
                  </View>
                  <BriefAction
                    label={`Explore inference: ${item.text}`}
                    disabled={!topologyTarget(item.topology_seed, extracted)}
                    onPress={() => {
                      const seed = topologyTarget(
                        item.topology_seed,
                        extracted,
                      );
                      if (seed) onTopology(seed);
                    }}
                  >
                    {topologyTarget(item.topology_seed, extracted)
                      ? "Explore inference"
                      : "Awaiting Brain indexing"}
                  </BriefAction>
                </View>
              ))
            ) : pending ? (
              <BriefSkeleton label="Loading inferences…" />
            ) : (
              <EmptyState
                title={brief ? "No retained output" : "Inferences unavailable"}
              />
            )}
          </View>
        </BriefSection>
      </View>
    </>
  );
}
