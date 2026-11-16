"use client";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { ActorHitView } from "@argos/api-client";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

/** One discovery hit. It opens the dossier by canonical id, which is shown when another hit shares the name. */
export function ActorHitCard({
  view,
  onPress,
}: {
  view: ActorHitView;
  onPress: () => void;
}) {
  const [active, setActive] = useState(false);
  const { hit } = view;
  return (
    <Pressable
      testID={`actor-hit-${hit.actor_id}`}
      accessibilityRole="link"
      accessibilityLabel={`Open actor dossier: ${hit.label}${view.id ? ` (${view.id})` : ""}`}
      onPress={onPress}
      onFocus={() => setActive(true)}
      onBlur={() => setActive(false)}
      onHoverIn={() => setActive(true)}
      onHoverOut={() => setActive(false)}
      style={({ pressed }) => [
        s.card,
        (active || pressed) && {
          borderColor: c.accent,
          backgroundColor: c.panelRaised,
        },
      ]}
    >
      <View style={s.content}>
        <Text numberOfLines={2} style={s.title}>
          {hit.label}
        </Text>
        {view.detail ? <Text style={s.meta}>{view.detail}</Text> : null}
        {hit.summary ? (
          <Text numberOfLines={2} style={s.summary}>
            {hit.summary}
          </Text>
        ) : null}
        {view.match ? <Text style={s.meta}>{view.match}</Text> : null}
        {view.id ? (
          <Text numberOfLines={1} style={s.meta}>
            ID {view.id}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
}

const s = StyleSheet.create({
  card: {
    padding: 12,
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: intelRadius.card,
    backgroundColor: c.panel,
    minWidth: 0,
  },
  content: { gap: 4, minWidth: 0 },
  title: {
    fontFamily: intelFonts.body,
    fontSize: 17,
    lineHeight: 23,
    fontWeight: "600",
    color: c.text,
  },
  summary: {
    fontFamily: intelFonts.body,
    fontSize: 15,
    lineHeight: 21,
    color: c.dim,
  },
  meta: {
    fontFamily: intelFonts.mono,
    fontSize: 12,
    lineHeight: 18,
    color: c.dim,
  },
});
