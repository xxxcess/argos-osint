import { describe, expect, it } from "vitest";
import {
  assignParticles,
  classifyNeighborhood,
  particleOffset,
} from "../src/topology/neighborhood";
import { createNodeGlyph } from "../src/topology/nodeGlyph";

const nodes = ["focus", "near", "far", "out"];
const edges = [
  { id: "a", from: "focus", to: "near" },
  { id: "b", from: "near", to: "far" },
  { id: "c", from: "far", to: "out" },
];

describe("neighborhood depth", () => {
  it("shows only the target and directly connected nodes at 1st degree", () => {
    const graph = classifyNeighborhood(nodes, edges, "focus", 1);
    expect(graph.role.get("focus")).toBe("focus");
    expect(graph.role.get("near")).toBe("first");
    expect(graph.role.has("far")).toBe(false);
    expect(graph.role.has("out")).toBe(false);
    expect([...graph.drawnEdgeIds]).toEqual(["a"]);
  });

  it("draws the next hop when 2nd degree is on and stops at that hop", () => {
    const graph = classifyNeighborhood(nodes, edges, "focus", 2);
    expect(graph.role.get("far")).toBe("second");
    expect(graph.role.has("out")).toBe(false);
    expect(graph.drawnEdgeIds).toEqual(new Set(["a", "b"]));
  });

  it("returns the whole graph when focus is cleared", () => {
    const graph = classifyNeighborhood(nodes, edges, null, 0);
    expect(graph.drawnEdgeIds).toEqual(new Set(["a", "b", "c"]));
    expect([...graph.role.values()].every((role) => role === "full")).toBe(
      true,
    );
  });

  it("drops 2nd-degree dots first once the particle cap is reached", () => {
    const links = [
      { id: "focus-edge", touchesFocus: true, secondDegree: false },
      ...Array.from({ length: 300 }, (_, index) => ({
        id: `second-${index}`,
        touchesFocus: false,
        secondDegree: true,
      })),
    ];
    const counts = assignParticles(links, false, 4);
    expect(counts.get("focus-edge")).toBe(3);
    expect(
      [...counts.values()].reduce((sum, count) => sum + count, 0),
    ).toBeLessThanOrEqual(4);
    expect(assignParticles(links, true).get("focus-edge")).toBe(0);
  });

  it("offsets particle phase by edge id", () => {
    expect(particleOffset("a")).not.toBe(particleOffset("b"));
    expect(particleOffset("a")).toBe(particleOffset("a"));
  });
});

describe("node glyphs", () => {
  it("uses a filled icon for every kind and falls back when an image errors", () => {
    for (const kind of [
      "actor",
      "claim",
      "inference",
      "link",
      "location",
      "article",
    ]) {
      const glyph = createNodeGlyph({ label: kind, kind });
      expect(glyph.querySelector("path")?.getAttribute("fill")).toBeTruthy();
    }
    const image = createNodeGlyph({
      label: "Harbor article",
      kind: "article",
      imageUrl: "https://example.test/og.png",
    });
    const photo = image.querySelector("img");
    expect(photo?.getAttribute("alt")).toBe("Harbor article");
    photo?.dispatchEvent(new Event("error"));
    expect(image.querySelector("img")).toBeNull();
    expect(image.querySelector("path")?.getAttribute("fill")).toBe("#5FD39A");
  });
});
