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
import { createNodeGlyph } from "./nodeGlyph";
import { intelMotion, topologyColors } from "../shell/intel-theme";

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
  /** Ids that set the automatic zoom. Defaults to every lit node. */
  cameraIds?: Set<string> | null;
  reducedMotion: boolean;
  orbit?: OrbitMode;
  orbitSuspended?: boolean;
  onSelect: (id: string | null) => void;
  onView?: (view: { yaw: number; zoom: number }) => void;
  /** True while the pointer is over a node or edge, not empty graph space. */
  onItemHover?: (overItem: boolean) => void;
};
type Position = { x: number; y: number; z: number };
type GraphMethods = {
  controls: () => { target: Position; update?: () => void };
  camera?: () => {
    fov: number;
    position?: Position;
    lookAt?: (x: number, y: number, z: number) => void;
  };
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
  glyph.dataset.nodeId = node.id;
  group.add(new CSS2DObject(glyph));
  group.addEventListener("removed", () => glyph.remove());
  return group;
}

function visibleLink() {
  const group = new Group();
  const line = new Line(
    new BufferGeometry(),
    new LineDashedMaterial({
      color: topologyColors.edge,
      dashSize: 0.35,
      gapSize: 0.45,
      transparent: true,
      opacity: 0.2,
    }),
  );
  group.add(line);
  return group;
}

function positionLink(
  raw: object,
  { start, end }: { start: Position; end: Position },
) {
  const group = raw as Group;
  const line = group.children[0] as Line;
  const material = line.material as LineDashedMaterial;
  // Preserve a quiet entrance while graph geometry settles.
  const target = 0.7;
  material.opacity += (target - material.opacity) * 0.16;
  line.geometry.setFromPoints([
    new Vector3(start.x, start.y, start.z || 0),
    new Vector3(end.x, end.y, end.z || 0),
  ]);
  line.computeLineDistances();
  return true;
}

function createLabelRenderer() {
  let reducedMotion = true;
  const renderer = new CSS2DRenderer();
  const opacityCache = new Map<string, number>();
  const render = renderer.render.bind(renderer);
  renderer.render = (scene, camera) => {
    render(scene, camera);
    const root = renderer.domElement;
    for (const glyph of root.querySelectorAll<HTMLElement>(
      ".intel-topology-node",
    )) {
      const id = glyph.dataset.nodeId ?? "";
      const target =
        glyph.dataset.role === "ghost"
          ? 0.35
          : glyph.dataset.role === "second"
            ? 0.7
            : 1;
      const previous = opacityCache.get(id) ?? 1;
      const opacity = reducedMotion
        ? target
        : previous + (target - previous) * 0.16;
      opacityCache.set(id, opacity);
      glyph.style.opacity = String(
        Math.abs(opacity - target) < 0.005 ? target : opacity,
      );
    }
  };
  renderer.domElement.style.pointerEvents = "none";
  renderer.domElement.setAttribute("aria-hidden", "true");
  return {
    renderers: [renderer],
    setReducedMotion(value: boolean) {
      reducedMotion = value;
    },
  };
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
    cameraIds,
    reducedMotion,
    orbit = "off",
    orbitSuspended = false,
    onSelect,
    onView,
    onItemHover,
  },
  ref,
) {
  const labelRenderer = useMemo(() => createLabelRenderer(), []);
  const extraRenderers = labelRenderer.renderers;
  useEffect(() => {
    labelRenderer.setReducedMotion(reducedMotion);
  }, [labelRenderer, reducedMotion]);
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
  const overNode = useRef(false);
  const overLink = useRef(false);
  const reportHover = () =>
    onItemHover?.(overNode.current || overLink.current);
  const resumeAt = useRef(0);
  // Automatic centering is a visit/reset action. Later selections and drags keep the camera.
  const framedVisit = useRef(false);
  const zoomMs = reducedMotion ? 0 : intelMotion.fit;
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
  const centerVisit = useCallback(
    (force = false) => {
      if (framedVisit.current && !force) return;
      const graph = graphRef.current;
      if (!graph || width <= 0 || height <= 0) return;
      const currentId = selectedRef.current;
      if (currentId === null) {
        const before = graph.cameraPosition();
        graph.zoomToFit(0, width < 400 ? 50 : 80);
        const after = graph.cameraPosition();
        if (
          before.x === after.x &&
          before.y === after.y &&
          before.z === after.z
        )
          return;
        framedVisit.current = true;
        return;
      }
      const landing = neighborhoodCamera(
        data.nodes,
        currentId,
        width,
        height,
        graph.camera?.().fov ?? 50,
        cameraIds ?? focusIds,
      );
      // Warmup may not have assigned coordinates yet. The next engine stop retries.
      if (!landing) return;
      const controls = graph.controls();
      controls.target.x = landing.target.x;
      controls.target.y = landing.target.y;
      controls.target.z = landing.target.z;
      // Snap. An in-flight tween leaves TrackballControls looking past the node,
      // so the glyph sits low in the viewport.
      graph.cameraPosition(landing.position, landing.target, 0);
      const now = graph.cameraPosition();
      if (
        Math.abs(now.x - landing.position.x) > 0.5 ||
        Math.abs(now.y - landing.position.y) > 0.5
      )
        return;
      const camera = graph.camera?.();
      if (camera?.position) {
        camera.position.x = landing.position.x;
        camera.position.y = landing.position.y;
        camera.position.z = landing.position.z;
      }
      camera?.lookAt?.(landing.target.x, landing.target.y, landing.target.z);
      controls.update?.();
      framedVisit.current = true;
    },
    [data, width, height, focusIds, cameraIds],
  );
  const centerRef = useRef(centerVisit);
  useEffect(() => {
    centerRef.current = centerVisit;
  }, [centerVisit]);
  const onEngineStop = useCallback(() => {
    centerRef.current();
  }, []);
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
        // A reset is a new visit: center once the reheated layout settles.
        framedVisit.current = false;
        if (!reducedMotion) {
          graphRef.current?.d3ReheatSimulation();
          return;
        }
        centerVisit(true);
      },
      pause: () => graphRef.current?.pauseAnimation(),
      resume: () => {
        if (!reducedMotion) graphRef.current?.resumeAnimation();
      },
    }),
    [zoomMs, reducedMotion, centerVisit],
  );
  useEffect(() => {
    let fittedFrame = 0;
    const frame = requestAnimationFrame(() => {
      fittedFrame = requestAnimationFrame(() => centerVisit());
    });
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(fittedFrame);
    };
  }, [centerVisit]);
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
  }, [labelRenderer]);
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
      onEngineStop={onEngineStop}
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
      linkHoverPrecision={8}
      onLinkHover={(link) => {
        overLink.current = !!link;
        reportHover();
      }}
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
      onNodeHover={(node) => {
        overNode.current = !!node;
        reportHover();
        engage();
      }}
      onNodeDrag={() => engage()}
      onBackgroundClick={() => onSelect(null)}
    />
  );
});
export default ForceCanvas;
