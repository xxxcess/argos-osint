"use client";
import { Skeleton } from "@argos/ui";
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
import { displayEvidence } from "../views/brief/displayEvidence";
import {
  Button,
  Copy,
  Panel,
  Notice,
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
  focus,
  onSeed,
  pageStyle,
  displayScale = 1,
}: {
  seed: string | null;
  focus?: string;
  onSeed: (s: Seed, focus?: string) => void;
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
      {graph.error && (
        <>
          <Notice>{graph.error.message}</Notice>
          <Button
            disabled={graph.isFetching}
            onPress={() => {
              void graph.refetch();
            }}
          >
            Retry topology
          </Button>
        </>
      )}
      {!seed ? (
        <EmptyState title="No topology selected">
          Open an actor, claim or article from a Focus Brief.
        </EmptyState>
      ) : null}
      {seed && !graph.data && graph.isLoading ? (
        <TopologySkeleton
          label="Loading topology…"
          displayScale={displayScale}
        />
      ) : null}
      {seed && graph.data?.nodes.length === 0 ? (
        <EmptyState title="No retained connections">
          No topology records are available for this seed.
        </EmptyState>
      ) : null}
      {graph.data && graph.data.nodes.length > 0 ? (
        <GraphExplorer
          key={seed}
          data={graph.data}
          graphSeed={seed}
          focus={focus}
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
  graphSeed,
  focus,
  onSeed,
  displayScale,
}: {
  data: TopoData;
  graphSeed: string | null;
  focus?: string;
  displayScale: number;
  onSeed: (s: Seed, focus?: string) => void;
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
  const { height: winHeight } = useWindowDimensions();
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
    focus,
  );
  const [previousFocus, setPreviousFocus] = useState(focus);
  if (focus !== previousFocus) {
    setPreviousFocus(focus);
    if (focus) setSelectedId(focus);
  }
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
    label: displayEvidence(n.label),
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
    selected
      ? `Selected ${displayEvidence(selected.label)}.`
      : "No node selected."
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
    const node = id ? byId.get(id) : undefined;
    const target = node?.topology_seed;
    const canonical =
      !!target &&
      [target.Actor, target.Claim, target.Inference].some(
        (value) => typeof value === "string" && !!value,
      );
    if (id && id !== previous && canonical && target) {
      onSeed(target);
      return;
    }
    if (
      id &&
      id !== previous &&
      graphSeed?.startsWith("brief:") &&
      (node?.kind === "claim" || node?.kind === "inference")
    )
      onSeed({ Brief: graphSeed }, id);
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
  const height = graphViewportHeight(winHeight, displayScale);
  const graphHeight = height;

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
              <Suspense fallback={<GraphSquare contained />}>
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

  const nodeList = (
    <Panel title="Nodes">
      <ScrollView
        nativeID="intel-topology-node-scroll"
        accessibilityLabel="Topology nodes"
        style={t.listBox}
        contentContainerStyle={t.listContent}
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
      </ScrollView>
    </Panel>
  );

  const connectionList = (
    <Panel title="Connections">
      <ScrollView
        nativeID="intel-topology-connection-scroll"
        accessibilityLabel="Topology connections"
        style={t.listBox}
        contentContainerStyle={t.listContent}
        nestedScrollEnabled
      >
        {links.length === 0 ? (
          <Copy>No connections match the current filters.</Copy>
        ) : (
          <View role="list" aria-label="Topology connections">
            {links.map((edge) => (
              <View role="listitem" key={edge.id} style={t.edgeRow}>
                <Copy>
                  {displayEvidence(byId.get(edge.from)?.label || edge.from)} →{" "}
                  {edge.label} →{" "}
                  {displayEvidence(byId.get(edge.to)?.label || edge.to)}
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
      <View nativeID="intel-topology-layout" style={t.layout}>
        {graphArea}
      </View>
      <View nativeID="intel-topology-nodes" style={t.nodesSection}>
        {nodeList}
      </View>
      <View nativeID="intel-topology-connections" style={t.nodesSection}>
        {connectionList}
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
      accessibilityLabel={`Select node ${displayEvidence(node.label)}, ${node.kind}`}
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
        {displayEvidence(node.label)}
      </Text>
      <Text style={t.itemKind}>{node.kind}</Text>
    </Pressable>
  );
}

const t = StyleSheet.create({
  kindLabel: { flexDirection: "row", alignItems: "center", gap: 6 },
  controls: {
    width: "100%",
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    minWidth: 0,
  },
  layout: { width: "100%", gap: 16, minWidth: 0 },
  graphCol: { width: "100%", minWidth: 0, gap: 12 },
  nodesSection: { width: "100%", minWidth: 0 },
  graphSquare: {
    alignSelf: "stretch",
    borderRadius: 12,
    overflow: "hidden",
    backgroundColor: "#34343A",
  },
  graphSquarePage: {
    width: "100%",
    maxWidth: "100%",
  },
  graphSquareContained: {
    height: "100%",
    maxWidth: "100%",
    maxHeight: "100%",
  },
  sectionRect: {
    width: "100%",
    height: 120,
    borderRadius: 12,
    overflow: "hidden",
  },
  sectionRectTall: {
    width: "100%",
    height: 220,
    borderRadius: 12,
    overflow: "hidden",
  },
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
    width: "100%",
    flexDirection: "column",
    alignItems: "stretch",
    gap: 16,
    padding: 16,
    backgroundColor: topologyColors.panel,
    borderColor: intelColors.border,
    borderWidth: 1,
    borderRadius: 10,
  },
  controlGroup: { width: "100%", gap: 8, minWidth: 0 },
  kindControls: { width: "100%" },
  hudHeading: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  hudTitle: { fontFamily: fonts.mono, fontSize: 13, color: intelColors.dim },
  hudDegree: {
    fontFamily: fonts.mono,
    fontSize: 13,
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
    fontSize: 13,
    color: intelColors.dim,
  },
  hudTelemetry: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
    paddingVertical: 8,
  },
  hudMetric: { fontFamily: fonts.mono, fontSize: 12, color: intelColors.muted },
  overlay: { position: "absolute", alignItems: "center", gap: 8, padding: 12 },
  count: { fontFamily: fonts.mono, fontSize: 13, color: c.dim },
  listBox: { maxHeight: 260, minHeight: 0, minWidth: 0 },
  listContent: { paddingRight: 8, paddingBottom: 8 },
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
  itemText: { flex: 1, minWidth: 0, color: c.text, fontSize: 15 },
  itemKind: { fontFamily: fonts.mono, fontSize: 12, color: c.muted },
  edgeRow: { paddingVertical: 4 },
});

/** Layout height whose on-screen size is 75% of the browser viewport. */
function graphViewportHeight(winHeight: number, displayScale: number) {
  const scale = displayScale > 0 ? displayScale : 1;
  const measured = winHeight > 0 ? winHeight : 800 * scale;
  return (measured * 0.75) / scale;
}

function GraphSquare({
  contained = false,
  height,
}: {
  contained?: boolean;
  height?: number;
}) {
  return (
    <View
      nativeID="intel-topology-graph-skeleton"
      style={[
        t.graphSquare,
        contained ? t.graphSquareContained : t.graphSquarePage,
        !contained && height != null ? { height } : null,
      ]}
    >
      <Skeleton fill radius={12} />
    </View>
  );
}

/** Mirrors the loaded graph, neighborhood controls, inspector, and node list. */
function TopologySkeleton({
  label,
  displayScale,
}: {
  label: string;
  displayScale: number;
}) {
  const { height: winHeight } = useWindowDimensions();
  const graphHeight = graphViewportHeight(winHeight, displayScale);
  return (
    <View
      nativeID="intel-topology-skeleton"
      role="status"
      accessibilityLabel={label}
      style={{ gap: 12, minWidth: 0 }}
    >
      <View style={t.layout}>
        <View style={t.graphCol}>
          <GraphSquare height={graphHeight} />
          <View nativeID="intel-topology-controls-skeleton" style={t.sectionRect}>
            <Skeleton fill radius={12} />
          </View>
        </View>
      </View>
      <View nativeID="intel-topology-nodes-skeleton" style={t.sectionRectTall}>
        <Skeleton fill radius={12} />
      </View>
      <View
        nativeID="intel-topology-connections-skeleton"
        style={t.sectionRectTall}
      >
        <Skeleton fill radius={12} />
      </View>
    </View>
  );
}
