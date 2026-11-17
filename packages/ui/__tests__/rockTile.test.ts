import { describe, expect, it } from "vitest";
import {
  ROCK_TILE,
  rockChecksum,
  rockShards,
  rockTileImage,
} from "../src/rockTile";
import { rockTextureStyle, textureOpacity } from "../src";

describe("fractured-obsidian tile", () => {
  it("matches the checksum pinned by apps/fleetworks-desktop/src/rock.rs", () => {
    expect(rockShards()).toHaveLength(96);
    expect(rockChecksum()).toBe(2914555029);
  });

  it("has zero mean tone, bounded tones and 12-28 px fragments", () => {
    const shards = rockShards();
    const sum = shards.reduce((acc, s) => acc + s.tone, 0);
    expect(Math.abs(sum)).toBeLessThan(1e-9);
    for (const shard of shards) {
      expect(Math.abs(shard.tone)).toBeGreaterThanOrEqual(0.3);
      expect(Math.abs(shard.tone)).toBeLessThanOrEqual(1);
      const xs = shard.points.map((p) => p[0]);
      const width = Math.max(...xs) - Math.min(...xs);
      expect(width).toBeGreaterThanOrEqual(12);
      expect(width).toBeLessThanOrEqual(28);
    }
  });

  it("is anchored at the top-left at a fixed scale for every state", () => {
    const rest = rockTextureStyle(textureOpacity.rest);
    expect(rest.backgroundPosition).toBe("0 0");
    expect(rest.backgroundSize).toBe(`${ROCK_TILE}px 90px`);
    expect(rest.backgroundRepeat).toBe("repeat-x");
  });

  it("draws fainter relief when disabled than at rest, and caches each opacity", () => {
    const alphas = (image: string) =>
      [...decodeURIComponent(image).matchAll(/fill-opacity="([\d.]+)"/g)].map(
        (m) => Number(m[1]),
      );
    const rest = alphas(rockTileImage(textureOpacity.rest));
    const off = alphas(rockTileImage(textureOpacity.disabled));
    expect(rest).toHaveLength(96);
    expect(Math.max(...rest)).toBeGreaterThan(Math.max(...off));
    expect(Math.max(...rest)).toBeLessThan(0.2);
    expect(rockTileImage(textureOpacity.rest)).toBe(rockTileImage(0.12));
  });
});
