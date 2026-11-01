"use client";
import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import ForceGraph3D from "react-force-graph-3d";
import {
  BufferGeometry,
  Group,
  Line,
  LineDashedMaterial,
  Mesh,
  MeshBasicMaterial,
  SphereGeometry,
  Vector3,
} from "three";
import {
  CSS2DObject,
  CSS2DRenderer,
} from "three/examples/jsm/renderers/CSS2DRenderer.js";
import { assignParticles, particleOffset } from "./neighborhood";
import { neighborhoodCamera } from "./cameraFrame";
import { placeEdgeLabels } from "./labelLayout";
import { createNodeGlyph } from "./nodeGlyph";
import { topologyColors } from "../shell/intel-theme";

export type NodeRole = "focus" | "first" | "second" | "ghost" | "full";
export type GNode = {
  id: string;
  label: string;
  kind: string;
  role?: NodeRole;
  imageUrl?: string | null;
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
export type OrbitMode = "off" | "auto" | "manual";
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
  orbit?: OrbitMode;
  orbitSuspended?: boolean;
  onSelect: (id: string | null) => void;
  onView?: (view: { yaw: number; zoom: number }) => void;
};
type Position = { x: number; y: number; z: number };
type GraphMethods = {
  controls: () => { target: Position };
  camera?: () => { fov: number };
  cameraPosition: (
    position?: Position,
    lookAt?: Position,
    ms?: number,
  ) => Position;
  zoomToFit: (ms?: number, padding?: number) => void;
  pauseAnimation: () => void;
  resumeAnimation: () => void;
  d3ReheatSimulation: () => void;
  scene?: () => {
    add: (obj: object) => void;
    getObjectByName?: (name: string) => object | undefined;
  };
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

function endpointId(value: GNode | string): string {
  return typeof value === "object" ? value.id : value;
}

function visibleNode(node: GNode) {
  const group = new Group();
  // Invisible geometry retains the canvas raycast/drag target for DOM tiles.
  group.add(
    new Mesh(
      new SphereGeometry(node.role === "focus" ? 7 : 5),
      new MeshBasicMaterial({
        transparent: true,
        opacity: 0,
        depthWrite: false,
      }),
    ),
  );
  const glyph = createNodeGlyph(node);
  group.add(new CSS2DObject(glyph));
  group.addEventListener("removed", () => glyph.remove());
  return group;
}

function visibleLink(raw: object) {
  const link = raw as GLink;
  const group = new Group();
  const line = new Line(
    new BufferGeometry(),
    new LineDashedMaterial({
      color: topologyColors.edge,
      dashSize: 0.35,
      gapSize: 0.45,
      transparent: true,
      opacity: 0.7,
    }),
  );
  group.add(line);
  const label = document.createElement("div");
  label.className = "intel-topology-edge-label";
  label.textContent = link.label;
  const anchor = document.createElement("div");
  anchor.appendChild(label);
  group.add(new CSS2DObject(anchor));
  group.addEventListener("removed", () => anchor.remove());
  return group;
}

function positionLink(
  raw: object,
  { start, end }: { start: Position; end: Position },
) {
  const group = raw as Group;
  const line = group.children[0] as Line;
  line.geometry.setFromPoints([
    new Vector3(start.x, start.y, start.z || 0),
    new Vector3(end.x, end.y, end.z || 0),
  ]);
  line.computeLineDistances();
  group.children[1].position.set(
    (start.x + end.x) / 2,
    (start.y + end.y) / 2,
    ((start.z || 0) + (end.z || 0)) / 2,
  );
  return true;
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
    orbit = "off",
    orbitSuspended = false,
    onSelect,
    onView,
  },
  ref,
) {
  const extraRenderers = useMemo(() => {
    const renderer = new CSS2DRenderer();
    const render = renderer.render.bind(renderer);
    let layoutFrame = 0;
    renderer.render = (scene, camera) => {
      render(scene, camera);
      layoutFrame = (layoutFrame + 1) % 6;
      if (layoutFrame !== 1) return;
      const root = renderer.domElement;
      const labels = [
        ...root.querySelectorAll<HTMLElement>(".intel-topology-edge-label"),
      ];
      for (const label of labels) label.style.transform = "";
      const blockers = [
        ...root.querySelectorAll<HTMLElement>(
          "[data-tile=node], .intel-topology-node-label",
        ),
      ].map((element) => element.getBoundingClientRect());
      const hud = root
        .closest("#intel-topology-viewport")
        ?.querySelector("#intel-topology-neighborhood");
      if (hud) blockers.push(hud.getBoundingClientRect());
      const offsets = placeEdgeLabels(
        blockers,
        labels.map((label) => label.getBoundingClientRect()),
        root.getBoundingClientRect(),
      );
      // Rectangles are viewport pixels; CSS transforms use the unscaled layout pixels.
      const scale = root.offsetWidth
        ? root.getBoundingClientRect().width / root.offsetWidth
        : 1;
      labels.forEach((label, index) => {
        const offset = offsets[index];
        label.style.transform = `translate(${offset.x / scale}px, ${offset.y / scale}px)`;
        label.style.opacity = offset.visible ? "1" : "0";
      });
    };
    renderer.domElement.style.pointerEvents = "none";
    renderer.domElement.setAttribute("aria-hidden", "true");
    return [renderer];
  }, []);
  useEffect(
    () => () => {
      extraRenderers[0].domElement.replaceChildren();
    },
    [extraRenderers],
  );
  const graphRef = useRef<GraphMethods | undefined>(undefined);
  // Controls and viewport telemetry must not restart d3's mutable simulation.
  // DTO values are bounded JSON; equal graph inputs retain the same engine copy.
  const graphKey = JSON.stringify({ nodes, links });
  const data = useMemo(
    () => JSON.parse(graphKey) as { nodes: GNode[]; links: GLink[] },
    [graphKey],
  );
  const dataRef = useRef(data);
  const selectedRef = useRef(selectedId);
  const suspendedRef = useRef(orbitSuspended);
  const onViewRef = useRef(onView);
  useEffect(() => {
    dataRef.current = data;
    selectedRef.current = selectedId;
    suspendedRef.current = orbitSuspended;
    onViewRef.current = onView;
  });
  const resumeAt = useRef(0);
  const framedSelection = useRef<string | null | undefined>(undefined);
  const zoomMs = reducedMotion ? 0 : 400;
  const focusMs = reducedMotion ? 0 : 700;
  const motion = reducedMotion ? "off" : orbit;
  const nodeThreeObject = useCallback(
    (raw: object) => visibleNode(raw as GNode),
    [],
  );
  const particles = useMemo(() => {
    const focus = selectedId;
    return assignParticles(
      links.map((link) => ({
        id: link.id,
        touchesFocus: link.source === focus || link.target === focus,
        secondDegree: false,
      })),
      reducedMotion,
    );
  }, [links, reducedMotion, selectedId]);
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
          zoomMs,
        );
      },
      fit: () => graphRef.current?.zoomToFit(zoomMs, 40),
      reset: () => {
        if (!reducedMotion) graphRef.current?.d3ReheatSimulation();
        graphRef.current?.zoomToFit(zoomMs, 40);
      },
      pause: () => graphRef.current?.pauseAnimation(),
      resume: () => {
        if (!reducedMotion) graphRef.current?.resumeAnimation();
      },
    }),
    [zoomMs, reducedMotion],
  );
  const frameSelection = useCallback(() => {
    // A render, resize, filter update or another engine stop must preserve
    // the user's camera. Only a different selection gets an automatic fit.
    if (framedSelection.current === selectedId) return;
    const graph = graphRef.current;
    if (!graph || width <= 0 || height <= 0) return;
    if (selectedId === null) {
      framedSelection.current = null;
      graph.zoomToFit(0, width < 400 ? 50 : 80);
      return;
    }
    const landing = neighborhoodCamera(
      data.nodes,
      selectedId,
      width,
      height,
      graph.camera?.().fov ?? 50,
      focusIds,
    );
    // Warmup may not have assigned coordinates yet. Engine stop retries this
    // pending selection, but never re-fits one that has already entered.
    if (!landing) return;
    framedSelection.current = selectedId;
    // Trackball still updates its target when navigation is disabled. Keep it
    // aligned with the explicit look-at point in reduced-motion mode as well.
    Object.assign(graph.controls().target, landing.target);
    if (!reducedMotion)
      graph.cameraPosition(
        {
          ...landing.position,
          z: landing.target.z + landing.distance * 1.25,
        },
        landing.target,
        0,
      );
    graph.cameraPosition(landing.position, landing.target, focusMs);
  }, [data, width, height, selectedId, focusIds, reducedMotion, focusMs]);
  useEffect(() => {
    let fittedFrame = 0;
    const frame = requestAnimationFrame(() => {
      fittedFrame = requestAnimationFrame(frameSelection);
    });
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(fittedFrame);
    };
  }, [frameSelection]);
  useEffect(() => {
    if (motion !== "auto") return;
    let frame = 0;
    let last = 0;
    let yaw = 0.13;
    const loop = (now: number) => {
      const dt = last ? Math.min(0.05, (now - last) / 1000) : 0;
      last = now;
      const graph = graphRef.current;
      const hold =
        suspendedRef.current || (resumeAt.current && now < resumeAt.current);
      if (graph && dt && !hold) {
        yaw += 0.15 * dt;
        const focus = dataRef.current.nodes.find(
          (node) => node.id === selectedRef.current,
        );
        const origin = {
          x: focus?.x ?? 0,
          y: focus?.y ?? 0,
          z: focus?.z ?? 0,
        };
        const distance = 240;
        const bob = Math.sin((now / 12000) * Math.PI * 2) * 0.12;
        const planar = Math.cos(0.45 + bob) * distance;
        graph.cameraPosition(
          {
            x: origin.x + Math.sin(yaw) * planar,
            y: origin.y + Math.sin(0.45 + bob) * distance,
            z: origin.z + Math.cos(yaw) * planar,
          },
          origin,
          0,
        );
      }
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame);
  }, [motion]);
  useEffect(() => {
    const timer = setInterval(() => {
      const graph = graphRef.current;
      if (!graph) return;
      const camera = graph.cameraPosition();
      const target = graph.controls().target;
      const x = camera.x - target.x;
      const y = camera.y - target.y;
      const z = camera.z - target.z;
      onViewRef.current?.({
        yaw: Math.round(Math.atan2(x, z) * 100) / 100,
        zoom: Math.round(30000 / (Math.hypot(x, y, z) || 1)),
      });
    }, 250);
    return () => clearInterval(timer);
  }, []);
  const engage = () => {
    resumeAt.current = performance.now() + 3000;
  };
  return (
    <ForceGraph3D
      ref={graphRef as never}
      graphData={data}
      extraRenderers={extraRenderers}
      numDimensions={2}
      width={width}
      height={height}
      backgroundColor="rgba(0,0,0,0)"
      showNavInfo={false}
      onEngineStop={frameSelection}
      linkThreeObject={visibleLink}
      linkPositionUpdate={positionLink}
      linkWidth={0}
      linkOpacity={0.9}
      warmupTicks={120}
      cooldownTicks={reducedMotion ? 0 : 120}
      d3AlphaDecay={0.03}
      enableNodeDrag={!reducedMotion}
      enableNavigationControls={!reducedMotion}
      nodeLabel={(n) =>
        escapeGraphLabel(`${(n as GNode).label} · ${(n as GNode).kind}`)
      }
      nodeThreeObject={nodeThreeObject}
      nodeThreeObjectExtend={false}
      nodeColor={(n) =>
        focusIds && !focusIds.has((n as GNode).id)
          ? palette.dim
          : (kindColor[(n as GNode).kind] ?? palette.accent)
      }
      nodeRelSize={6}
      nodeVal={(n) => ((n as GNode).id === selectedId ? 4 : 2)}
      nodeOpacity={0.9}
      linkLabel={(l) => escapeGraphLabel((l as unknown as GLink).label)}
      linkColor={(l) => {
        const edge = l as unknown as {
          source: GNode | string;
          target: GNode | string;
        };
        const source = endpointId(edge.source);
        const target = endpointId(edge.target);
        return !focusIds || (focusIds.has(source) && focusIds.has(target))
          ? palette.accent
          : palette.bg;
      }}
      linkDirectionalArrowLength={2}
      linkDirectionalArrowRelPos={0.85}
      linkDirectionalArrowColor={() => topologyColors.edge}
      linkDirectionalParticles={(link) =>
        particles.get((link as { id?: string }).id || "") ?? 0
      }
      linkDirectionalParticleWidth={1}
      linkDirectionalParticleColor={() => topologyColors.accent}
      linkDirectionalParticleSpeed={() => (reducedMotion ? 0 : 0.009)}
      linkDirectionalParticleOffset={(link) =>
        particleOffset(String((link as { id?: string }).id || ""))
      }
      onNodeClick={(n) => onSelect((n as GNode).id)}
      onNodeHover={() => engage()}
      onNodeDrag={() => engage()}
      onBackgroundClick={() => onSelect(null)}
    />
  );
});
export default ForceCanvas;
