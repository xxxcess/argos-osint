import { describe, expect, it } from "vitest";
import { neighborhoodCamera } from "../src/topology/cameraFrame";

describe("target neighborhood framing", () => {
  it("centers an off-origin target and keeps nearby nodes within caption clearance", () => {
    const frame = neighborhoodCamera(
      [
        { id: "focus", label: "Target", kind: "claim", x: 80, y: 25, z: 0 },
        {
          id: "neighbor",
          label: "Neighbor",
          kind: "actor",
          x: -80,
          y: -20,
          z: 0,
        },
      ],
      "focus",
      320,
      600,
    )!;
    expect(frame.target).toEqual({ x: 80, y: 25, z: 0 });
    expect(frame.position.x).toBe(frame.target.x);
    expect(frame.position.y).toBe(frame.target.y);
    const visibleHalfWidth =
      (frame.distance * Math.tan((25 * Math.PI) / 180) * 320) / 600;
    expect(visibleHalfWidth * (1 - 0.44)).toBeGreaterThanOrEqual(159.99);
  });
  it("fits every first-degree neighbor without letting distant ghosts set the zoom", () => {
    const nodes = [
      { id: "focus", label: "Target", kind: "claim", x: 0, y: 0, z: 0 },
      { id: "first", label: "Neighbor", kind: "actor", x: 30, y: 20, z: 0 },
      {
        id: "ghost",
        label: "Distant context",
        kind: "inference",
        x: 3000,
        y: 2000,
        z: 0,
      },
    ];
    const nearby = neighborhoodCamera(
      nodes,
      "focus",
      760,
      580,
      50,
      new Set(["focus", "first"]),
    )!;
    const whole = neighborhoodCamera(nodes, "focus", 760, 580)!;
    expect(nearby.distance).toBeLessThan(whole.distance);
    const halfHeight = nearby.distance * Math.tan((25 * Math.PI) / 180);
    expect(halfHeight * (1 - 220 / 580)).toBeGreaterThanOrEqual(20);
  });
  it("frames the two-dimensional engine when z is omitted", () => {
    const frame = neighborhoodCamera(
      [
        { id: "focus", label: "Target", kind: "claim", x: 20, y: 10 },
        { id: "near", label: "Neighbor", kind: "actor", x: -30, y: 40 },
      ],
      "focus",
      760,
      580,
    )!;
    expect(frame.target).toEqual({ x: 20, y: 10, z: 0 });
    expect(Number.isFinite(frame.distance)).toBe(true);
  });
  it("waits for positioned nodes before animating", () => {
    expect(
      neighborhoodCamera(
        [{ id: "focus", label: "Target", kind: "claim" }],
        "focus",
        760,
        580,
      ),
    ).toBeNull();
  });
});
