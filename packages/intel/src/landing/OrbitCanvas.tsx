"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import { Mesh, Sprite } from "three";
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
      next.halo?.material.color.set(haloColor.current);
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
      const halo = glyph.current?.halo;
      if (halo) {
        halo.scale.setScalar(90);
        halo.material.opacity = 0.55;
      }
      return;
    }
    let frame = 0;
    const loop = (now: number) => {
      const elapsed = now / 1000;
      const halo = glyph.current?.halo;
      if (halo) {
        const pulse = Math.sin((elapsed * Math.PI * 2) / 2.4);
        halo.scale.setScalar(90 + pulse * 12);
        halo.material.opacity = 0.5 + pulse * 0.12;
      }
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
    glyph.current?.halo?.material.color.set(coreColor);
  }, [coreColor]);

  useEffect(
    () => () => {
      glyph.current?.group.traverse((object) => {
        if (object instanceof Mesh) {
          object.geometry.dispose();
          const materials = Array.isArray(object.material)
            ? object.material
            : [object.material];
          materials.forEach((material) => material.dispose());
        }
        if (object instanceof Sprite) {
          object.material.map?.dispose();
          object.material.dispose();
        }
      });
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
