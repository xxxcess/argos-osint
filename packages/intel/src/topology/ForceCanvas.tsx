"use client";
import React, {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import ForceGraph3D from "react-force-graph-3d";

export type GNode = {
  id: string;
  label: string;
  kind: string;
  x?: number;
  y?: number;
  z?: number;
};
export type GLink = {
  id: string;
  source: string;
  target: string;
  label: string;
};
export type CanvasHandle = {
  zoomBy: (factor: number) => void;
  fit: () => void;
  reset: () => void;
  pause: () => void;
  resume: () => void;
};
type Props = {
  nodes: GNode[];
  links: GLink[];
  width: number;
  height: number;
  kindColor: Record<string, string>;
  palette: { text: string; dim: string; bg: string; accent: string };
  selectedId: string | null;
  /** Ids to keep fully lit; null means nothing is dimmed. */
  focusIds: Set<string> | null;
  reducedMotion: boolean;
  onSelect: (id: string | null) => void;
};
type Position = { x: number; y: number; z: number };
type GraphMethods = {
  controls: () => { target: Position };
  cameraPosition: (
    position?: Position,
    lookAt?: Position,
    ms?: number,
  ) => Position;
  zoomToFit: (ms?: number, padding?: number) => void;
  pauseAnimation: () => void;
  resumeAnimation: () => void;
  d3ReheatSimulation: () => void;
};

// ForceGraph tooltips interpret strings as HTML; entity text is untrusted.
export function escapeGraphLabel(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (char) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        char
      ]!,
  );
}

const ForceCanvas = forwardRef<CanvasHandle, Props>(function ForceCanvas(
  {
    nodes,
    links,
    width,
    height,
    kindColor,
    palette,
    selectedId,
    focusIds,
    reducedMotion,
    onSelect,
  },
  ref,
) {
  const graphRef = useRef<GraphMethods | undefined>(undefined);
  // D3 mutates endpoints and positions. Query cache objects never enter the engine.
  const data = useMemo(
    () => ({
      nodes: nodes.map((n) => ({ ...n })),
      links: links.map((l) => ({ ...l })),
    }),
    [nodes, links],
  );
  const fittedData = useRef<typeof data | null>(null);
  const ms = reducedMotion ? 0 : 400;
  useImperativeHandle(
    ref,
    () => ({
      zoomBy: (factor) => {
        const graph = graphRef.current;
        if (!graph) return;
        const position = graph.cameraPosition();
        const target = graph.controls().target;
        graph.cameraPosition(
          {
            x: target.x + (position.x - target.x) / factor,
            y: target.y + (position.y - target.y) / factor,
            z: target.z + (position.z - target.z) / factor,
          },
          { x: target.x, y: target.y, z: target.z },
          ms,
        );
      },
      fit: () => graphRef.current?.zoomToFit(ms, 40),
      reset: () => {
        if (!reducedMotion) graphRef.current?.d3ReheatSimulation();
        graphRef.current?.zoomToFit(ms, 40);
      },
      pause: () => graphRef.current?.pauseAnimation(),
      resume: () => {
        if (!reducedMotion) graphRef.current?.resumeAnimation();
      },
    }),
    [ms, reducedMotion],
  );
  useEffect(() => {
    if (!selectedId) return;
    const node = data.nodes.find((n) => n.id === selectedId);
    if (node?.x == null || node.y == null || node.z == null) return;
    const distance = Math.hypot(node.x, node.y, node.z);
    const scale = distance > 0 ? 1 + 200 / distance : 1;
    graphRef.current?.cameraPosition(
      {
        x: node.x * scale,
        y: node.y * scale,
        z: distance > 0 ? node.z * scale : 200,
      },
      { x: node.x, y: node.y, z: node.z },
      ms,
    );
  }, [selectedId, data, ms]);
  return (
    <ForceGraph3D
      ref={graphRef as never}
      graphData={data}
      width={width}
      height={height}
      backgroundColor={palette.bg}
      showNavInfo={false}
      onEngineStop={() => {
        if (fittedData.current === data) return;
        fittedData.current = data;
        graphRef.current?.zoomToFit(0, 60);
      }}
      linkWidth={1.5}
      linkOpacity={0.85}
      warmupTicks={reducedMotion ? 120 : 0}
      cooldownTicks={reducedMotion ? 0 : 120}
      enableNodeDrag={!reducedMotion}
      nodeLabel={(n) =>
        escapeGraphLabel(`${(n as GNode).label} · ${(n as GNode).kind}`)
      }
      nodeColor={(n) =>
        focusIds && !focusIds.has((n as GNode).id)
          ? palette.dim
          : (kindColor[(n as GNode).kind] ?? palette.accent)
      }
      nodeRelSize={1.5}
      nodeVal={(n) => ((n as GNode).id === selectedId ? 4 : 2)}
      nodeOpacity={0.9}
      linkLabel={(l) => escapeGraphLabel((l as unknown as GLink).label)}
      linkColor={(l) => {
        const edge = l as unknown as {
          source: GNode | string;
          target: GNode | string;
        };
        const source =
          typeof edge.source === "object" ? edge.source.id : edge.source;
        const target =
          typeof edge.target === "object" ? edge.target.id : edge.target;
        return !focusIds || (focusIds.has(source) && focusIds.has(target))
          ? palette.dim
          : palette.bg;
      }}
      linkDirectionalArrowLength={4}
      linkDirectionalArrowRelPos={1}
      onNodeClick={(n) => onSelect((n as GNode).id)}
      onBackgroundClick={() => onSelect(null)}
    />
  );
});
export default ForceCanvas;
