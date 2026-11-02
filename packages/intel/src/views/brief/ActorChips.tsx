"use client";
import React from "react";
import { View, Text, Pressable } from "react-native";
import type { Brief, Seed } from "@argos/api-client";
import { EmptyState } from "@argos/ui";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { BriefSection } from "./BriefSection";
import { BriefSkeleton } from "./BriefSkeleton";
import { extractedFocus, topologyTarget } from "./BriefAction";
import { briefStyles as s } from "./briefStyles";
export function ActorChips({
  actors,
  pending,
  brief,
  extracted = null,
  onTopology,
}: {
  actors: Brief["actors"];
  pending: boolean;
  brief?: Brief;
  extracted?: Seed | null;
  onTopology: (seed: Seed, focus?: string) => void;
}) {
  const evidenceState = (
    empty: string,
    loading = "Loading extracted evidence…",
  ) =>
    pending && (!brief || brief.actors_pending) ? (
      <BriefSkeleton
        variant="actors"
        label={brief?.actors_pending ? "Matching catalog actors…" : loading}
      />
    ) : (
      <EmptyState title={brief ? empty : "Evidence unavailable"} />
    );
  return (
    <>
      <BriefSection
        title="IDENTIFIED ACTORS"
        color={kinds.actor}
        icon="actor"
        count={pending && actors.length === 0 ? undefined : actors.length}
      >
        <View style={s.evidenceBox}>
          {actors.length
            ? actors.map((actor, index) => (
                <Pressable
                  key={actor.id}
                  accessibilityRole="button"
                  accessibilityLabel={`Explore actor ${actor.display_name}`}
                  disabled={!topologyTarget(actor.topology_seed, extracted)}
                  accessibilityState={{
                    disabled: !topologyTarget(actor.topology_seed, extracted),
                  }}
                  onPress={() => {
                    const seed = topologyTarget(actor.topology_seed, extracted);
                    const focus = seed
                      ? extractedFocus(seed, extracted, `actor:${actor.id}`)
                      : undefined;
                    if (seed && focus) onTopology(seed, focus);
                    else if (seed) onTopology(seed);
                  }}
                  style={s.evidenceRow}
                >
                  <Text style={[s.marker, { color: kinds.actor }]}>
                    [{index + 1}]
                  </Text>
                  <Text style={s.evidenceText}>
                    {actor.display_name}
                    {actor.type_hint ? ` · ${actor.type_hint}` : ""}
                    {topologyTarget(actor.topology_seed, extracted)
                      ? " · Topology"
                      : ""}
                  </Text>
                </Pressable>
              ))
            : evidenceState("No actors extracted")}
        </View>
      </BriefSection>
    </>
  );
}
