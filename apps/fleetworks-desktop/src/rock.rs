//! The rock tile for Submit, Confirm, Create, Delete and Connect, and the shared facet hash.
//!
//! The tile is the approved crescent's coarse rock, derived once by
//! `assets/brand/source/derive_rock_tile.py` (recipe: `docs/design/matte-canvas/theme.md`,
//! "Deterministic asset and tint recipe") and committed as raw RGBA under `assets/brand/rock/`.
//! It is a 512 x 512 overlay shown at a fixed 256 x 256 logical pixels and repeated, never
//! stretched. Nothing is generated at runtime and no image decoder is involved: the bytes go
//! straight into a Rune `Pixels` once per UI thread.

use rune::Pixels;

/// Logical size of one tile; the 512 px source is shown at half scale.
pub(crate) const TILE: f32 = 256.0;
/// Source pixels per tile side.
const SIZE: usize = 512;
/// The committed overlay: white at alpha 0.22 T where T > 0, near-black at alpha 1.25 |T| where
/// T < 0, at opacity 1. A button scales it by its state opacity.
static RGBA: &[u8] = include_bytes!("../../../assets/brand/rock/fleetworks-rock-tile.rgba");

thread_local! {
    static PIXELS: Pixels = {
        let pixels = Pixels::new();
        pixels.set(SIZE as u32, SIZE as u32, RGBA.to_vec());
        pixels
    };
}

/// The tile as Rune pixels, shared by every textured action on this thread.
pub(crate) fn pixels() -> Pixels {
    PIXELS.with(Pixels::clone)
}

/// Fixed seed for the crescent facet hash in `orbit.rs`. Changing it changes every facet.
const SEED: u32 = 0xA460_5A11;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `fnv1a32` of the committed overlay, as recorded in the asset manifest.
    const FNV1A32: u32 = 2_000_210_541;

    fn pixel(x: usize, y: usize) -> [u8; 4] {
        let at = (y * SIZE + x) * 4;
        [RGBA[at], RGBA[at + 1], RGBA[at + 2], RGBA[at + 3]]
    }

    #[test]
    fn the_overlay_is_the_committed_asset() {
        assert_eq!(RGBA.len(), SIZE * SIZE * 4);
        let mut hash = 0x811C_9DC5_u32;
        for byte in RGBA {
            hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
        }
        assert_eq!(hash, FNV1A32, "the manifest records the same hash");
    }

    #[test]
    fn relief_is_matte_balanced_and_bounded() {
        let (mut light, mut dark, mut peak) = (0.0_f64, 0.0_f64, 0.0_f64);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let [r, _, _, a] = pixel(x, y);
                let alpha = f64::from(a) / 255.0;
                if a == 0 {
                    continue;
                }
                if r > 128 {
                    light += alpha / 0.22;
                    peak = peak.max(alpha);
                } else {
                    dark += alpha / 1.25;
                }
            }
        }
        // The overlay was normalized to zero mean, so light and dark relief balance.
        let balance = (light - dark) / (SIZE * SIZE) as f64;
        assert!(balance.abs() < 0.01, "mean T is {balance}");
        // At the hover opacity the lightest pixel adds under 8 channel levels: matte, not gloss.
        assert!(peak * 0.14 * 255.0 < 8.0, "peak alpha {peak}");
    }

    #[test]
    fn the_tile_repeats_without_a_seam() {
        for i in 0..SIZE {
            assert_eq!(pixel(i, 0), pixel(i, SIZE - 1), "column {i}");
            assert_eq!(pixel(0, i), pixel(SIZE - 1, i), "row {i}");
        }
    }

    #[test]
    fn the_facet_hash_is_fixed() {
        assert_eq!(noise(3, 5, 21), noise(3, 5, 21));
        assert_ne!(noise(3, 5, 21), noise(5, 3, 21));
        assert_ne!(noise(3, 5, 21), noise(3, 5, 22));
    }
}
