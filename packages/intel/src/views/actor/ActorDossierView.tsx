"use client";
import React, { useState } from "react";
import { View, Text, Pressable, useWindowDimensions } from "react-native";
import type {
  ActorAssociation,
  ActorDossier,
  Article,
  Brief,
  Seed,
} from "@argos/api-client";
import { EmptyState, Notice, LoadingState, Skeleton } from "@argos/ui";
import { ExtractedActorDossierView } from "./ExtractedActorDossierView";
import { BriefAction } from "../brief/BriefAction";
import { BriefSection } from "../brief/BriefSection";
import { ArticleCard } from "../bulletin/ArticleCard";
import { DeskButton } from "../desk-ui";
import {
  intelColors as c,
  intelKindColors as kinds,
} from "../../shell/intel-theme";
import { dossierStyles as s } from "./dossierStyles";

function EvidenceDetails({ association }: { association: ActorAssociation }) {
  const [open, setOpen] = useState(false);
  return (
    <View style={s.record}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Provenance for ${association.node.label}`}
        accessibilityState={{ expanded: open }}
        aria-expanded={open}
        onPress={() => setOpen(!open)}
        style={s.disclosure}
      >
        <Text style={s.meta}>{open ? "− Hide" : "+ Show"} provenance</Text>
      </Pressable>
      {open ? (
        <View style={s.records}>
          {association.node.revision ? (
            <Text style={[s.meta, s.wrap]}>
              Revision · {association.node.revision}
            </Text>
          ) : null}
          <Text style={[s.meta, s.wrap]}>
            {association.relationship.rationale}
          </Text>
          {association.relationship.receipts.map((receipt, i) => (
            <Text
              key={`${association.relationship.id}:${i}`}
              style={[s.meta, s.wrap]}
            >
              {receipt}
            </Text>
          ))}
        </View>
      ) : null}
    </View>
  );
}
function InsightRecord({
  association: a,
  color,
  onTopology,
}: {
  association: ActorAssociation;
  color: string;
  onTopology: (seed: Seed) => void;
}) {
  const [hovered, setHovered] = useState(false),
    [focused, setFocused] = useState(false);
  return (
    <View style={s.record}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Explore ${a.node.topology_seed?.Inference ? "inference" : "claim"}: ${a.node.label}`}
        disabled={!a.node.topology_seed}
        onPress={() => {
          if (a.node.topology_seed) onTopology(a.node.topology_seed);
        }}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        style={[
          s.card,
          { borderLeftWidth: 2, borderLeftColor: color },
          (hovered || focused) && {
            borderColor: c.accent,
            backgroundColor: c.panelRaised,
          },
        ]}
      >
        <View style={s.toolbar}>
          <Text style={[s.meta, { color }]}>
            {a.relationship.actor_reference?.role || "Recorded reference"} ·{" "}
            {a.relationship.actor_reference?.binding_state ||
              a.relationship.status}
          </Text>
          {a.recorded_at ? (
            <Text style={s.meta}>Recorded · {a.recorded_at.slice(0, 10)}</Text>
          ) : null}
        </View>
        <Text style={[s.cardTitle, s.wrap]}>{a.node.label}</Text>
        <Text style={s.meta}>Open topology ↗</Text>
      </Pressable>
      <EvidenceDetails association={a} />
    </View>
  );
}
function Block({
  height,
  flex,
}: {
  height: number;
  flex?: number;
}) {
  return (
    <View
      style={{
        height,
        flex,
        width: flex ? undefined : "100%",
        minWidth: 0,
        borderRadius: 12,
        overflow: "hidden",
      }}
    >
      <Skeleton fill radius={12} />
    </View>
  );
}

function DossierSkeleton({
  narrow,
  back,
}: {
  narrow: boolean;
  back: React.ReactNode;
}) {
  return (
    <View
      nativeID="intel-actor-dossier"
      role="status"
      accessibilityLabel="Loading Actor Dossier…"
      style={s.shell}
    >
      <View style={s.header}>
        <View style={s.toolbar}>{back}</View>
        <View
          style={{
            flexDirection: narrow ? "column" : "row",
            gap: 12,
            alignItems: "stretch",
          }}
        >
          <Block height={108} flex={narrow ? undefined : 1} />
          <View style={{ width: narrow ? "100%" : 200, minWidth: 0 }}>
            <Block height={44} />
          </View>
        </View>
        <View style={s.metrics}>
          {[0, 1, 2].map((metric) => (
            <View key={metric} style={{ flexGrow: 1, flexBasis: 120, minWidth: 0 }}>
              <Block height={72} />
            </View>
          ))}
        </View>
      </View>
      <View style={[s.columns, { flexDirection: narrow ? "column" : "row" }]}>
        <View style={[s.reader, { flex: narrow ? undefined : 2 }]}>
          <Block height={88} />
          <Block height={160} />
          <Block height={160} />
          <Block height={220} />
        </View>
        <View
          style={[
            s.sidebar,
            { flex: narrow ? undefined : 1, width: narrow ? "100%" : undefined },
          ]}
        >
          <Block height={180} />
          <Block height={120} />
        </View>
      </View>
    </View>
  );
}

/** @tag:intel-brief — Compact dossier/detail using the Bulletin desk components. */
export function ActorDossierView({
  dossier: d,
  extractedActor,
  sourceArticle,
  pending,
  refreshing,
  error,
  missing,
  onRetry,
  onBack,
  fromBrief,
  onTopology,
  onArticle,
}: {
  dossier?: ActorDossier;
  extractedActor?: Brief["actors"][number];
  sourceArticle?: Article;
  pending: boolean;
  refreshing: boolean;
  error?: string;
  missing: boolean;
  onRetry: () => void;
  onBack: () => void;
  fromBrief: boolean;
  onTopology: (seed: Seed) => void;
  onArticle: (article: Article) => void;
}) {
  const narrow = useWindowDimensions().width < 900;
  const [cardWidth, setCardWidth] = useState(0);
  const back = (
    <BriefAction
      variant="back"
      label={fromBrief ? "Return to Brief" : "Return to Bulletin"}
      onPress={onBack}
    >
      {fromBrief ? "‹ Brief" : "‹ Bulletin"}
    </BriefAction>
  );
  if (!d && extractedActor && sourceArticle)
    return (
      <ExtractedActorDossierView
        actor={extractedActor}
        article={sourceArticle}
        onBack={onBack}
        fromBrief={fromBrief}
        onArticle={onArticle}
        onTopology={onTopology}
      />
    );
  if (!d)
    return pending ? (
      <DossierSkeleton narrow={narrow} back={back} />
    ) : (
      <View style={[s.shell, s.reader]}>
        {back}
        {missing ? (
          <EmptyState title="Actor unavailable">
            This identity is missing or is not an active actor.
          </EmptyState>
        ) : (
          <>
            <Notice>{error || "Actor profile unavailable"}</Notice>
            <DeskButton onPress={onRetry}>Retry</DeskButton>
          </>
        )}
      </View>
    );
  const inferences = d.inferences || [];
  return (
    <View nativeID="intel-actor-dossier" style={s.shell}>
      <View style={s.header}>
        <View style={s.toolbar}>
          {back}
          <BriefAction
            variant="back"
            label="Refresh dossier"
            disabled={refreshing}
            onPress={onRetry}
          >
            Refresh
          </BriefAction>
        </View>
        <View
          style={[
            s.toolbar,
            {
              alignItems: "flex-start",
              flexDirection: narrow ? "column" : "row",
            },
          ]}
        >
          <View
            style={[s.identity, narrow && { flex: undefined, width: "100%" }]}
          >
            <Text style={s.kicker}>ACTOR DOSSIER</Text>
            <Text accessibilityRole="header" style={[s.title, s.wrap]}>
              {d.display_name}
            </Text>
            <View style={s.tags}>
              <Text
                style={[
                  s.tag,
                  { color: kinds.actor, borderColor: kinds.actor },
                ]}
              >
                {d.catalog_type || d.subtype}
              </Text>
              <Text style={s.tag}>Identity: {d.identity_state}</Text>
              <Text style={s.tag}>
                {d.catalog_source_id ? "Catalog identity" : "Stored identity"}
              </Text>
            </View>
            <Text style={[s.meta, s.wrap]}>Canonical ID · {d.id}</Text>
          </View>
          <DeskButton active onPress={() => onTopology({ Actor: d.id })}>
            OPEN ACTOR TOPOLOGY
          </DeskButton>
        </View>
        <View style={s.metrics}>
          {[
            {
              value: d.loaded_claims,
              label: "RECORDED CLAIM REFERENCES",
              color: kinds.claim,
            },
            {
              value: d.loaded_inferences ?? inferences.length,
              label: "RECORDED INFERENCES",
              color: kinds.inference,
            },
            {
              value: d.loaded_briefs,
              label: "RETAINED ARTICLE MENTIONS",
              color: c.accent,
            },
          ].map((metric) => (
            <View key={metric.label} style={s.metric}>
              <Text style={[s.number, { color: metric.color }]}>
                {metric.value}
              </Text>
              <Text style={s.meta}>{metric.label}</Text>
            </View>
          ))}
        </View>
        {refreshing ? (
          <LoadingState compact label="Refreshing Actor Dossier…" />
        ) : null}
        {error ? (
          <>
            <Notice>{error}</Notice>
            <DeskButton onPress={onRetry}>Retry</DeskButton>
          </>
        ) : null}
      </View>
      <View style={[s.columns, { flexDirection: narrow ? "column" : "row" }]}>
        <View
          style={[
            s.reader,
            {
              flex: narrow ? undefined : 2,
              width: narrow ? "100%" : undefined,
            },
          ]}
        >
          <BriefSection title="ACTOR OVERVIEW">
            <Text style={s.body}>
              {d.description ||
                `${d.display_name} is a recorded ${d.catalog_type || d.subtype} identity (${d.identity_state}).`}
            </Text>
            {d.description ? (
              <Text style={s.meta}>
                Catalog description · {d.catalog_source_id || "Stored catalog"}
              </Text>
            ) : null}
          </BriefSection>
          <BriefSection
            title="ASSOCIATED CLAIMS"
            color={kinds.claim}
            count={d.loaded_claims}
          >
            <Text style={s.meta}>
              Showing {d.loaded_claims} retained claim references
            </Text>
            {d.claims.length ? (
              <View style={s.records}>
                {d.claims.map((a) => (
                  <InsightRecord
                    key={a.relationship.id}
                    association={a}
                    color={kinds.claim}
                    onTopology={onTopology}
                  />
                ))}
              </View>
            ) : (
              <EmptyState title="No recorded actor–claim references">
                Claims require a recorded actor reference; an article mention
                alone is not a claim association.
              </EmptyState>
            )}
          </BriefSection>
          <BriefSection
            title="ASSOCIATED INFERENCES"
            color={kinds.inference}
            count={d.loaded_inferences ?? inferences.length}
          >
            <Text style={s.meta}>
              Showing {d.loaded_inferences ?? inferences.length} retained
              inference references
            </Text>
            {inferences.length ? (
              <View style={s.records}>
                {inferences.map((a) => (
                  <InsightRecord
                    key={a.relationship.id}
                    association={a}
                    color={kinds.inference}
                    onTopology={onTopology}
                  />
                ))}
              </View>
            ) : (
              <EmptyState title="No recorded actor–inference references">
                Premise associations are labeled separately from independent
                support.
              </EmptyState>
            )}
          </BriefSection>
          <BriefSection
            title="LINKED FOCUS BRIEFS"
            color={c.accent}
            count={d.loaded_briefs}
          >
            <Text style={s.meta}>
              Showing {d.loaded_briefs} retained article mentions · mention is
              not corroboration
            </Text>
            <View
              onLayout={(event) => setCardWidth(event.nativeEvent.layout.width)}
              style={s.cards}
            >
              {d.briefs.length ? (
                d.briefs.map((a) =>
                  a.article ? (
                    <View
                      key={a.relationship.id}
                      style={{
                        width: cardWidth >= 680 ? "48%" : "100%",
                        minWidth: 0,
                        gap: 8,
                      }}
                    >
                      <Text style={s.meta}>Actor mentioned in article</Text>
                      <ArticleCard
                        article={a.article}
                        onPress={() => {
                          if (a.article) onArticle(a.article);
                        }}
                      />
                      <EvidenceDetails association={a} />
                    </View>
                  ) : null,
                )
              ) : (
                <EmptyState title="No retained article mentions">
                  Only indexed, current-revision mentions appear here.
                </EmptyState>
              )}
            </View>
          </BriefSection>
          {d.truncated || d.gaps.length > 0 ? (
            <View style={s.card}>
              <Text style={s.meta}>
                Partial neighborhood—open topology to explore.
              </Text>
              {d.gaps.map((gap) => (
                <Text key={gap} style={[s.meta, s.wrap]}>
                  {gap}
                </Text>
              ))}
            </View>
          ) : null}
        </View>
        <View
          style={[
            s.sidebar,
            {
              flex: narrow ? undefined : 1,
              width: narrow ? "100%" : undefined,
              borderLeftWidth: narrow ? 0 : 1,
              borderTopWidth: narrow ? 1 : 0,
            },
          ]}
        >
          <BriefSection title="CATALOG METADATA">
            <View style={s.field}>
              <Text style={s.meta}>SOURCE CATALOG ID</Text>
              <Text style={[s.body, s.wrap]}>
                {d.catalog_source_id || "Unavailable (legacy actor)"}
              </Text>
            </View>
            <View style={s.field}>
              <Text style={s.meta}>TYPE</Text>
              <Text style={s.body}>{d.catalog_type || d.subtype}</Text>
            </View>
            {d.metadata.map((field) => (
              <View key={field.key} style={s.field}>
                <Text style={s.meta}>
                  {field.key.replaceAll("_", " ").toUpperCase()}
                </Text>
                <Text style={[s.body, s.wrap]}>
                  {field.value ?? "Unavailable (malformed source value)"}
                </Text>
              </View>
            ))}
          </BriefSection>
          <BriefSection title="KNOWN ALIASES" count={d.aliases.length}>
            {d.aliases.length ? (
              <View style={s.tags}>
                {d.aliases.map((alias) => (
                  <Text key={alias} style={[s.tag, s.wrap]}>
                    {alias}
                  </Text>
                ))}
              </View>
            ) : (
              <EmptyState title="No known aliases recorded" />
            )}
          </BriefSection>
        </View>
      </View>
    </View>
  );
}
