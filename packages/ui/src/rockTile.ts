// The deterministic fractured-obsidian tile for Submit, Confirm, Create, Delete and Connect.
//
// This is the same integer algorithm as apps/fleetworks-desktop/src/rock.rs; both tests pin the
// same checksum. No tile asset is shipped. The tile is 256 x 256 logical pixels on a periodic
// 12 x 12 grid; every vertex moves by a hashed offset of at most 3.2 px, so fragments are
// 15-28 px wide, and each cell splits into two triangles with opposite tones (mean tone zero).
// Reproduce it with the steps in docs/design/matte-canvas/theme.md.

export const ROCK_TILE = 256;
/** Rows 0-3 reach 85 px; the image is this tall and repeats along x only (shortcut: no texture below). */
export const ROCK_TILE_HEIGHT = 90;
const CELLS = 12;
const TILE_UNITS = 25_600; // hundredths of a pixel; all vertex math is integer
const JITTER_UNITS = 320;
const ROWS = 4; // rows 0-3 reach 85 px, past the tallest action button
const SEED = 0xa4605a11;

export type RockShard = {
  points: [[number, number], [number, number], [number, number]];
  tone: number;
};

// lowbias32
function mix(input: number): number {
  let x = input >>> 0;
  x ^= x >>> 16;
  x = Math.imul(x, 0x7feb352d) >>> 0;
  x ^= x >>> 15;
  x = Math.imul(x, 0x846ca68b) >>> 0;
  return (x ^ (x >>> 16)) >>> 0;
}

function noise(i: number, j: number, salt: number): number {
  return mix(
    (mix((SEED ^ Math.imul(salt, 0x9e3779b1)) >>> 0) ^ (j * 16 + i)) >>> 0,
  );
}

function vertex(i: number, j: number): [number, number] {
  const wi = i % CELLS;
  const wj = j % CELLS;
  const jitter = (salt: number) =>
    (noise(wi, wj, salt) % (2 * JITTER_UNITS + 1)) - JITTER_UNITS;
  return [
    Math.trunc((i * TILE_UNITS) / CELLS) + jitter(1),
    Math.trunc((j * TILE_UNITS) / CELLS) + jitter(2),
  ];
}

let cache: RockShard[] | null = null;

/** Every triangle of the tile's first rows, in a fixed order. */
export function rockShards(): RockShard[] {
  if (cache) return cache;
  const px = (v: [number, number]): [number, number] => [
    v[0] / 100,
    v[1] / 100,
  ];
  const out: RockShard[] = [];
  for (let j = 0; j < ROWS; j++) {
    for (let i = 0; i < CELLS; i++) {
      const p00 = px(vertex(i, j));
      const p10 = px(vertex(i + 1, j));
      const p01 = px(vertex(i, j + 1));
      const p11 = px(vertex(i + 1, j + 1));
      const [first, second]: [RockShard["points"], RockShard["points"]] =
        (noise(i, j, 3) & 1) === 0
          ? [
              [p00, p10, p11],
              [p00, p11, p01],
            ]
          : [
              [p00, p10, p01],
              [p10, p11, p01],
            ];
      const magnitude = (300 + (noise(i, j, 4) % 701)) / 1000;
      const tone = (noise(i, j, 5) & 1) === 0 ? magnitude : -magnitude;
      out.push({ points: first, tone }, { points: second, tone: -tone });
    }
  }
  cache = out;
  return out;
}

/** The checksum both implementations pin. */
export function rockChecksum(): number {
  return rockShards().reduce((acc, shard, n) => {
    const tone = Math.round(shard.tone * 1000);
    const x = Math.round(shard.points[0][0] * 100);
    return (Math.imul(acc, 31) + (tone + x + n)) >>> 0;
  }, 0);
}

const images = new Map<number, string>();

/**
 * A CSS `url(...)` of the tile at `opacity`. Lighter shards add at most about `50 * opacity`
 * channel levels and darker ones remove as many, which is matte relief, not a gradient.
 */
export function rockTileImage(opacity: number): string {
  const hit = images.get(opacity);
  if (hit) return hit;
  const polygons = rockShards()
    .map((shard) => {
      const light = shard.tone > 0;
      const alpha = Math.min(
        1,
        (light ? 0.22 : 1.25) * opacity * Math.abs(shard.tone),
      );
      const points = shard.points.map((p) => `${p[0]},${p[1]}`).join(" ");
      return `<polygon points="${points}" fill="${light ? "#D5DCE5" : "#000"}" fill-opacity="${alpha.toFixed(4)}"/>`;
    })
    .join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${ROCK_TILE}" height="${ROCK_TILE_HEIGHT}" viewBox="0 0 ${ROCK_TILE} ${ROCK_TILE_HEIGHT}">${polygons}</svg>`;
  const image = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
  images.set(opacity, image);
  return image;
}
