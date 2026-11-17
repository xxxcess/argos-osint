//! The deterministic fractured-obsidian tile for Submit, Confirm, Create, Delete and Connect.
//!
//! No tile asset is shipped, so the tile is generated from integers only. The web build uses the
//! same algorithm in `packages/ui/src/rockTile.ts`; both are pinned to the same checksum in their
//! tests. Reproduce it with the steps in `docs/design/matte-canvas/theme.md`.
//!
//! The tile is 256 x 256 logical pixels on a periodic 12 x 12 grid. Each grid vertex moves by a
//! hashed offset of at most 3.2 px, so fragments are 15-28 px wide. Each grid cell splits into
//! two triangles; the two carry opposite tones, so the mean tone is exactly zero.

/// Logical size of the repeating tile.
pub(crate) const TILE: f32 = 256.0;
/// Grid cells per tile side.
const CELLS: u32 = 12;
/// Tile size in hundredths of a pixel; all vertex math is integer.
const TILE_UNITS: i32 = 25_600;
/// Largest vertex offset in hundredths of a pixel.
const JITTER_UNITS: i32 = 320;
/// Cell rows generated from the top. Rows 0-3 reach 85 px, past the tallest action button.
pub(crate) const ROWS: u32 = 4;
/// Fixed seed. Changing it changes every tile, so it is part of the asset manifest.
const SEED: u32 = 0xA460_5A11;

/// One triangle of the tile: three points in logical pixels and a tone in [-1, 1].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shard {
    pub(crate) points: [[f32; 2]; 3],
    pub(crate) tone: f32,
}

/// The `lowbias32` integer hash.
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A hash of grid position `(i, j)` for one purpose `salt`. Also seeds the crescent facets.
pub(crate) fn noise(i: u32, j: u32, salt: u32) -> u32 {
    mix(mix(SEED ^ salt.wrapping_mul(0x9E37_79B1)) ^ (j * 16 + i))
}

/// Vertex `(i, j)` in hundredths of a pixel. Index `CELLS` wraps to 0, so the tile repeats
/// without a seam.
fn vertex(i: u32, j: u32) -> [i32; 2] {
    let (wi, wj) = (i % CELLS, j % CELLS);
    let jitter = |salt| (noise(wi, wj, salt) % (2 * JITTER_UNITS as u32 + 1)) as i32 - JITTER_UNITS;
    [
        (i as i32) * TILE_UNITS / CELLS as i32 + jitter(1),
        (j as i32) * TILE_UNITS / CELLS as i32 + jitter(2),
    ]
}

/// Every triangle of the tile's first [`ROWS`] cell rows, in a fixed order.
pub(crate) fn shards() -> &'static [Shard] {
    static CACHE: std::sync::OnceLock<Vec<Shard>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let px = |v: [i32; 2]| [v[0] as f32 / 100.0, v[1] as f32 / 100.0];
        let mut out = Vec::with_capacity((CELLS * ROWS * 2) as usize);
        for j in 0..ROWS {
            for i in 0..CELLS {
                let (p00, p10) = (px(vertex(i, j)), px(vertex(i + 1, j)));
                let (p01, p11) = (px(vertex(i, j + 1)), px(vertex(i + 1, j + 1)));
                let (first, second) = if noise(i, j, 3) & 1 == 0 {
                    ([p00, p10, p11], [p00, p11, p01])
                } else {
                    ([p00, p10, p01], [p10, p11, p01])
                };
                let magnitude = (300 + noise(i, j, 4) % 701) as f32 / 1000.0;
                let tone = if noise(i, j, 5) & 1 == 0 {
                    magnitude
                } else {
                    -magnitude
                };
                out.push(Shard {
                    points: first,
                    tone,
                });
                out.push(Shard {
                    points: second,
                    tone: -tone,
                });
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKSUM: u32 = 2_914_555_029;

    fn width(shard: &Shard) -> f32 {
        let xs = shard.points.map(|p| p[0]);
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
    }

    #[test]
    fn the_tile_is_the_same_every_time_and_matches_the_web_checksum() {
        assert_eq!(shards(), shards());
        assert_eq!(shards().len(), 96);
        // Mirrored in packages/ui/__tests__/rockTile.test.ts.
        let checksum = shards().iter().enumerate().fold(0u32, |acc, (n, s)| {
            let tone = (s.tone * 1000.0).round() as i32;
            let x = (s.points[0][0] * 100.0).round() as i32;
            acc.wrapping_mul(31)
                .wrapping_add((tone + x + n as i32) as u32)
        });
        assert_eq!(checksum, CHECKSUM);
    }

    #[test]
    fn tones_are_bounded_and_average_to_zero() {
        let sum: f32 = shards().iter().map(|s| s.tone).sum();
        assert!(sum.abs() < 1e-3, "{sum}");
        assert!(shards().iter().all(|s| (0.3..=1.0).contains(&s.tone.abs())));
    }

    #[test]
    fn fragments_are_between_twelve_and_twenty_eight_pixels_wide() {
        for (n, shard) in shards().iter().enumerate() {
            let w = width(shard);
            // Every triangle has a vertex in each of two adjacent columns, 21.3 px apart, and
            // each vertex moves by at most 3.2 px.
            assert!((12.0..=28.0).contains(&w), "shard {n} is {w} px wide");
        }
    }

    #[test]
    fn the_tile_repeats_without_a_seam() {
        assert_eq!(vertex(0, 0)[0] + TILE_UNITS, vertex(CELLS, 0)[0]);
        assert_eq!(vertex(0, 0)[1], vertex(CELLS, 0)[1]);
    }
}
