"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import { coreGlyph } from "./coreGlyph";
import { landingColors, topologyColors } from "../shell/intel-theme";

type Node = {
  id: string;
  fx: number;
  fy: number;
  fz: number;
};

export default function OrbitCanvas({
  reduced,
  width = 560,
  height = 320,
  coreColor = landingColors.core,
}: {
  reduced: boolean;
  width?: number;
  height?: number;
  coreColor?: string;
}) {
  const [data] = useState(() => ({
    nodes: [{ id: "core", fx: 0, fy: 0, fz: 0 }],
    links: [],
  }));
  const graph = useRef<
    import("react-force-graph-3d").ForceGraphMethods<Node> | undefined
  >(undefined);
  const glyph = useRef<ReturnType<typeof coreGlyph> | null>(null);
  const haloColor = useRef(coreColor);

  const createGlyph = useCallback(() => {
    if (!glyph.current) {
      const next = coreGlyph();
      next.setHaloColor(haloColor.current);
      glyph.current = next;
    }
    return glyph.current.group;
  }, []);

  useEffect(() => {
    const api = graph.current;
    if (!api) return;
    for (const force of ["charge", "link", "center"]) api.d3Force(force, null);
    api.cameraPosition({ x: 0, y: 100, z: 480 });
    if (reduced) {
      glyph.current?.settleHalo();
      return;
    }
    let frame = 0;
    const loop = (now: number) => {
      const elapsed = now / 1000;
      glyph.current?.pulseHalo(elapsed);
      api.cameraPosition({
        x: Math.sin(elapsed * 0.1) * 480,
        y: 100,
        z: Math.cos(elapsed * 0.1) * 480,
      });
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame);
  }, [reduced]);

  useEffect(() => {
    haloColor.current = coreColor;
    glyph.current?.setHaloColor(coreColor);
  }, [coreColor]);

  useEffect(
    () => () => {
      glyph.current?.dispose();
      glyph.current = null;
    },
    [],
  );

  return (
    <ForceGraph3D
      ref={graph}
      graphData={data}
      width={width}
      height={height}
      backgroundColor={topologyColors.bg}
      showNavInfo={false}
      enableNavigationControls={false}
      enableNodeDrag={false}
      enablePointerInteraction={false}
      d3AlphaDecay={0}
      cooldownTicks={reduced ? 0 : Infinity}
      nodeThreeObject={createGlyph}
    />
  );
}
