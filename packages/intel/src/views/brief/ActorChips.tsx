"use client";
import React from "react";
import { View, Text, Pressable } from "react-native";
import type { Brief } from "@argos/api-client";
import { EmptyState, LoadingState } from "@argos/ui";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { BriefSection } from "./BriefSection";
import { BriefSkeleton } from "./BriefSkeleton";
import { briefStyles as s } from "./briefStyles";
export function ActorChips({
  actors,
  pending,
  brief,
  onActor,
}: {
  actors: Brief["actors"];
  pending: boolean;
  brief?: Brief;
  onActor?: (actor: Brief["actors"][number]) => void;
}) {
  const evidenceState = (
    empty: string,
    loading = "Loading extracted evidence…",
  ) =>
    pending && (!brief || brief.actors_pending || brief.actors_body_pending) ? (
      <BriefSkeleton
        variant="actors"
        label={
          brief?.actors_body_pending
            ? "Checking cleaned article for actors…"
            : brief?.actors_pending
              ? "Matching catalog actors…"
              : loading
        }
      />
    ) : (
      <EmptyState
        title={
          brief?.actors_body_unavailable
            ? "Actor detection incomplete"
            : brief
              ? empty
              : "Evidence unavailable"
        }
      >
        {brief?.actors_body_unavailable
          ? "The cleaned article is unavailable. Actors could not be checked in its body."
          : null}
      </EmptyState>
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
                  disabled={!onActor}
                  accessibilityState={{
                    disabled: !onActor,
                  }}
                  onPress={() => onActor?.(actor)}
                  style={s.evidenceRow}
                >
                  <Text style={[s.marker, { color: kinds.actor }]}>
                    [{index + 1}]
                  </Text>
                  <Text style={s.evidenceText}>
                    {actor.display_name}
                    {actor.type_hint ? ` · ${actor.type_hint}` : ""}
                    {" · Dossier"}
                  </Text>
                </Pressable>
              ))
            : evidenceState("No actors extracted")}
          {actors.length > 0 && brief?.actors_body_pending ? (
            <LoadingState
              compact
              label="Checking cleaned article for more actors…"
            />
          ) : null}
        </View>
      </BriefSection>
    </>
  );
}
