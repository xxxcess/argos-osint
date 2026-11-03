import type { GNode } from "./ForceCanvas";

/** Center the target while leaving pixel clearance for every visible caption. */
export function neighborhoodCamera(
  nodes: GNode[],
  selectedId: string | null,
  width: number,
  height: number,
  fov = 50,
  contextIds?: Set<string> | null,
) {
  const focus = nodes.find((node) => node.id === selectedId);
  if (focus?.x == null || focus.y == null || width <= 0 || height <= 0)
    return null;
  const target = { x: focus.x, y: focus.y, z: focus.z ?? 0 };
  const positioned = nodes.filter(
    (node) =>
      node.x != null &&
      node.y != null &&
      (!contextIds || contextIds.has(node.id) || node.id === selectedId),
  );
  const padding = Math.min(110, width * 0.22, height * 0.22);
  const tangent = Math.tan((fov * Math.PI) / 360);
  const x = Math.max(
    0,
    ...positioned.map((node) => Math.abs(node.x! - target.x)),
  );
  const y = Math.max(
    0,
    ...positioned.map((node) => Math.abs(node.y! - target.y)),
  );
  const z = Math.max(
    0,
    ...positioned.map((node) => Math.abs((node.z ?? 0) - target.z)),
  );
  const distance =
    Math.max(
      32,
      ((x / tangent) * height) / (width - padding * 2),
      ((y / tangent) * height) / (height - padding * 2),
    ) + z;
  return {
    target,
    position: { x: target.x, y: target.y, z: target.z + distance },
    distance,
  };
}
