"use client";
import React, {
  lazy,
  Suspense,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  View,
  Text,
  TextInput,
  Pressable,
  ScrollView,
  useWindowDimensions,
  StyleSheet,
  type LayoutChangeEvent,
} from "react-native";
import { useTopology, type Seed } from "@argos/api-client";
import {
  Button,
  Label,
  Copy,
  Panel,
  Notice,
  LoadingState,
  EmptyState,
  Frame,
  StatusTag,
  fonts,
  glass,
  toneColor,
  type Tone,
  colors as c,
  ErrorBoundary,
} from "@argos/ui";
import type { CanvasHandle, GLink, GNode } from "./ForceCanvas";

// Canvas + d3 only load in the browser (static export must not evaluate them).
const ForceCanvas = lazy(() => import("./ForceCanvas"));

type TopoData = NonNullable<ReturnType<typeof useTopology>["data"]>;
type TNode = TopoData["nodes"][number];
type TEdge = TopoData["edges"][number];

const tones: Tone[] = ["accent", "success", "warning", "danger", "purple"];
const kindTone = (kind: string): Tone => {
  let h = 0;
  for (const ch of kind) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return tones[h % tones.length];
};
const noopSubscribe = () => () => {};
const subscribeMotion = (cb: () => void) => {
  const q = window.matchMedia("(prefers-reduced-motion: reduce)");
  q.addEventListener("change", cb);
  return () => q.removeEventListener("change", cb);
};
const motionSnapshot = () =>
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export function Topology({
  seed,
  onSeed,
  pageStyle,
  titleStyle,
}: {
  seed: Seed | null;
  onSeed: (s: Seed) => void;
  pageStyle: object;
  titleStyle: object;
}) {
  const graph = useTopology(seed);
  return (
    <View style={pageStyle}>
      <Label>Brain / Topology</Label>
      <Text accessibilityRole="header" style={titleStyle}>
        {graph.data?.root_label || "Evidence topology"}
      </Text>
      <Copy>
        Select an actor, claim or article from a Focus Brief to begin.
        Connections preserve their recorded meaning.
      </Copy>
      {graph.error && <Notice>{graph.error.message}</Notice>}
      {!seed ? (
        <EmptyState title="No topology selected">
          Open an actor, claim or article from a Focus Brief.
        </EmptyState>
      ) : null}
      {seed && graph.isPending ? (
        <LoadingState label="Loading topology…" />
      ) : null}
      {graph.isRefetching ? (
        <LoadingState label="Refreshing topology…" compact />
      ) : null}
      {seed && graph.isSuccess && graph.data.nodes.length === 0 ? (
        <EmptyState title="No retained connections">
          No topology records are available for this seed.
        </EmptyState>
      ) : null}
      {graph.data && graph.data.nodes.length > 0 ? (
        <GraphExplorer data={graph.data} onSeed={onSeed} />
      ) : null}
      {graph.data?.findings.gaps.map((g, i) => (
        <Notice key={i}>{g}</Notice>
      ))}
      {graph.data?.has_continuation && (
        <Copy>
          Showing bounded topology. Navigate a node to explore further.
        </Copy>
      )}
    </View>
  );
}

function GraphExplorer({
  data,
  onSeed,
}: {
  data: TopoData;
  onSeed: (s: Seed) => void;
}) {
  const { width: winWidth } = useWindowDimensions();
  const wide = winWidth >= 900;
  const client = useSyncExternalStore(
    noopSubscribe,
    () => true,
    () => false,
  );
  const reduced = useSyncExternalStore(
    subscribeMotion,
    motionSnapshot,
    () => false,
  );
  const canvas = useRef<CanvasHandle>(null);
  const [box, setBox] = useState(0);
  // undefined = default to first node; null = explicitly cleared.
  const [selectedId, setSelectedId] = useState<string | null | undefined>(
    undefined,
  );
  const [hiddenKinds, setHiddenKinds] = useState<string[]>([]);
  const [query, setQuery] = useState("");
  const [edgeLabel, setEdgeLabel] = useState<string | null>(null);
  const [depth, setDepth] = useState<0 | 1 | 2>(0);
  const [paused, setPaused] = useState(false);

  const byId = new Map(data.nodes.map((n) => [n.id, n]));
  const edges = data.edges.filter((e) => byId.has(e.from) && byId.has(e.to));
  const kinds = [...new Set(data.nodes.map((n) => n.kind))].sort();
  const edgeLabels = [...new Set(edges.map((e) => e.label))].sort();
  const selected: TNode | null =
    selectedId === null
      ? null
      : ((selectedId ? byId.get(selectedId) : undefined) ?? data.nodes[0]);

  const adjacency = new Map<string, Set<string>>();
  for (const e of edges) {
    if (!adjacency.has(e.from)) adjacency.set(e.from, new Set());
    if (!adjacency.has(e.to)) adjacency.set(e.to, new Set());
    adjacency.get(e.from)!.add(e.to);
    adjacency.get(e.to)!.add(e.from);
  }
  let reach: Set<string> | null = null;
  if (depth > 0 && selected) {
    reach = new Set([selected.id]);
    let frontier = [selected.id];
    for (let d = 0; d < depth; d++) {
      const next: string[] = [];
      for (const id of frontier)
        for (const nb of adjacency.get(id) ?? [])
          if (!reach.has(nb)) {
            reach.add(nb);
            next.push(nb);
          }
      frontier = next;
    }
  }
  const q = query.trim().toLowerCase();
  const nodes = data.nodes.filter(
    (n) =>
      n.id === selected?.id ||
      (!hiddenKinds.includes(n.kind) &&
        (!q || n.label.toLowerCase().includes(q)) &&
        (!reach || reach.has(n.id))),
  );
  const visible = new Set(nodes.map((n) => n.id));
  const links = edges.filter(
    (e) =>
      visible.has(e.from) &&
      visible.has(e.to) &&
      (edgeLabel === null || e.label === edgeLabel),
  );
  const filtered =
    hiddenKinds.length > 0 || q !== "" || edgeLabel !== null || depth > 0;
  const focusIds = selected
    ? new Set([selected.id, ...(adjacency.get(selected.id) ?? [])])
    : null;
  const gNodes: GNode[] = nodes.map((n) => ({
    id: n.id,
    label: n.label,
    kind: n.kind,
  }));
  const gLinks: GLink[] = links.map((e) => ({
    id: e.id,
    source: e.from,
    target: e.to,
    label: e.label,
  }));
  const kindColor = Object.fromEntries(
    kinds.map((k) => [k, toneColor[kindTone(k)]]),
  );
  const summary = `Topology graph with ${data.nodes.length} nodes and ${edges.length} connections across ${kinds.length} kinds. ${
    selected ? `Selected ${selected.label}.` : "No node selected."
  } Use the node and connection lists for keyboard access.`;
  const reset = () => {
    setHiddenKinds([]);
    setQuery("");
    setEdgeLabel(null);
    setDepth(0);
  };
  const onLayout = (e: LayoutChangeEvent) =>
    setBox(Math.floor(e.nativeEvent.layout.width));
  const height = wide ? 520 : 340;

  // Neighbors of the selection, grouped by kind.
  const hood = new Map<
    string,
    { key: string; node: TNode; edge: TEdge; out: boolean }[]
  >();
  if (selected)
    for (const e of edges) {
      if (e.from !== selected.id && e.to !== selected.id) continue;
      const out = e.from === selected.id;
      const node = byId.get(out ? e.to : e.from)!;
      const list = hood.get(node.kind) ?? [];
      list.push({ key: e.id, node, edge: e, out });
      hood.set(node.kind, list);
    }
  const degree = (id: string) => ({
    out: edges.filter((e) => e.from === id).length,
    in: edges.filter((e) => e.to === id).length,
  });
  const maxDegree = Math.max(
    1,
    ...data.nodes.map((n) => {
      const d = degree(n.id);
      return d.in + d.out;
    }),
  );
  const sel = selected ? degree(selected.id) : null;

  const graphArea = (
    <View style={[t.graphCol, wide && t.graphColWide]}>
      <Frame>
        <View
          role="img"
          aria-label={summary}
          onLayout={onLayout}
          style={[t.canvasBox, { height }]}
        >
          {client && box > 0 && nodes.length > 0 ? (
            <ErrorBoundary label="Topology graph" onRetry={reset}>
              <Suspense fallback={<LoadingState label="Loading graph…" />}>
                <ForceCanvas
                  ref={canvas}
                  nodes={gNodes}
                  links={gLinks}
                  width={box}
                  height={height}
                  kindColor={kindColor}
                  palette={{
                    text: c.text,
                    dim: c.dim,
                    bg: c.bg,
                    accent: c.accent,
                  }}
                  selectedId={selected?.id ?? null}
                  focusIds={focusIds}
                  reducedMotion={reduced}
                  onSelect={setSelectedId}
                />
              </Suspense>
            </ErrorBoundary>
          ) : null}
          {nodes.length === 0 ? (
            <View style={t.overlay}>
              <EmptyState title="No nodes match the filters">
                Every node is hidden by the current filters.
              </EmptyState>
              <Button onPress={reset}>Reset filters</Button>
            </View>
          ) : null}
        </View>
      </Frame>
      <View style={t.controls} role="group" aria-label="Graph legend">
        {kinds.map((kind) => (
          <StatusTag key={kind} tone={kindTone(kind)}>
            {kind}
          </StatusTag>
        ))}
        <Copy>
          Lines show recorded relationships. Select a node to inspect its
          neighborhood.
        </Copy>
      </View>
      <Text style={t.count} accessibilityLiveRegion="polite">
        Showing {nodes.length} of {data.nodes.length} nodes · {links.length} of{" "}
        {edges.length} connections
      </Text>
    </View>
  );

  const details = (
    <Panel title="Neighborhood">
      {selected ? (
        <View style={{ gap: 8 }}>
          <StatusTag tone={kindTone(selected.kind)}>{selected.kind}</StatusTag>
          <Text style={t.headline}>{selected.label}</Text>
          <Copy>{selected.detail}</Copy>
          {selected.topology_seed && (
            <Button onPress={() => onSeed(selected.topology_seed!)}>
              Navigate this node →
            </Button>
          )}
          {sel && (
            <View style={{ gap: 4 }}>
              <DegreeBar
                label="Outgoing"
                value={sel.out}
                max={maxDegree}
                color={c.accent}
              />
              <DegreeBar
                label="Incoming"
                value={sel.in}
                max={maxDegree}
                color={c.accent2}
              />
            </View>
          )}
          {hood.size === 0 ? (
            <Copy>No retained connections for this node.</Copy>
          ) : (
            [...hood.entries()].map(([kind, items]) => (
              <View key={kind} style={{ gap: 4 }}>
                <Label>
                  {kind} ({items.length})
                </Label>
                {items.map((it) => (
                  <Button
                    key={it.key}
                    variant="quiet"
                    onPress={() => setSelectedId(it.node.id)}
                    label={`Select neighbor ${it.node.label}, ${
                      it.out ? "outgoing" : "incoming"
                    } ${it.edge.label}`}
                  >
                    {it.out ? "→" : "←"} {it.edge.label} · {it.node.label}
                  </Button>
                ))}
              </View>
            ))
          )}
        </View>
      ) : (
        <Copy>No node selected. Choose one from the graph or the list.</Copy>
      )}
    </Panel>
  );

  const nodeList = (
    <Panel title="Nodes">
      <ScrollView style={t.listBox} nestedScrollEnabled>
        <View role="list" aria-label="Topology nodes">
          {nodes.map((n) => {
            const on = n.id === selected?.id;
            return (
              <View role="listitem" key={n.id}>
                <NodeListRow
                  node={n}
                  dotColor={kindColor[n.kind] ?? c.accent}
                  on={on}
                  onPress={() => setSelectedId(n.id)}
                />
              </View>
            );
          })}
        </View>
      </ScrollView>
    </Panel>
  );

  return (
    <View style={{ gap: 12, minWidth: 0 }}>
      <Frame>
        <View style={t.controls}>
          <Button onPress={() => canvas.current?.zoomBy(1.4)}>Zoom in</Button>
          <Button onPress={() => canvas.current?.zoomBy(1 / 1.4)}>
            Zoom out
          </Button>
          <Button onPress={() => canvas.current?.fit()}>Fit</Button>
          <Button onPress={() => canvas.current?.reset()}>Reset</Button>
          <Button
            active={paused}
            onPress={() => {
              if (paused) canvas.current?.resume();
              else canvas.current?.pause();
              setPaused(!paused);
            }}
          >
            {paused ? "Resume layout" : "Pause layout"}
          </Button>
          <Button disabled={!selected} onPress={() => setSelectedId(null)}>
            Clear selection
          </Button>
        </View>
        <View style={t.controls}>
          <TextInput
            accessibilityLabel="Filter nodes by label"
            placeholder="Filter by label"
            placeholderTextColor={c.muted}
            value={query}
            onChangeText={setQuery}
            style={t.input}
          />
          {filtered ? (
            <Button variant="quiet" onPress={reset}>
              Reset filters
            </Button>
          ) : null}
        </View>
        <View style={t.controls} role="group" aria-label="Node kinds">
          {kinds.map((k) => {
            const hidden = hiddenKinds.includes(k);
            return (
              <Button
                key={k}
                active={!hidden}
                label={`Show ${k} nodes`}
                onPress={() =>
                  setHiddenKinds(
                    hidden
                      ? hiddenKinds.filter((x) => x !== k)
                      : [...hiddenKinds, k],
                  )
                }
              >
                {k}
              </Button>
            );
          })}
        </View>
        <View style={t.controls} role="group" aria-label="Connection labels">
          <Button
            active={edgeLabel === null}
            label="All connection labels"
            onPress={() => setEdgeLabel(null)}
          >
            All connections
          </Button>
          {edgeLabels.map((l) => (
            <Button
              key={l}
              active={edgeLabel === l}
              label={`Connections labelled ${l}`}
              onPress={() => setEdgeLabel(edgeLabel === l ? null : l)}
            >
              {l}
            </Button>
          ))}
        </View>
        <View style={t.controls} role="group" aria-label="Neighbor depth">
          <Button active={depth === 0} onPress={() => setDepth(0)}>
            All nodes
          </Button>
          <Button
            active={depth === 1}
            disabled={!selected}
            onPress={() => setDepth(1)}
          >
            Neighbors depth 1
          </Button>
          <Button
            active={depth === 2}
            disabled={!selected}
            onPress={() => setDepth(2)}
          >
            Neighbors depth 2
          </Button>
        </View>
      </Frame>
      <View style={[t.layout, wide && t.layoutWide]}>
        {graphArea}
        <View style={[t.sideCol, wide && t.sideColWide]}>
          {details}
          {nodeList}
        </View>
      </View>
      <Panel title="Connections">
        {links.length === 0 ? (
          <Copy>No connections match the current filters.</Copy>
        ) : (
          <View role="list" aria-label="Topology connections">
            {links.map((e) => (
              <View role="listitem" key={e.id} style={t.edgeRow}>
                <Copy>
                  {byId.get(e.from)?.label || e.from} → {e.label} →{" "}
                  {byId.get(e.to)?.label || e.to}
                </Copy>
              </View>
            ))}
          </View>
        )}
      </Panel>
    </View>
  );
}

function DegreeBar({
  label,
  value,
  max,
  color,
}: {
  label: string;
  value: number;
  max: number;
  color: string;
}) {
  return (
    <View
      role="img"
      aria-label={`${label} connections: ${value}`}
      style={t.barRow}
    >
      <Text style={t.barLabel}>
        {label} {value}
      </Text>
      <View style={t.barTrack}>
        <View
          style={[
            t.barFill,
            { backgroundColor: color, width: `${(value / max) * 100}%` },
          ]}
        />
      </View>
    </View>
  );
}

function NodeListRow({
  node,
  dotColor,
  on,
  onPress,
}: {
  node: TNode;
  dotColor: string;
  on: boolean;
  onPress: () => void;
}) {
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Select node ${node.label}, ${node.kind}`}
      aria-pressed={on}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={({ pressed }) => [
        t.item,
        !on && (hovered || pressed) && t.itemHover,
        !on && pressed && t.itemPressed,
        on && t.itemOn,
      ]}
    >
      <View style={[t.dot, { backgroundColor: dotColor }]} />
      <Text style={t.itemText} numberOfLines={2}>
        {node.label}
      </Text>
      <Text style={t.itemKind}>{node.kind}</Text>
    </Pressable>
  );
}

const t = StyleSheet.create({
  controls: { flexDirection: "row", flexWrap: "wrap", gap: 8, minWidth: 0 },
  layout: { gap: 12, minWidth: 0 },
  layoutWide: { flexDirection: "row", alignItems: "flex-start" },
  graphCol: { width: "100%", minWidth: 0, gap: 6 },
  graphColWide: { flexGrow: 3, flexShrink: 1, flexBasis: 0 },
  sideCol: { width: "100%", minWidth: 0, gap: 12 },
  sideColWide: { flexGrow: 2, flexShrink: 1, flexBasis: 0 },
  canvasBox: {
    width: "100%",
    minWidth: 0,
    overflow: "hidden",
    alignItems: "center",
    justifyContent: "center",
  },
  overlay: { position: "absolute", alignItems: "center", gap: 8, padding: 12 },
  count: { fontFamily: fonts.mono, fontSize: 12, color: c.dim },
  headline: { fontSize: 19, lineHeight: 28, fontWeight: "600", color: c.text },
  input: {
    ...(glass as object),
    flexGrow: 1,
    flexBasis: 160,
    minWidth: 0,
    color: c.text,
    fontFamily: fonts.body,
    fontSize: 14,
    paddingVertical: 8,
    paddingHorizontal: 10,
    borderRadius: 2,
  },
  listBox: { maxHeight: 260 },
  item: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingVertical: 8,
    paddingHorizontal: 8,
    borderLeftWidth: 3,
    borderLeftColor: "transparent",
    minHeight: 40,
    minWidth: 0,
  },
  itemOn: { borderLeftColor: c.accent, backgroundColor: c.accentSoft },
  itemHover: { backgroundColor: c.glassStrong },
  itemPressed: { backgroundColor: c.glassStrong, borderLeftColor: c.border },
  dot: { width: 10, height: 10, borderRadius: 5 },
  itemText: { flex: 1, minWidth: 0, color: c.text, fontSize: 14 },
  itemKind: { fontFamily: fonts.mono, fontSize: 11, color: c.muted },
  edgeRow: { paddingVertical: 4 },
  barRow: { gap: 2 },
  barLabel: { fontFamily: fonts.mono, fontSize: 12, color: c.dim },
  barTrack: {
    height: 6,
    backgroundColor: c.raised,
    borderRadius: 3,
    overflow: "hidden",
  },
  barFill: { height: 6, borderRadius: 3 },
});
