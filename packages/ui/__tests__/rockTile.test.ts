import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { ROCK_TILE, rockTileImage } from "../src/rockTile";
import { ROCK_TILE_DATA_URI } from "../src/rockTileData";
import { rockTextureStyle, textureOpacity } from "../src";

const root = join(__dirname, "../../..");
const manifest = JSON.parse(
  readFileSync(
    join(root, "assets/brand/rock/fleetworks-rock-tile.manifest.json"),
    "utf8",
  ),
);

describe("approved crescent rock tile", () => {
  it("embeds exactly the committed PNG that the manifest records", () => {
    const prefix = "data:image/png;base64,";
    expect(ROCK_TILE_DATA_URI.startsWith(prefix)).toBe(true);
    const bytes = Buffer.from(
      ROCK_TILE_DATA_URI.slice(prefix.length),
      "base64",
    );
    const committed = readFileSync(
      join(root, "assets/brand/rock/fleetworks-rock-tile.png"),
    );
    expect(bytes.equals(committed)).toBe(true);
    const recorded = manifest.outputs["fleetworks-rock-tile.png"];
    expect(bytes.length).toBe(recorded.bytes);
    expect(createHash("sha256").update(bytes).digest("hex")).toBe(
      recorded.sha256,
    );
  });

  it("is a 512 px overlay shown at 256 logical px, from the approved source", () => {
    expect(manifest.tile_size_px).toBe(512);
    expect(manifest.displayed_logical_px).toBe(ROCK_TILE);
    expect(manifest.source.sha256).toBe(
      "1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112",
    );
    // The native overlay is the same pixels: one 512 x 512 RGBA image.
    const rgba = readFileSync(
      join(root, "assets/brand/rock/fleetworks-rock-tile.rgba"),
    );
    expect(rgba.length).toBe(512 * 512 * 4);
    expect(
      Math.abs(manifest.normalization.mean_T_after_quantization),
    ).toBeLessThan(0.01);
  });

  it("is anchored top-left at a fixed scale and repeated on both axes", () => {
    expect(rockTextureStyle.backgroundImage).toBe(rockTileImage);
    expect(rockTextureStyle.backgroundPosition).toBe("0 0");
    expect(rockTextureStyle.backgroundSize).toBe(
      `${ROCK_TILE}px ${ROCK_TILE}px`,
    );
    expect(rockTextureStyle.backgroundRepeat).toBe("repeat");
  });

  it("keeps the state opacities of the recipe, faintest when disabled", () => {
    expect(textureOpacity).toEqual({
      rest: 0.12,
      hover: 0.14,
      pressed: 0.1,
      disabled: 0.04,
    });
  });
});
