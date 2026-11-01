"use client";
import { Skeleton } from "@argos/ui";
import { InspectorPresence } from "./InspectorPresence";
import { useSelectedNodeScroll } from "./selection.web";
import React, {
  lazy,
  Suspense,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  View,
  Image,
  Text,
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
  fonts,
  colors as c,
  ErrorBoundary,
} from "@argos/ui";
import type { CanvasHandle, GLink, GNode } from "./ForceCanvas";
import { kindHue, kindIconSvg } from "./nodeGlyph";
import { classifyNeighborhood } from "./neighborhood";
import { intelColors, topologyColors } from "../shell/intel-theme";

// Canvas + d3 only load in the browser (server rendering must not evaluate them).
const ForceCanvas = lazy(() => import("./ForceCanvas"));

type TopoData = NonNullable<ReturnType<typeof useTopology>["data"]>;
type TNode = TopoData["nodes"][number];
type TEdge = TopoData["edges"][number];

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
  displayScale = 1,
}: {
  seed: string | null;
  onSeed: (s: Seed) => void;
  pageStyle: object;
  displayScale?: number;
}) {
  const graph = useTopology(seed);
  return (
    <View style={[pageStyle, { minWidth: 0, maxWidth: "100%" }]}>
      {!seed ? (
        <Copy>
          Select an actor, claim or article from a Focus Brief to begin.
          Connections preserve their recorded meaning.
        </Copy>
      ) : null}
      {graph.error && <Notice>{graph.error.message}</Notice>}
      {!seed ? (
        <EmptyState title="No topology selected">
          Open an actor, claim or article from a Focus Brief.
        </EmptyState>
      ) : null}
      {seed && graph.isPending ? (
        <CanvasSkeleton label="Loading topology…" />
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
        <GraphExplorer
          key={seed}
          data={graph.data}
          onSeed={onSeed}
          displayScale={displayScale}
        />
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
  data: sourceData,
  onSeed,
  displayScale,
}: {
  data: TopoData;
  displayScale: number;
  onSeed: (s: Seed) => void;
}) {
  const retainedNodes = sourceData.nodes.filter(
    (node) => node.kind !== "topic",
  );
  const retainedIds = new Set(retainedNodes.map((node) => node.id));
  const data = {
    ...sourceData,
    nodes: retainedNodes,
    edges: sourceData.edges.filter(
      (edge) => retainedIds.has(edge.from) && retainedIds.has(edge.to),
    ),
  };
  const { width: winWidth, height: winHeight } = useWindowDimensions();
  const wide = winWidth >= 1100;
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
  const [depth, setDepth] = useState<0 | 1 | 2>(1);
  const [view] = useState(createViewStore);
  const history = useRef<string[]>([]);
  const explicitSecond = useRef(false);

  const byId = new Map(data.nodes.map((n) => [n.id, n]));
  const edges = data.edges.filter((e) => byId.has(e.from) && byId.has(e.to));
  const kinds = [
    ...new Set(["inference", ...data.nodes.map((node) => node.kind)]),
  ].sort();
  const selected: TNode | null =
    selectedId === null
      ? null
      : ((selectedId ? byId.get(selectedId) : undefined) ??
        data.nodes.find((node) => node.is_root) ??
        data.nodes[0]);

  useSelectedNodeScroll(selected?.id);

  const neighborhood = classifyNeighborhood(
    data.nodes.map((node) => node.id),
    edges,
    selected?.id ?? null,
    depth,
  );
  const nodes = data.nodes.filter((n) => {
    const role = neighborhood.role.get(n.id);
    if (!role) return false;
    if (n.id === selected?.id) return true;
    return !hiddenKinds.includes(n.kind);
  });
  const visible = new Set(nodes.map((n) => n.id));
  const links = edges.filter(
    (e) =>
      neighborhood.drawnEdgeIds.has(e.id) &&
      visible.has(e.from) &&
      visible.has(e.to),
  );
  const focusIds = selected ? new Set(nodes.map((node) => node.id)) : null;
  const gNodes: GNode[] = nodes.map((n) => ({
    id: n.id,
    label: n.label,
    kind: n.kind,
    role: neighborhood.role.get(n.id),
  }));
  const gLinks: GLink[] = links.map((e) => ({
    id: e.id,
    source: e.from,
    target: e.to,
    label: e.label,
  }));
  const kindColor = Object.fromEntries(
    kinds.map((kind) => [kind, kindHue(kind)]),
  );
  const summary = `Topology graph with ${data.nodes.length} nodes and ${edges.length} connections across ${kinds.length} kinds. ${
    selected ? `Selected ${selected.label}.` : "No node selected."
  } Use the node and connection lists for keyboard access.`;
  const reset = () => {
    setHiddenKinds([]);
    explicitSecond.current = false;
    setDepth(selected ? 1 : 0);
  };
  function choose(id: string | null) {
    const previous = selected?.id;
    if (id && previous && id !== previous) history.current.push(previous);
    setSelectedId(id);
    if (id && !explicitSecond.current) setDepth(1);
    const target = id ? byId.get(id)?.topology_seed : null;
    if (
      id &&
      id !== previous &&
      target &&
      [target.Actor, target.Claim, target.Inference].some(
        (value) => typeof value === "string" && !!value,
      )
    )
      onSeed(target);
  }
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        event.key !== "Backspace" &&
        !(event.altKey && event.key === "ArrowLeft")
      )
        return;
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA")
      )
        return;
      const previous = history.current.pop();
      if (!previous) return;
      setSelectedId(previous);
      if (!explicitSecond.current) setDepth(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const onLayout = (e: LayoutChangeEvent) =>
    setBox(Math.floor(e.nativeEvent.layout.width));
  const height =
    Math.max(580, Math.min(900, winHeight - 180 * displayScale)) / displayScale;
  const graphHeight = height;
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
    <View nativeID="intel-topology-graph" style={t.graphCol}>
      <View
        nativeID="intel-topology-viewport"
        role="group"
        aria-label={summary}
        onLayout={onLayout}
        style={[t.canvasBox, { height }]}
      >
        <View
          nativeID="intel-topology-grid"
          pointerEvents="none"
          style={StyleSheet.absoluteFill}
        />
        {client && box > 0 && nodes.length > 0 ? (
          <View
            nativeID="intel-topology-engine"
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              right: 0,
              height: graphHeight,
            }}
          >
            <ErrorBoundary label="Topology graph" onRetry={reset}>
              <Suspense fallback={<CanvasSkeleton />}>
                <ForceCanvas
                  ref={canvas}
                  nodes={gNodes}
                  links={gLinks}
                  width={box}
                  height={graphHeight}
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
                  onView={view.report}
                  onSelect={choose}
                />
              </Suspense>
            </ErrorBoundary>
          </View>
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
      <View nativeID="intel-topology-neighborhood" style={t.neighborhoodHud}>
        <View style={t.controlGroup}>
          <View style={t.hudHeading}>
            <Text style={t.hudTitle}>◈ NEIGHBORHOOD</Text>
            <Text style={t.hudDegree}>DEG: {depth}</Text>
          </View>
          <View style={t.hudRow} role="group" aria-label="Neighbor depth">
            <HudButton
              label="1st DEG"
              active={depth === 1 && !!selected}
              disabled={!selected}
              onPress={() => {
                explicitSecond.current = false;
                setDepth(1);
              }}
            >
              1st DEG
            </HudButton>
            <HudButton
              label="2nd DEG"
              active={depth === 2}
              disabled={!selected}
              onPress={() => {
                explicitSecond.current = true;
                setDepth(2);
              }}
            >
              2nd DEG
            </HudButton>
          </View>
        </View>
        <View style={[t.controlGroup, t.kindControls]}>
          <Text style={t.hudTitle}>NODE TYPE FILTER</Text>
          <View style={t.hudKinds} role="group" aria-label="Node kinds">
            {kinds.map((kind) => (
              <HudButton
                key={kind}
                label={`Show ${kind} nodes`}
                active={!hiddenKinds.includes(kind)}
                onPress={() =>
                  setHiddenKinds(
                    hiddenKinds.includes(kind)
                      ? hiddenKinds.filter((item) => item !== kind)
                      : [...hiddenKinds, kind],
                  )
                }
              >
                <View style={t.kindLabel}>
                  <KindIcon kind={kind} />
                  <Text style={[t.hudButtonText, { color: kindColor[kind] }]}>
                    {kind.toUpperCase()}
                  </Text>
                </View>
              </HudButton>
            ))}
          </View>
        </View>
        <ViewTelemetry store={view} />
      </View>
      <View style={t.controls} role="group" aria-label="Graph legend">
        {kinds.map((kind) => (
          <View key={kind} style={t.kindLabel}>
            <KindIcon kind={kind} />
            <Text style={[t.hudTitle, { color: kindColor[kind] }]}>
              {kind.toUpperCase()}
            </Text>
          </View>
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
      <InspectorPresence visible={!!selected}>
        {selected ? (
          <View
            key={selected.id}
            nativeID="intel-topology-inspector"
            style={{ gap: 8 }}
          >
            <View style={t.kindLabel}>
              <KindIcon kind={selected.kind} />
              <Text style={[t.hudTitle, { color: kindColor[selected.kind] }]}>
                {selected.kind.toUpperCase()}
              </Text>
            </View>
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
                      onPress={() => choose(it.node.id)}
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
      </InspectorPresence>
    </Panel>
  );

  const nodeList = (
    <Panel title="Nodes">
      <ScrollView
        nativeID="intel-topology-node-scroll"
        style={t.listBox}
        nestedScrollEnabled
      >
        <View role="list" aria-label="Topology nodes">
          {nodes.map((n) => {
            const on = n.id === selected?.id;
            return (
              <View role="listitem" key={n.id} nativeID={`intel-node-${n.id}`}>
                <NodeListRow
                  node={n}
                  dotColor={kindColor[n.kind] ?? c.accent}
                  on={on}
                  onPress={() => choose(n.id)}
                />
              </View>
            );
          })}
        </View>
        <Label>Connections</Label>
        {links.length === 0 ? (
          <Copy>No connections match the current filters.</Copy>
        ) : (
          <View role="list" aria-label="Topology connections">
            {links.map((edge) => (
              <View role="listitem" key={edge.id} style={t.edgeRow}>
                <Copy>
                  {byId.get(edge.from)?.label || edge.from} → {edge.label} →{" "}
                  {byId.get(edge.to)?.label || edge.to}
                </Copy>
              </View>
            ))}
          </View>
        )}
      </ScrollView>
    </Panel>
  );

  return (
    <View style={{ gap: 12, minWidth: 0 }}>
      <View
        nativeID="intel-topology-layout"
        style={[t.layout, wide && t.layoutWide]}
      >
        {graphArea}
        <View
          nativeID="intel-topology-side"
          style={[t.sideCol, wide && t.sideColWide]}
        >
          {details}
        </View>
      </View>
      <View nativeID="intel-topology-nodes" style={t.nodesSection}>
        {nodeList}
      </View>
    </View>
  );
}

function KindIcon({ kind }: { kind: string }) {
  return (
    <Image
      accessibilityLabel={`${kind} icon`}
      source={{
        uri: `data:image/svg+xml;utf8,${kindIconSvg(kind, kindHue(kind), "24")}`,
      }}
      style={{ width: 14, height: 14 }}
      resizeMode="contain"
    />
  );
}

type GraphView = { yaw: number; zoom: number };
function createViewStore() {
  let snapshot: GraphView = { yaw: 0, zoom: 100 };
  const listeners = new Set<() => void>();
  return {
    getSnapshot: () => snapshot,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    report: (next: GraphView) => {
      if (snapshot.yaw === next.yaw && snapshot.zoom === next.zoom) return;
      snapshot = next;
      listeners.forEach((listener) => listener());
    },
  };
}
function ViewTelemetry({
  store,
}: {
  store: ReturnType<typeof createViewStore>;
}) {
  const view = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  return (
    <View style={t.hudTelemetry}>
      <Text style={t.hudMetric}>YAW: {view.yaw.toFixed(2)} rad</Text>
      <Text style={t.hudMetric}>ZOOM: {view.zoom}%</Text>
    </View>
  );
}

function HudButton({
  children,
  label,
  active = false,
  disabled = false,
  onPress,
}: {
  children: React.ReactNode;
  label: string;
  active?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled, selected: active }}
      aria-pressed={active}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        t.hudButton,
        active && t.hudButtonActive,
        disabled && { opacity: 0.4 },
        pressed && { opacity: 0.7 },
      ]}
    >
      {typeof children === "string" ? (
        <Text
          numberOfLines={1}
          style={[t.hudButtonText, active && { color: topologyColors.accent }]}
        >
          {children}
        </Text>
      ) : (
        children
      )}
    </Pressable>
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
  kindLabel: { flexDirection: "row", alignItems: "center", gap: 6 },
  controls: { flexDirection: "row", flexWrap: "wrap", gap: 8, minWidth: 0 },
  layout: { gap: 16, minWidth: 0 },
  layoutWide: { flexDirection: "row-reverse", alignItems: "flex-start" },
  graphCol: { flex: 1, minWidth: 0, gap: 12 },
  nodesSection: { width: "100%", minWidth: 0 },
  sideCol: { width: "100%", minWidth: 0, gap: 12 },
  sideColWide: { width: 320, flexShrink: 0 },
  canvasBox: {
    width: "100%",
    minWidth: 0,
    backgroundColor: topologyColors.bg,
    borderColor: intelColors.border,
    borderWidth: 1,
    borderRadius: 12,
    overflow: "hidden",
    alignItems: "center",
    justifyContent: "center",
  },
  neighborhoodHud: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "flex-end",
    gap: 16,
    padding: 16,
    backgroundColor: topologyColors.panel,
    borderColor: intelColors.border,
    borderWidth: 1,
    borderRadius: 10,
  },
  controlGroup: { gap: 8, minWidth: 0 },
  kindControls: { flexGrow: 1, flexShrink: 1 },
  hudHeading: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  hudTitle: { fontFamily: fonts.mono, fontSize: 12, color: intelColors.dim },
  hudDegree: {
    fontFamily: fonts.mono,
    fontSize: 12,
    color: topologyColors.accent,
  },
  hudRow: { flexDirection: "row", gap: 6 },
  hudKinds: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  hudButton: {
    minHeight: 32,
    minWidth: 0,
    paddingHorizontal: 12,
    paddingVertical: 5,
    borderRadius: 3,
    borderWidth: 1,
    borderColor: intelColors.border,
    backgroundColor: intelColors.inset,
    alignItems: "center",
    justifyContent: "center",
  },
  hudButtonActive: {
    borderColor: topologyColors.accent,
    backgroundColor: intelColors.accentSoft,
  },
  hudButtonText: {
    fontFamily: fonts.mono,
    fontSize: 12,
    color: intelColors.dim,
  },
  hudTelemetry: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
    paddingVertical: 8,
  },
  hudMetric: { fontFamily: fonts.mono, fontSize: 11, color: intelColors.muted },
  overlay: { position: "absolute", alignItems: "center", gap: 8, padding: 12 },
  count: { fontFamily: fonts.mono, fontSize: 12, color: c.dim },
  headline: { fontSize: 19, lineHeight: 28, fontWeight: "600", color: c.text },
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

function CanvasSkeleton({ label = "Loading graph…" }: { label?: string }) {
  return (
    <View
      nativeID="intel-topology-skeleton"
      role="status"
      accessibilityLabel={label}
      style={{ padding: 32, gap: 32 }}
    >
      <Skeleton width="50%" height={12} />
      <View style={{ flexDirection: "row", justifyContent: "space-around" }}>
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} width={40} height={40} radius={20} />
        ))}
      </View>
      <Skeleton width="80%" height={12} />
    </View>
  );
}
