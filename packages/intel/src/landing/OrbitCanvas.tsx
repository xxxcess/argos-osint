"use client";
import React, { useEffect, useRef, useMemo } from "react";
import ForceGraph3D from "react-force-graph-3d";
import {
  Group,
  Mesh,
  Sprite,
  Line,
  BufferGeometry,
  LineBasicMaterial,
  Vector3,
  MeshBasicMaterial,
} from "three";
import { coreGlyph } from "./coreGlyph";
import { spawnOrbiter, stepOrbiter } from "./orbit";
import { landingColors, topologyColors } from "../shell/intel-theme";
type Node = {
  id: string;
  x: number;
  y: number;
  z: number;
  fx: number;
  fy: number;
  fz: number;
};
const seedFor = (id: string) =>
  [...id].reduce(
    (hash, char) => Math.imul(hash, 31) + char.charCodeAt(0),
    17,
  ) >>> 0;
export default function OrbitCanvas({
  reduced,
  onComplete,
  checks = [],
  ready = false,
  width = 560,
  height = 320,
  coreColor = landingColors.core,
}: {
  reduced: boolean;
  onComplete?: () => void;
  checks?: string[];
  ready?: boolean;
  width?: number;
  height?: number;
  coreColor?: string;
}) {
  const checkKey = JSON.stringify(checks);
  const data = useMemo(() => {
    const ids = JSON.parse(checkKey) as string[];
    const nodes: Node[] = [
      { id: "core", x: 0, y: 0, z: 0, fx: 0, fy: 0, fz: 0 },
      ...ids.map((id) => {
        const o = stepOrbiter(spawnOrbiter(seedFor(id)), reduced ? 6 : 0);
        return { id, x: o.x, y: o.y, z: o.z, fx: o.x, fy: o.y, fz: o.z };
      }),
    ];
    return {
      nodes,
      links: nodes
        .slice(1)
        .map((node) => ({ source: "core", target: node.id })),
    };
  }, [checkKey, reduced]);
  const births = useRef(new Map<string, number>());
  const graph = useRef<
    import("react-force-graph-3d").ForceGraphMethods<Node> | undefined
  >(undefined);
  const glyphs = useRef(new Map<string, Group>());
  const linkMaterials = useRef(new Map<string, LineBasicMaterial>());
  useEffect(() => {
    const api = graph.current;
    if (!api) return;
    for (const force of ["charge", "link", "center"]) api.d3Force(force, null);
    api.cameraPosition({ x: 0, y: 100, z: 480 });
    const now = performance.now();
    let added = 0;
    for (const node of data.nodes.slice(1))
      if (!births.current.has(node.id)) {
        births.current.set(node.id, now + added * 250);
        added++;
      }
    for (const id of births.current.keys())
      if (!data.nodes.some((node) => node.id === id)) births.current.delete(id);
    if (reduced) {
      for (let i = 1; i < data.nodes.length; i++) {
        const o = stepOrbiter(spawnOrbiter(seedFor(data.nodes[i].id)), 6);
        Object.assign(data.nodes[i], {
          x: o.x,
          y: o.y,
          z: o.z,
          fx: o.x,
          fy: o.y,
          fz: o.z,
        });
        const glyph = glyphs.current.get(data.nodes[i].id);
        if (glyph) {
          glyph.visible = true;
          glyph.position.set(o.x, o.y, o.z);
        }
      }
      for (const material of linkMaterials.current.values())
        material.opacity = 0.25;
      return;
    }
    let frame = 0;
    let completed = false;
    const loop = (now: number) => {
      const elapsed = now / 1000;
      let arrived = data.nodes.length > 1;

      for (let i = 1; i < data.nodes.length; i++) {
        const node = data.nodes[i];
        const born = births.current.get(node.id) ?? now;
        const age = Math.max(0, (now - born) / 1000);
        if (age < 4) arrived = false;
        const material = linkMaterials.current.get(node.id);
        if (material)
          material.opacity = Math.min(1, Math.max(0, (age - 2) / 2)) * 0.25;
        const o = stepOrbiter(spawnOrbiter(seedFor(node.id)), age);
        Object.assign(node, {
          x: o.x,
          y: o.y,
          z: o.z,
          fx: o.x,
          fy: o.y,
          fz: o.z,
        });
        const glyph = glyphs.current.get(node.id);
        if (glyph) {
          glyph.visible = now >= born;
          glyph.position.set(o.x, o.y, o.z);
        }
      }
      if (!completed && ready && arrived) {
        completed = true;
        onComplete?.();
      }
      const halo = glyphs.current.get("core")?.children[1] as
        | Sprite
        | undefined;
      if (halo) {
        halo.scale.setScalar(90 + Math.sin((elapsed * Math.PI * 2) / 2.4) * 12);
        halo.material.opacity =
          0.5 + Math.sin((elapsed * Math.PI * 2) / 2.4) * 0.12;
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
  }, [data, reduced, onComplete, ready]);
  useEffect(() => {
    const group = glyphs.current.get("core");
    if (!group) return;
    const sphere = group.children[0] as Mesh;
    (sphere.material as MeshBasicMaterial).color.set(coreColor);
    const halo = group.children[1] as Sprite | undefined;
    halo?.material.color.set(coreColor);
  }, [coreColor]);
  useEffect(() => {
    const objects = glyphs.current;
    const materials = linkMaterials.current;
    return () => {
      for (const group of objects.values())
        group.traverse((obj) => {
          if (obj instanceof Mesh) {
            obj.geometry.dispose();
            const materials = Array.isArray(obj.material)
              ? obj.material
              : [obj.material];
            materials.forEach((m) => m.dispose());
          }
          if (obj instanceof Sprite) {
            obj.material.map?.dispose();
            obj.material.dispose();
          }
        });
      objects.clear();
      for (const material of materials.values()) material.dispose();
      materials.clear();
    };
  }, []);
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
      nodeThreeObject={(raw) => {
        const n = raw as Node;
        const glyph = coreGlyph(
          n.id === "core",
          data.nodes.findIndex((node) => node.id === n.id) % 2 === 0,
          coreColor,
        );
        glyph.visible =
          n.id === "core" ||
          reduced ||
          performance.now() >= (births.current.get(n.id) ?? Infinity);
        glyphs.current.set(n.id, glyph);
        return glyph;
      }}
      linkThreeObject={(raw) => {
        const link = raw as { target: Node | string };
        const id =
          typeof link.target === "string" ? link.target : link.target.id;
        const material = new LineBasicMaterial({
          color: landingColors.link,
          transparent: true,
          opacity: reduced ? 0.25 : 0,
        });
        linkMaterials.current.set(id, material);
        return new Line(new BufferGeometry(), material);
      }}
      linkPositionUpdate={(object, { start, end }) => {
        (object as Line).geometry.setFromPoints([
          new Vector3(start.x, start.y, start.z),
          new Vector3(end.x, end.y, end.z),
        ]);
        return true;
      }}
      linkWidth={0}
    />
  );
}
