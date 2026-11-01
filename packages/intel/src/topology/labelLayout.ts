/** Screen-space label placement inside ForceCanvas, independent of graph physics. */
export type Box = { left: number; top: number; right: number; bottom: number };
export type LabelOffset = { x: number; y: number; visible: boolean };
export function intersects(a: Box, b: Box, gap = 8): boolean {
  return (
    a.left < b.right + gap &&
    a.right > b.left - gap &&
    a.top < b.bottom + gap &&
    a.bottom > b.top - gap
  );
}
export function placeEdgeLabels(
  blockers: Box[],
  labels: Box[],
  bounds: Box,
): LabelOffset[] {
  const placed: Box[] = [...blockers];
  const offsets: [number, number][] = [];
  // Keep captions beside their connection, testing progressively wider clearances.
  for (const distance of [18, 30, 46, 64, 88, 116]) {
    offsets.push(
      [0, -distance],
      [0, distance],
      [distance, -distance],
      [-distance, distance],
      [-distance, -distance],
      [distance, distance],
      [distance, 0],
      [-distance, 0],
    );
  }
  return labels.map((label) => {
    for (const [x, y] of offsets) {
      const moved = {
        left: label.left + x,
        right: label.right + x,
        top: label.top + y,
        bottom: label.bottom + y,
      };
      if (
        moved.left < bounds.left + 8 ||
        moved.right > bounds.right - 8 ||
        moved.top < bounds.top + 8 ||
        moved.bottom > bounds.bottom - 8 ||
        placed.some((box) => intersects(moved, box))
      )
        continue;
      placed.push(moved);
      return { x, y, visible: true };
    }
    // Dense views retain the relationship in the accessible connection list.
    return { x: 0, y: 0, visible: false };
  });
}
