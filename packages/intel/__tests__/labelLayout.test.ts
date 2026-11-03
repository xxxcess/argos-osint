import { describe, expect, it } from "vitest";
import {
  intersects,
  placeEdgeLabels,
  type Box,
} from "../src/topology/labelLayout";

describe("relationship label clearance", () => {
  it("keeps labels away from node captions, each other and viewport edges", () => {
    const nodes: Box[] = [
      { left: 200, top: 200, right: 360, bottom: 230 },
      { left: 380, top: 260, right: 520, bottom: 290 },
    ];
    const labels: Box[] = [
      { left: 270, top: 210, right: 350, bottom: 230 },
      { left: 300, top: 210, right: 390, bottom: 230 },
      { left: 400, top: 270, right: 480, bottom: 290 },
    ];
    const bounds = { left: 0, top: 0, right: 760, bottom: 580 };
    const offsets = placeEdgeLabels(nodes, labels, bounds);
    const moved = labels.map((label, index) => ({
      left: label.left + offsets[index].x,
      right: label.right + offsets[index].x,
      top: label.top + offsets[index].y,
      bottom: label.bottom + offsets[index].y,
    }));
    expect(offsets.every((offset) => offset.visible)).toBe(true);
    for (let i = 0; i < moved.length; i++) {
      expect(nodes.some((node) => intersects(moved[i], node))).toBe(false);
      expect(
        moved.slice(0, i).some((label) => intersects(moved[i], label)),
      ).toBe(false);
      expect(moved[i].left).toBeGreaterThanOrEqual(8);
      expect(moved[i].right).toBeLessThanOrEqual(752);
    }
  });
  it("defers a caption to the list twin when no clear placement fits", () => {
    expect(
      placeEdgeLabels(
        [{ left: 0, top: 0, right: 100, bottom: 100 }],
        [{ left: 20, top: 20, right: 80, bottom: 40 }],
        { left: 0, top: 0, right: 100, bottom: 100 },
      )[0].visible,
    ).toBe(false);
  });
});
