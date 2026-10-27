"use client";
import React, {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import ForceGraph2D from "react-force-graph-2d";

export type GNode = {
  id: string;
  label: string;
  kind: string;
  x?: number;
  y?: number;
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
type Graph2DMethods = {
  zoom: (k?: number, ms?: number) => number | void;
  zoomToFit: (ms?: number, padding?: number) => void;
  centerAt: (x?: number, y?: number, ms?: number) => void;
  pauseAnimation: () => void;
  resumeAnimation: () => void;
  d3ReheatSimulation: () => void;
};

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
  const graphRef = useRef<Graph2DMethods | undefined>(undefined);
  // The library mutates nodes/links (x, y, link endpoints); always hand it clones.
  const data = useMemo(
    () => ({
      nodes: nodes.map((n) => ({ ...n })),
      links: links.map((l) => ({ ...l })),
    }),
    [nodes, links],
  );
  const ms = reducedMotion ? 0 : 400;
  useImperativeHandle(
    ref,
    () => ({
      zoomBy: (f) => {
        const g = graphRef.current;
        if (g) g.zoom((g.zoom() as number) * f, ms);
      },
      fit: () => graphRef.current?.zoomToFit(ms, 40),
      reset: () => {
        graphRef.current?.d3ReheatSimulation();
        graphRef.current?.zoomToFit(ms, 40);
      },
      pause: () => graphRef.current?.pauseAnimation(),
      resume: () => graphRef.current?.resumeAnimation(),
    }),
    [ms],
  );
  useEffect(() => {
    if (!selectedId) return;
    const n = data.nodes.find((x) => x.id === selectedId);
    if (n && n.x != null && n.y != null)
      graphRef.current?.centerAt(n.x, n.y, ms);
  }, [selectedId, data, ms]);

  return (
    <ForceGraph2D
      ref={graphRef as never}
      graphData={data}
      width={width}
      height={height}
      backgroundColor="rgba(0,0,0,0)"
      warmupTicks={reducedMotion ? 20 : 0}
      cooldownTicks={reducedMotion ? 30 : 120}
      d3AlphaDecay={reducedMotion ? 0.1 : 0.0228}
      enableNodeDrag={!reducedMotion}
      nodeLabel={(n) => `${(n as GNode).label} · ${(n as GNode).kind}`}
      linkColor={(l) => {
        const link = l as unknown as {
          source: GNode | string;
          target: GNode | string;
        };
        const sid =
          typeof link.source === "object" ? link.source.id : link.source;
        const tid =
          typeof link.target === "object" ? link.target.id : link.target;
        const lit = !focusIds || (focusIds.has(sid) && focusIds.has(tid));
        return lit ? "rgba(181,181,181,0.55)" : "rgba(83,83,83,0.2)";
      }}
      linkDirectionalArrowLength={4}
      linkDirectionalArrowRelPos={1}
      linkCanvasObjectMode={() => "after"}
      linkCanvasObject={(l, ctx, scale) => {
        if (scale < 1.6) return;
        const link = l as unknown as {
          label: string;
          source: GNode;
          target: GNode;
        };
        const { source: a, target: b } = link;
        if (typeof a !== "object" || a.x == null || b.x == null) return;
        if (focusIds && !(focusIds.has(a.id) && focusIds.has(b.id))) return;
        ctx.font = `${10 / scale}px sans-serif`;
        ctx.fillStyle = palette.dim;
        ctx.textAlign = "center";
        ctx.fillText(
          link.label,
          (a.x + b.x!) / 2,
          ((a.y ?? 0) + (b.y ?? 0)) / 2,
        );
      }}
      nodeCanvasObject={(n, ctx, scale) => {
        const node = n as GNode;
        const x = node.x ?? 0;
        const y = node.y ?? 0;
        const color = kindColor[node.kind] ?? palette.accent;
        const dim = focusIds != null && !focusIds.has(node.id);
        const selected = node.id === selectedId;
        ctx.globalAlpha = dim ? 0.2 : 1;
        if (selected) {
          ctx.shadowColor = color;
          ctx.shadowBlur = 16;
          ctx.beginPath();
          ctx.arc(x, y, 8, 0, 2 * Math.PI);
          ctx.strokeStyle = palette.text;
          ctx.lineWidth = 2 / scale;
          ctx.stroke();
        }
        ctx.beginPath();
        ctx.arc(x, y, 5, 0, 2 * Math.PI);
        ctx.fillStyle = color;
        ctx.fill();
        ctx.shadowBlur = 0;
        if (scale > 0.8 || selected) {
          ctx.font = `${(selected ? 12 : 10) / scale}px sans-serif`;
          ctx.textAlign = "center";
          ctx.textBaseline = "top";
          ctx.fillStyle = palette.text;
          ctx.fillText(node.label, x, y + 8);
        }
        ctx.globalAlpha = 1;
      }}
      nodePointerAreaPaint={(n, color, ctx) => {
        const node = n as GNode;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(node.x ?? 0, node.y ?? 0, 9, 0, 2 * Math.PI);
        ctx.fill();
      }}
      onNodeClick={(n) => onSelect((n as GNode).id)}
      onBackgroundClick={() => onSelect(null)}
    />
  );
});
export default ForceCanvas;
