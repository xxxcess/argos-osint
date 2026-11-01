"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import type { Article, Brief, Seed } from "@argos/api-client";
import { EmptyState, LoadingState } from "@argos/ui";
import {
  intelColors as c,
  intelKindColors as kinds,
  intelType,
  intelEffects,
} from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

// @tag:intel-brief — canonical Brain identities gate read-only navigation.
function hasMemorySeed(seed: Seed | null | undefined): seed is Seed {
  return (
    !!seed &&
    [seed.Actor, seed.Claim, seed.Inference].some(
      (value) => typeof value === "string" && !!value,
    )
  );
}
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
export function BriefSection({
  title,
  children,
  color = c.muted,
  icon,
}: {
  title: string;
  children: React.ReactNode;
  color?: string;
  icon?: "actor" | "claim" | "inference" | "source" | "entities";
}) {
  return (
    <View style={s.section}>
      <View style={s.sectionHeading}>
        {icon ? (
          <View
            accessibilityElementsHidden
            importantForAccessibility="no-hide-descendants"
            style={s.icon}
          >
            {icon === "actor" ? (
              <>
                <View
                  style={{
                    width: 7,
                    height: 7,
                    borderRadius: 4,
                    borderWidth: 1.5,
                    borderColor: color,
                  }}
                />
                <View
                  style={{
                    width: 14,
                    height: 8,
                    borderWidth: 1.5,
                    borderColor: color,
                    borderTopLeftRadius: 7,
                    borderTopRightRadius: 7,
                  }}
                />
              </>
            ) : icon === "inference" ? (
              <Text style={{ color, fontSize: 22, lineHeight: 24 }}>△</Text>
            ) : (
              <View
                style={{
                  width: 16,
                  height: 16,
                  borderWidth: 1.5,
                  borderColor: color,
                  borderRadius: icon === "source" ? 5 : 2,
                }}
              />
            )}
          </View>
        ) : null}
        <Text accessibilityRole="header" style={s.sectionTitle}>
          {title}
        </Text>
      </View>
      {children}
    </View>
  );
}
function BriefAction({
  children,
  label,
  onPress,
  variant = "row",
  disabled = false,
}: {
  children: React.ReactNode;
  label?: string;
  onPress: () => void;
  variant?: "row" | "back" | "cta";
  disabled?: boolean;
}) {
  const cta = variant === "cta";
  const back = variant === "back";
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      style={({ pressed }) => [
        s.action,
        disabled && { opacity: 0.45 },
        cta && s.cta,
        back && s.back,
        (hovered || focused) && {
          borderColor: c.accent,
          backgroundColor: back ? "transparent" : c.accentSoft,
        },
        cta && (hovered || focused) && { boxShadow: intelEffects.accentGlow },
        pressed && { backgroundColor: c.panelRaised },
      ]}
    >
      <Text
        style={[
          s.actionText,
          cta && s.ctaText,
          back && { color: hovered || focused ? c.accent : c.dim },
        ]}
      >
        {children}
      </Text>
    </Pressable>
  );
}
export function RelatedBriefCard({
  article,
  basis,
  tentative,
  onPress,
}: {
  article: Article;
  basis: string[];
  tentative: boolean;
  onPress: () => void;
}) {
  const { width } = useWindowDimensions();
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open Focus Brief for related lead: ${article.title}`}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={[
        s.related,
        width >= 900 && { maxWidth: 650 },
        hovered && { borderColor: c.borderStrong },
      ]}
    >
      <View style={s.meta}>
        <Text style={s.micro}>{article.category.toUpperCase()}</Text>
        <Text style={s.micro}>
          {article.published_at.slice(0, 10) || "Date unavailable"}
        </Text>
      </View>
      <Text style={s.relatedTitle}>{article.title}</Text>
      <Text numberOfLines={2} style={s.relatedCopy}>
        {article.description || "No description available."}
      </Text>
      <Text style={s.micro}>
        {basis.join(" · ") || "Basis unavailable"}
        {tentative ? " · tentative" : ""} · Related lead, not corroboration
      </Text>
    </Pressable>
  );
}
export function BriefView({
  article,
  brief,
  pending,
  onBack,
  onTopology,
  related,
  operations,
  notices,
  source,
}: {
  article: Article;
  brief?: Brief;
  pending: boolean;
  onBack: () => void;
  onTopology: (seed: Seed) => void;
  related: React.ReactNode;
  operations: React.ReactNode;
  notices: React.ReactNode;
  source: React.ReactNode;
}) {
  const { width } = useWindowDimensions();
  const narrow = width < 900;
  const shown = brief?.article || article;
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
  const score = brief?.confidence?.score;
  const measured = score != null && Number.isFinite(score);
  const confidenceColor = !measured
    ? c.dim
    : score >= 70
      ? c.accent
      : score >= 40
        ? c.warning
        : c.danger;
  const evidenceState = (
    empty: string,
    loading = "Loading extracted evidence…",
  ) =>
    pending ? (
      <LoadingState label={loading} />
    ) : (
      <EmptyState title={brief ? empty : "Evidence unavailable"} />
    );
  return (
    <View nativeID="intel-brief" style={s.shell}>
      <View style={[s.header, narrow && s.narrowPadding]}>
        <BriefAction variant="back" label="Return to Bulletin" onPress={onBack}>
          ‹ RETURN TO BULLETIN
        </BriefAction>
        <View style={[s.headerColumns, narrow && { flexDirection: "column" }]}>
          <View style={s.titleColumn}>
            <Text style={s.micro}>
              FOCUS BRIEF / {shown.category.toUpperCase()}
            </Text>
            <Text
              accessibilityRole="header"
              style={[s.title, narrow && { fontSize: 30, lineHeight: 38 }]}
            >
              {shown.title}
            </Text>
          </View>
          <View style={[s.reference, narrow && { alignItems: "flex-start" }]}>
            <Text style={s.date}>
              # {shown.published_at.slice(0, 10) || "Date unavailable"}
            </Text>
          </View>
        </View>
      </View>
      <View style={[s.body, narrow && s.narrowPadding]}>
        {notices}
        <Text style={[s.lede, narrow && { fontSize: 20, lineHeight: 32 }]}>
          {shown.description || "No description available."}
        </Text>
        {source}
        <BriefSection
          title="IDENTIFIED ACTORS"
          color={kinds.actor}
          icon="actor"
        >
          <View style={s.evidenceBox}>
            {actors.length
              ? actors.map((actor, index) => (
                  <Pressable
                    key={actor.id}
                    accessibilityRole="button"
                    accessibilityLabel={`Explore actor ${actor.display_name}`}
                    disabled={!hasMemorySeed(actor.topology_seed)}
                    accessibilityState={{
                      disabled: !hasMemorySeed(actor.topology_seed),
                    }}
                    onPress={() => {
                      if (hasMemorySeed(actor.topology_seed))
                        onTopology(actor.topology_seed);
                    }}
                    style={s.evidenceRow}
                  >
                    <Text style={[s.marker, { color: kinds.actor }]}>
                      [{index + 1}]
                    </Text>
                    <Text style={s.evidenceText}>
                      {actor.display_name}
                      {actor.type_hint ? ` · ${actor.type_hint}` : ""}
                      {!hasMemorySeed(actor.topology_seed)
                        ? " · Awaiting Brain indexing"
                        : ""}
                    </Text>
                  </Pressable>
                ))
              : evidenceState("No actors extracted")}
          </View>
        </BriefSection>
        <BriefSection title="RETAINED CLAIMS" color={kinds.claim} icon="claim">
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
                      {claim.confidence == null
                        ? "Unassessed"
                        : `${Math.round(claim.confidence * 100)}%`}{" "}
                      · {claim.admiralty || "Admiralty unavailable"} ·{" "}
                      {claim.classification}
                    </Text>
                    <BriefAction
                      label={`Explore claim: ${claim.text}`}
                      disabled={!hasMemorySeed(claim.topology_seed)}
                      onPress={() => {
                        if (hasMemorySeed(claim.topology_seed))
                          onTopology(claim.topology_seed);
                      }}
                    >
                      {hasMemorySeed(claim.topology_seed)
                        ? "Explore claim"
                        : "Awaiting Brain indexing"}
                    </BriefAction>
                  </View>
                ))
              : evidenceState(
                  "No published claims",
                  "Loading retained claims…",
                )}
          </View>
        </BriefSection>
        <View testID="reader-inferences">
          <BriefSection
            title="ANALYTICAL INFERENCES"
            color={kinds.inference}
            icon="inference"
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
                      disabled={!hasMemorySeed(item.topology_seed)}
                      onPress={() => {
                        if (hasMemorySeed(item.topology_seed))
                          onTopology(item.topology_seed);
                      }}
                    >
                      {hasMemorySeed(item.topology_seed)
                        ? "Explore inference"
                        : "Awaiting Brain indexing"}
                    </BriefAction>
                  </View>
                ))
              ) : pending ? (
                <LoadingState label="Loading inferences…" />
              ) : (
                <EmptyState
                  title={
                    brief ? "No retained output" : "Inferences unavailable"
                  }
                />
              )}
            </View>
          </BriefSection>
        </View>
        <BriefSection title="CORRELATED INTELLIGENCE REPORTS">
          <Text style={s.micro}>Related leads · not corroboration</Text>
          {related}
        </BriefSection>
      </View>
      <View style={[s.lower, narrow && s.narrowPadding]}>
        <BriefSection title="INTELLIGENCE ORIGIN" icon="source">
          <View style={[s.origin, narrow && { flexDirection: "column" }]}>
            <View style={s.field}>
              <Text style={s.micro}>SOURCE</Text>
              <Text style={[s.value, { color: c.cool }]}>
                {shown.source_name ||
                  shown.source_domain ||
                  "Source unavailable"}
              </Text>
              {shown.author ? (
                <Text style={s.micro}>{shown.author}</Text>
              ) : null}
            </View>
            <View style={s.field}>
              <Text style={s.micro}>CONFIDENCE</Text>
              <Text style={[s.value, { color: confidenceColor }]}>
                {measured ? `${score}%` : "Not assessed"}
              </Text>
              {measured ? (
                <Text style={s.micro}>
                  {brief?.confidence?.provisional
                    ? "Provisional evidence score"
                    : "Assessed evidence score"}
                </Text>
              ) : null}
            </View>
          </View>
        </BriefSection>
      </View>
      <View style={[s.lower, narrow && s.narrowPadding]}>
        <BriefSection title="EXTRACTED ENTITIES" icon="entities">
          <Text style={s.micro}>PRIMARY ACTORS</Text>
          {actors.length ? (
            actors.map((actor) => (
              <BriefAction
                key={actor.id}
                label={`Extracted actor ${actor.display_name}`}
                disabled={!hasMemorySeed(actor.topology_seed)}
                onPress={() => {
                  if (hasMemorySeed(actor.topology_seed))
                    onTopology(actor.topology_seed);
                }}
              >
                <Text style={{ color: kinds.actor }}>● </Text>
                {actor.display_name}
              </BriefAction>
            ))
          ) : pending ? (
            <LoadingState label="Loading actor entities…" />
          ) : (
            <Text style={s.micro}>
              {brief
                ? "No retained actor entities."
                : "Actor entities unavailable."}
            </Text>
          )}
          <Text style={s.micro}>
            Geospatial nodes and correlated IOC topology seeds are not supplied
            by this brief.
          </Text>
        </BriefSection>
      </View>
      <View style={[s.lower, narrow && s.narrowPadding]}>
        <Text style={[s.micro, { textAlign: "center" }]}>
          VISUALIZE 3D RELATIONAL DATA
        </Text>
        <BriefAction
          variant="cta"
          label="Launch 3D topology"
          disabled={!hasMemorySeed(brief?.topology_seed)}
          onPress={() => {
            if (hasMemorySeed(brief?.topology_seed))
              onTopology(brief.topology_seed);
          }}
        >
          {hasMemorySeed(brief?.topology_seed)
            ? "◇ LAUNCH 3D TOPOLOGY"
            : "BRAIN INDEXING PENDING"}
        </BriefAction>
      </View>
      <View style={[s.lower, narrow && s.narrowPadding]}>
        <BriefSection title="RETAINED BRIEF CONTROLS">
          {operations}
        </BriefSection>
      </View>
    </View>
  );
}
const s = StyleSheet.create({
  shell: {
    backgroundColor: c.panel,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: 16,
    minWidth: 0,
    width: "100%",
  },
  header: {
    backgroundColor: c.panelRaised,
    paddingHorizontal: 40,
    paddingTop: 40,
    paddingBottom: 44,
    borderBottomWidth: 1,
    borderColor: c.border,
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
    gap: 24,
  },
  narrowPadding: { paddingHorizontal: 20, paddingVertical: 28 },
  headerColumns: { flexDirection: "row", gap: 28, minWidth: 0 },
  titleColumn: {
    flexGrow: 1,
    flexShrink: 1,
    flexBasis: "auto",
    minWidth: 0,
    gap: 16,
  },
  title: {
    ...intelType.display,
    lineHeight: 50,
    color: c.text,
    fontFamily: intelFonts.body,
  },
  reference: {
    gap: 12,
    alignItems: "flex-end",
    flexShrink: 1,
    minWidth: 0,
    maxWidth: "100%",
  },
  date: { color: c.muted, fontSize: 20, fontFamily: intelFonts.mono },
  body: { padding: 52, gap: 40, minWidth: 0 },
  lede: {
    fontFamily: intelFonts.body,
    fontSize: 24,
    lineHeight: 38,
    color: c.dim,
  },
  section: { gap: 20, minWidth: 0 },
  sectionHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    borderBottomWidth: 1,
    borderColor: c.divider,
    paddingBottom: 16,
    minWidth: 0,
  },
  icon: {
    width: 22,
    height: 24,
    gap: 3,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  sectionTitle: {
    fontFamily: intelFonts.mono,
    fontWeight: "700",
    fontSize: 20,
    color: c.muted,
    flex: 1,
  },
  evidenceBox: {
    backgroundColor: c.inset,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: 12,
    padding: 28,
    gap: 20,
    minWidth: 0,
  },
  evidenceRow: { flexDirection: "row", gap: 20, minWidth: 0 },
  marker: { fontFamily: intelFonts.mono, fontSize: 22, flexShrink: 0 },
  evidenceText: {
    fontFamily: intelFonts.mono,
    fontSize: 22,
    lineHeight: 34,
    color: c.text,
    flex: 1,
    minWidth: 0,
  },
  micro: {
    ...intelType.micro,
    fontFamily: intelFonts.mono,
    color: c.muted,
    flexShrink: 1,
    lineHeight: 20,
  },
  lower: {
    borderTopWidth: 1,
    borderColor: c.border,
    backgroundColor: c.panelRaised,
    padding: 40,
    gap: 24,
    minWidth: 0,
  },
  origin: { flexDirection: "row", gap: 32, minWidth: 0 },
  field: {
    flexGrow: 1,
    flexShrink: 1,
    flexBasis: "auto",
    gap: 12,
    minWidth: 0,
  },
  value: { fontFamily: intelFonts.mono, fontSize: 22, lineHeight: 32 },
  action: {
    minHeight: 54,
    paddingHorizontal: 18,
    paddingVertical: 12,
    backgroundColor: c.inset,
    borderColor: c.borderStrong,
    borderWidth: 1,
    borderRadius: 6,
    justifyContent: "center",
    minWidth: 0,
    maxWidth: "100%",
  },
  actionText: {
    fontFamily: intelFonts.mono,
    fontSize: 22,
    lineHeight: 32,
    color: c.text,
    flexShrink: 1,
  },
  cta: {
    minHeight: 72,
    backgroundColor: c.accentSoft,
    borderColor: c.accentBorder,
    borderRadius: 10,
    alignItems: "center",
  },
  back: {
    borderWidth: 0,
    backgroundColor: "transparent",
    paddingHorizontal: 0,
    alignSelf: "flex-start",
  },
  ctaText: { color: c.accent, fontWeight: "800", fontSize: 22 },
  related: {
    backgroundColor: c.panelRaised,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: 14,
    padding: 26,
    gap: 16,
    minWidth: 0,
    flexBasis: 520,
    flexGrow: 1,
    maxWidth: "100%",
  },
  meta: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 14,
    minWidth: 0,
  },
  relatedTitle: {
    fontFamily: intelFonts.body,
    fontSize: 22,
    fontWeight: "700",
    color: c.text,
  },
  relatedCopy: {
    fontFamily: intelFonts.body,
    fontSize: 18,
    lineHeight: 28,
    color: c.dim,
  },
});
