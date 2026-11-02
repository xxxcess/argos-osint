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
  stage,
  brief,
  extracted = null,
  onTopology,
}: {
  claims: Brief["claims"];
  pending: boolean;
  stage?: BriefStage;
  brief?: Brief;
  extracted?: Seed | null;
  onTopology: (seed: Seed, focus?: string) => void;
}) {
  const evidenceState = (
    empty: string,
    loading = "Loading extracted evidence…",
  ) =>
    pending && !brief ? (
      <BriefSkeleton variant="claims" label={loading} />
    ) : (
      <EmptyState title={brief ? empty : "Evidence unavailable"} />
    );
  return (
    <>
      <BriefSection
        title="RETAINED CLAIMS"
        color={kinds.claim}
        icon="claim"
        count={stage && stage.state !== "completed" ? undefined : claims.length}
      >
        <View style={s.evidenceBox}>
          <StageContent stage={stage} kind="claims">
            {claims.length
              ? claims.map((claim, index) => {
                  const text = displayEvidence(claim.text);
                  return (
                    <Pressable
                      key={claim.id}
                      style={s.section}
                      accessibilityRole="button"
                      accessibilityLabel={`Explore claim: ${text}`}
                      disabled={!topologyTarget(claim.topology_seed, extracted)}
                      onPress={() => {
                      const seed = topologyTarget(
                        claim.topology_seed,
                        extracted,
                      );
                      const focus = seed
                        ? extractedFocus(seed, extracted, `claim:${claim.id}`)
                        : undefined;
                      if (seed && focus) onTopology(seed, focus);
                      else if (seed) onTopology(seed);
                      }}
                    >
                      <View style={s.evidenceRow}>
                        <Text style={[s.marker, { color: kinds.claim }]}>
                          {claimMarker(index)}
                        </Text>
                        <Text style={s.evidenceText}>{text}</Text>
                      </View>
                      {claim.admiralty &&
                      claim.admiralty.toLowerCase() !== "unassessed" ? (
                        <Text style={s.micro}>{claim.admiralty}</Text>
                      ) : null}
                    </Pressable>
                  );
                })
              : evidenceState(
                  "No published claims",
                  "Loading retained claims…",
                )}
          </StageContent>
        </View>
      </BriefSection>
    </>
  );
}
