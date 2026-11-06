"use client";
import React from "react";
import { View, Text, StyleSheet } from "react-native";
import {
  BRIEF_FACETS,
  useBackend,
  useTopologySummary,
  useGenerateTopologySummary,
  type NeighborhoodBrief,
  type NeighborhoodBriefItem,
  type Seed,
  type Topology,
} from "@argos/api-client";
import { Button, Copy, Notice, Panel, fonts, colors } from "@argos/ui";

/** Default-density evidence summary in the existing topology reading flow. */
export function NeighborhoodSummary({
  seed,
  fallback,
  nodes,
  onSeed,
}: {
  seed: string | null;
  fallback: NeighborhoodBrief;
  nodes: Topology["nodes"];
  onSeed: (seed: Seed) => void;
}) {
  const backend = useBackend();
  const delivery = useTopologySummary(seed, fallback.basis_key);
  const generate = useGenerateTopologySummary();
  // Each query is occurrence/basis scoped. An older mutation cannot replace
  // the current graph's fallback when navigation changes that identity.
  const current =
    delivery.data?.brief.basis_key === fallback.basis_key
      ? delivery.data
      : undefined;
  const brief = current?.brief ?? fallback;
  const sameAction =
    generate.variables?.seed === seed &&
    generate.variables?.basis === fallback.basis_key;
  const pending =
    current?.kind === "pending" || (sameAction && generate.isPending);
  const supported =
    !!seed &&
    !seed.startsWith("brief:") &&
    brief.coverage.admitted_edges > 0 &&
    !["isolated", "missing"].includes(brief.coverage.status);
  const sections = brief.facets?.length
    ? BRIEF_FACETS.map((facet) => {
        const block = brief.facets?.find((block) => block.facet === facet.id);
        return {
          key: facet.id,
          name: facet.name,
          items: block?.items ?? [],
          empty:
            block?.empty_reason ??
            "No supported retained evidence for this facet.",
        };
      })
    : [
        {
          key: "legacy-overview",
          name: "Overview",
          items: brief.overview ? [brief.overview] : [],
          empty: "No recorded overview.",
        },
        {
          key: "legacy-connections",
          name: "Connections",
          items: brief.connections,
          empty: "No recorded connections.",
        },
        {
          key: "legacy-interpretation",
          name: "Interpretation",
          items: brief.interpretation,
          empty: "No stored interpretation.",
        },
        {
          key: "legacy-limits",
          name: "Limits",
          items: brief.limits,
          empty: "No recorded limitations.",
        },
      ];
  return (
    <Panel title="Neighborhood summary">
      <Copy>
        {current?.kind === "generated"
          ? `Generated with ${current.route}`
          : pending
            ? "Generation is queued or running. Recorded evidence remains visible."
            : "Recorded evidence · generate an interpretation when you need it."}
      </Copy>
      {!brief.facets?.length ? <Copy>Legacy summary</Copy> : null}
      {brief.coverage.status === "limited" ? (
        <Notice>
          Limited coverage: omitted evidence can change this interpretation.
        </Notice>
      ) : null}
      <Copy>
        {brief.coverage.admitted_nodes} of {brief.coverage.discovered_nodes}{" "}
        discovered records · {brief.coverage.admitted_edges} of{" "}
        {brief.coverage.discovered_edges} relationships ·{" "}
        {brief.coverage.payload_reads_skipped} payload reads skipped
      </Copy>
      {current?.kind === "failed" ? (
        <Notice>Generation unavailable: {current.diagnostic.message}</Notice>
      ) : null}
      {delivery.error ? (
        <Notice>Summary status unavailable: {delivery.error.message}</Notice>
      ) : null}
      {sameAction && generate.error ? (
        <Notice>Generation was not accepted: {generate.error.message}</Notice>
      ) : null}
      <View style={s.actions}>
        {supported ? (
          <Button
            disabled={!backend.connected || pending}
            onPress={() => {
              if (seed) generate.mutate({ seed, basis: fallback.basis_key });
            }}
          >
            {pending
              ? "Generation pending"
              : current?.kind === "failed"
                ? "Retry summary"
                : "Generate summary"}
          </Button>
        ) : null}
        <Button
          disabled={!backend.connected || !seed || delivery.isFetching}
          onPress={() => {
            void delivery.refetch();
          }}
        >
          Refresh summary status
        </Button>
      </View>
      {sections.map((section) => (
        <View key={section.key} style={s.section}>
          <Text accessibilityRole="header" style={s.heading}>
            {section.name}
          </Text>
          {section.items.length ? (
            section.items.map((item) => (
              <Copy key={itemKey(item)}>
                {item.text}
                {item.basis.length ? ` [${item.basis.join(", ")}]` : ""}
              </Copy>
            ))
          ) : (
            <Copy>{section.empty}</Copy>
          )}
        </View>
      ))}
      {brief.references.length ? (
        <View style={s.section}>
          <Text accessibilityRole="header" style={s.heading}>
            Retained basis
          </Text>
          {brief.references.map((reference) => {
            const id =
              "Node" in reference.target ? reference.target.Node : null;
            const node = id ? nodes.find((node) => node.id === id) : undefined;
            return node?.topology_seed ? (
              <Button
                key={reference.n}
                onPress={() => {
                  if (node.topology_seed) onSeed(node.topology_seed);
                }}
              >
                [{reference.n}] {reference.label}
              </Button>
            ) : (
              <Copy key={reference.n}>
                [{reference.n}] {reference.label}
              </Copy>
            );
          })}
        </View>
      ) : null}
      {current?.kind === "generated"
        ? current.host_notes.map((note) => <Copy key={note}>{note}</Copy>)
        : null}
    </Panel>
  );
}
function itemKey(item: NeighborhoodBriefItem) {
  return `${item.epistemic}:${item.node_ids.join(",")}:${item.edge_ids.join(",")}:${item.text}`;
}
const s = StyleSheet.create({
  section: { gap: 8, minWidth: 0, marginTop: 16 },
  heading: { color: colors.text, fontFamily: fonts.mono, fontSize: 13 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8, minWidth: 0 },
});
