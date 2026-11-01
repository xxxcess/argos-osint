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
function claimMarker(index: number): string {
  let value = index + 1;
  let label = "";
  while (value > 0) {
    value--;
    label = String.fromCharCode(65 + (value % 26)) + label;
    value = Math.floor(value / 26);
  }
  return `[${label}]`;
}
export function ClaimList({
  claims,
  pending,
  brief,
  extracted = null,
  onTopology,
}: {
  claims: Brief["claims"];
  pending: boolean;
  brief?: Brief;
  extracted?: Seed | null;
  onTopology: (seed: Seed) => void;
}) {
  const evidenceState = (
    empty: string,
    loading = "Loading extracted evidence…",
  ) =>
    pending ? (
      <BriefSkeleton label={loading} />
    ) : (
      <EmptyState title={brief ? empty : "Evidence unavailable"} />
    );
  return (
    <>
      <BriefSection
        title="RETAINED CLAIMS"
        color={kinds.claim}
        icon="claim"
        count={claims.length}
      >
        <View style={s.evidenceBox}>
          {claims.length
            ? claims.map((claim, index) => (
                <View key={claim.id} style={s.section}>
                  <View style={s.evidenceRow}>
                    <Text style={[s.marker, { color: kinds.claim }]}>
                      {claimMarker(index)}
                    </Text>
                    <Text style={s.evidenceText}>{claim.text}</Text>
                  </View>
                  <Text style={s.micro}>
                    {claim.admiralty &&
                    claim.admiralty.toLowerCase() !== "unassessed"
                      ? `${claim.admiralty} · `
                      : ""}
                    {claim.classification}
                  </Text>
                  <BriefAction
                    label={`Explore claim: ${claim.text}`}
                    disabled={!topologyTarget(claim.topology_seed, extracted)}
                    onPress={() => {
                      const seed = topologyTarget(
                        claim.topology_seed,
                        extracted,
                      );
                      if (seed) onTopology(seed);
                    }}
                  >
                    {topologyTarget(claim.topology_seed, extracted)
                      ? "Explore claim"
                      : "Awaiting Brain indexing"}
                  </BriefAction>
                </View>
              ))
            : evidenceState("No published claims", "Loading retained claims…")}
        </View>
      </BriefSection>
    </>
  );
}
