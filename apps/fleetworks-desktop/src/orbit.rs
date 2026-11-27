//! Quiet Orbit: the native mark over the shared `#060709` canvas.
//!
//! The mark is a 144 × 176 envelope with a fractured obsidian crescent and a smoky
//! glass sphere. The core uses the approved texture, cropped by the asset tooling
//! and uploaded as raw pixels. Two texture layers drift at the reference pace. The
//! crescent, rim and envelope stay fixed while the core moves and the mark docks.
//! See `docs/design/matte-canvas/motion.md` for the startup and handoff timings.

use std::f32::consts::TAU;

use std::sync::OnceLock;

use rune::canvas;
use rune::el;
use rune::El;
use rune::LineCap;
use rune::Shape;

use crate::rock;
use crate::theme;

/// Scene envelope in scene units; the element scales it uniformly.
pub(crate) const ENVELOPE: [f32; 2] = [144.0, 176.0];
/// One core texture cycle at the sixteen-second motion reference pace.
pub(crate) const LOOP_SECONDS: f32 = 16.0;
/// The moment the static, reduced-motion pose is taken from.
const STILL_SECONDS: f32 = 1.5;

const CORE: [f32; 2] = [72.0, 61.0];
const CORE_RADIUS: f32 = 40.0;
const CRESCENT_CENTER: [f32; 2] = [72.0, 105.0];
const CRESCENT_RADIUS: f32 = 62.0;
const CUT_CENTER: [f32; 2] = [72.0, 69.0];
const CUT_RADIUS: f32 = 50.0;
const ARC_STEPS: usize = 40;

/// Faint inclusions inside the crescent glass, widely spaced.
const FLECKS: [[f32; 2]; 5] = [
    [72.0, 142.0],
    [44.0, 149.0],
    [96.0, 135.0],
    [110.0, 143.0],
    [62.0, 159.0],
];
const FLECK_RADIUS: f32 = 1.0;
/// Grid cell of the crescent's obsidian facets, in scene units: coarse, angular fragments.
const FACET_CELL: f32 = 10.0;
/// Facets stay this far inside the glass edge.
const FACET_MARGIN: f32 = 1.5;
/// One fragment of the crescent's coarse fractured obsidian.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facet {
    pub points: [[f32; 2]; 3],
    /// 0 darkest, 1 lightest.
    pub tone: f32,
    /// A few facets catch a faint edge light.
    pub lit: bool,
}

/// Whether `point` is inside the crescent glass by `margin`.
pub(crate) fn inside_crescent(point: [f32; 2], margin: f32) -> bool {
    (point[0] - CRESCENT_CENTER[0]).hypot(point[1] - CRESCENT_CENTER[1]) + margin < CRESCENT_RADIUS
        && (point[0] - CUT_CENTER[0]).hypot(point[1] - CUT_CENTER[1]) - margin > CUT_RADIUS
}

/// The facets: a jittered grid cut into triangles, keeping those wholly inside the crescent. The
/// hash is [`rock::noise`], so the pattern is the same on every run.
pub(crate) fn facets() -> &'static [Facet] {
    static CACHE: OnceLock<Vec<Facet>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let vertex = |i: u32, j: u32| {
            let jitter = |salt| (rock::noise(i, j, salt) % 1000) as f32 / 1000.0 - 0.5;
            [
                8.0 + i as f32 * FACET_CELL + jitter(21) * FACET_CELL * 0.6,
                40.0 + j as f32 * FACET_CELL + jitter(22) * FACET_CELL * 0.6,
            ]
        };
        let mut out = Vec::new();
        for j in 0..13 {
            for i in 0..14 {
                let (a, b) = (vertex(i, j), vertex(i + 1, j));
                let (c, d) = (vertex(i + 1, j + 1), vertex(i, j + 1));
                let halves = if rock::noise(i, j, 23) & 1 == 0 {
                    [[a, b, c], [a, c, d]]
                } else {
                    [[a, b, d], [b, c, d]]
                };
                for (n, points) in halves.into_iter().enumerate() {
                    if points.iter().all(|p| inside_crescent(*p, FACET_MARGIN)) {
                        out.push(Facet {
                            points,
                            tone: (rock::noise(i, j, 24 + n as u32) % 1000) as f32 / 1000.0,
                            lit: rock::noise(i, j, 26).is_multiple_of(3),
                        });
                    }
                }
            }
        }
        out
    })
}

/// The core's photographic texture: the approved smoky glass sphere with its molten ivory and
/// amber flow, cropped from the approved source by `assets/brand/source/derive_core_texture.py`
/// (384 x 384 opaque RGBA, covering [`CORE_TEXTURE_MARGIN`] sphere diameters). It is drawn as two
/// layers that drift against each other inside a circle, so the flow moves without the baked GIF.
static CORE_RGBA: &[u8] =
    include_bytes!("../../../assets/brand/orbit/fleetworks-core-texture.rgba");
const CORE_TEXTURE_SIZE: u32 = 384;
/// The texture covers this multiple of the sphere diameter.
const CORE_TEXTURE_MARGIN: f32 = 1.12;
/// Drift of the front layer, as a fraction of the sphere diameter. The margin keeps every
/// corner of the source image outside the circle at this amplitude.
const CORE_DRIFT: f32 = 0.035;

thread_local! {
    static CORE_PIXELS: rune::Pixels = {
        let pixels = rune::Pixels::new();
        pixels.set(CORE_TEXTURE_SIZE, CORE_TEXTURE_SIZE, CORE_RGBA.to_vec());
        pixels
    };
}

/// Where the two core layers sit `seconds` into the loop, as fractions of the sphere diameter
/// from the centred rest position: `[front, back]`, each `[x, y]`. Both are whole turns of the
/// sixteen-second loop, so the flow repeats seamlessly, and the layers move in opposite senses at
/// different rates so the texture shears like liquid.
pub(crate) fn core_drift(seconds: f32) -> [[f32; 2]; 2] {
    let turn = phase(seconds);
    [
        [CORE_DRIFT * turn.sin(), CORE_DRIFT * turn.cos()],
        [
            -0.8 * CORE_DRIFT * (2.0 * turn).sin(),
            0.8 * CORE_DRIFT * (2.0 * turn + 1.0).cos(),
        ],
    ]
}

fn circle(center: [f32; 2], radius: f32, steps: usize) -> Vec<[f32; 2]> {
    (0..steps)
        .map(|i| {
            let angle = TAU * i as f32 / steps as f32;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

fn arc(center: [f32; 2], radius: f32, from: f32, to: f32, steps: usize) -> Vec<[f32; 2]> {
    (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

/// The crescent: the outer disc minus the cut disc, as one simple polygon through the bottom.
pub(crate) fn crescent() -> Vec<[f32; 2]> {
    let d = CRESCENT_CENTER[1] - CUT_CENTER[1];
    let a = (d * d + CRESCENT_RADIUS * CRESCENT_RADIUS - CUT_RADIUS * CUT_RADIUS) / (2.0 * d);
    let h = (CRESCENT_RADIUS * CRESCENT_RADIUS - a * a).sqrt();
    let tip_y = CRESCENT_CENTER[1] - a;
    let mut points = arc(
        CRESCENT_CENTER,
        CRESCENT_RADIUS,
        (-a).atan2(h),
        (-a).atan2(-h) + TAU,
        ARC_STEPS,
    );
    let inner = arc(
        CUT_CENTER,
        CUT_RADIUS,
        (tip_y - CUT_CENTER[1]).atan2(-h) + TAU,
        (tip_y - CUT_CENTER[1]).atan2(h),
        ARC_STEPS,
    );
    // The inner arc's end points repeat the tips.
    points.extend_from_slice(&inner[1..inner.len() - 1]);
    points
}

/// Core texture phase in radians at `seconds` into the loop.
fn phase(seconds: f32) -> f32 {
    TAU * (seconds / LOOP_SECONDS).fract()
}

/// Pixel size of the whole envelope inside a window, per the owner's layout rule: padding
/// `clamp(0.04 * min(W, H), 16, 32)`, target height `min(0.66 * (H - 2P), 440)`, one uniform
/// scale. `window` is `None` before the first layout; a roomy default is used then.
pub(crate) fn display_size(window: Option<[f32; 2]>) -> [f32; 2] {
    let [width, height] = window.unwrap_or([1180.0, 780.0]);
    let pad = (0.04 * width.min(height)).clamp(16.0, 32.0);
    let target_height = (0.66 * (height - 2.0 * pad)).min(440.0);
    let scale = ((width - 2.0 * pad) / ENVELOPE[0])
        .min(target_height / ENVELOPE[1])
        .max(0.0);
    [ENVELOPE[0] * scale, ENVELOPE[1] * scale]
}

/// The stationary vector layers below and above the moving core texture.
fn layers() -> (Shape, Shape) {
    let mut back = Shape::new();
    // Obsidian crescent: coarse fractured facets under a polished glass edge, faint flecks.
    back = back.fill(crescent(), theme::hex("#0A0C10"));
    for facet in facets() {
        let color = theme::hex("#07090C").mix(theme::hex("#323A46"), facet.tone.powf(2.0));
        back = back.fill(facet.points.to_vec(), color).stroke_with(
            facet.points.to_vec(),
            true,
            0.35,
            LineCap::Butt,
            theme::hex("#04060A").alpha(0.7),
        );
        if facet.lit {
            back = back.line(
                facet.points[0],
                facet.points[1],
                0.5,
                LineCap::Butt,
                theme::hex("#AEB8C8").alpha(0.38),
            );
        }
    }
    back = back.stroke_with(
        crescent(),
        true,
        1.0,
        LineCap::Butt,
        theme::hex("#A8B2C0").alpha(0.62),
    );
    for (n, fleck) in FLECKS.iter().enumerate() {
        let (color, alpha) = if n < 3 {
            ("#F0DDA8", 0.60)
        } else {
            ("#C9D2E0", 0.35)
        };
        back = back.fill(
            circle(*fleck, FLECK_RADIUS, 6),
            theme::hex(color).alpha(alpha),
        );
    }

    // Over the sphere's photographic body: only a faint rim line, since the texture already
    // carries the glass edge, its reflections and the molten flow.
    let front = Shape::new().stroke_with(
        circle(CORE, CORE_RADIUS - 0.5, 72),
        true,
        0.8,
        LineCap::Butt,
        theme::hex("#CFC5AA").alpha(0.30),
    );
    (
        back.view_box(ENVELOPE[0], ENVELOPE[1]),
        front.view_box(ENVELOPE[0], ENVELOPE[1]),
    )
}

/// Draw the crescent, core and thin glass rim in a uniformly scaled envelope.
fn stack(seconds: f32, width: f32, height: f32) -> El {
    let (back, front) = layers();
    let k = width / ENVELOPE[0];
    let diameter = 2.0 * CORE_RADIUS * k;
    let tile = CORE_PIXELS.with(rune::Pixels::clone);
    // Two copies of the photographic sphere inside a circle: the front one drifts, the back one
    // drifts against it at a different rate and shows through at half strength.
    let image = diameter * CORE_TEXTURE_MARGIN;
    let inset = (diameter - image) / 2.0;
    let [front_drift, back_drift] = core_drift(seconds);
    let layer = |drift: [f32; 2], opacity: f32, key: &'static str| {
        el().key(key)
            .absolute(inset + drift[0] * diameter, inset + drift[1] * diameter)
            .size(image, image)
            .pixels(&tile)
            .opacity(opacity)
            .a11y_hidden()
    };
    let body = el()
        .key("orbit-core")
        .absolute((CORE[0] - CORE_RADIUS) * k, (CORE[1] - CORE_RADIUS) * k)
        .size(diameter, diameter)
        .radius(diameter / 2.0)
        .clip_rounded(diameter / 2.0)
        .fill(theme::hex("#0B0907"))
        .child(layer(back_drift, 1.0, "orbit-core-back"))
        .child(layer(front_drift, 0.55, "orbit-core-front"))
        .a11y_hidden();
    el().w(width)
        .h(height)
        .child(
            canvas(back)
                .key("orbit-crescent")
                .absolute(0.0, 0.0)
                .size(width, height)
                .a11y_hidden(),
        )
        .child(body)
        .child(
            canvas(front)
                .key("orbit-core-rim")
                .absolute(0.0, 0.0)
                .size(width, height)
                .a11y_hidden(),
        )
}

/// The crescent and core at rest, `height` points tall, for chrome such as the sidebar.
pub(crate) fn mark(height: f32) -> El {
    stack(STILL_SECONDS, height * ENVELOPE[0] / ENVELOPE[1], height)
        .key("brand-mark")
        .label("Fleetworks")
}

fn handoff_pose(seconds: f32, settle: f32) -> f32 {
    seconds * (1.0 - settle) + STILL_SECONDS * settle
}

/// Draw the selected pose in flight and settle the core texture into its rail pose.
pub(crate) fn handoff(seconds: f32, settle: f32, width: f32, height: f32) -> El {
    stack(handoff_pose(seconds, settle), width, height)
}

/// The mark in the chooser or setup. Reduced Motion uses the same still as the rail.
pub(crate) fn view(seconds: f32, animated: bool, width: f32, height: f32) -> El {
    let seconds = if animated { seconds } else { STILL_SECONDS };
    stack(seconds, width, height)
        .key("startup-orbit")
        .label("Fleetworks")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two loops at 60 Hz also cover the wrap.
    fn samples() -> impl Iterator<Item = f32> {
        (0..=(2.0 * LOOP_SECONDS * 60.0) as usize).map(|i| i as f32 / 60.0)
    }

    #[test]
    fn crescent_is_a_bounded_polygon_that_leaves_the_sphere_clear() {
        let polygon = crescent();
        assert!(polygon.len() > 40);
        for [x, y] in &polygon {
            assert!((0.0..=ENVELOPE[0]).contains(x) && (0.0..=ENVELOPE[1]).contains(y));
            assert!((x - CORE[0]).hypot(y - CORE[1]) > CORE_RADIUS);
        }
        let lowest = polygon.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!((lowest - (CRESCENT_CENTER[1] + CRESCENT_RADIUS)).abs() < 0.5);
    }

    #[test]
    fn the_foreground_contains_only_the_core_rim() {
        let (crescent, foreground) = layers();
        assert!(!crescent.is_empty());
        let rim = Shape::new()
            .stroke_with(
                circle(CORE, CORE_RADIUS - 0.5, 72),
                true,
                0.8,
                LineCap::Butt,
                theme::hex("#CFC5AA").alpha(0.30),
            )
            .view_box(ENVELOPE[0], ENVELOPE[1]);
        assert_eq!(foreground, rim);
    }

    #[test]
    fn the_handoff_preserves_the_selected_core_pose_until_settling_into_the_rail() {
        for seconds in [0.0, STILL_SECONDS, 3.0] {
            assert_eq!(handoff_pose(seconds, 0.0), seconds);
            assert_eq!(handoff_pose(seconds, 1.0), STILL_SECONDS);
        }
        assert_ne!(handoff_pose(3.0, 0.5), STILL_SECONDS);
    }

    #[test]
    fn rock_and_flecks_sit_inside_the_crescent_glass() {
        for fleck in FLECKS {
            assert!(inside_crescent(fleck, FLECK_RADIUS), "{fleck:?}");
        }
        assert!(facets().len() > 60, "{} facets", facets().len());
        for facet in facets() {
            for vertex in facet.points {
                assert!(inside_crescent(vertex, FACET_MARGIN - 1e-3), "{vertex:?}");
            }
            assert!((0.0..=1.0).contains(&facet.tone));
        }
        // Coarse fragments of mixed tone, a few with an edge light, the same on every call.
        assert!(facets().iter().any(|f| f.lit) && facets().iter().any(|f| !f.lit));
        let tones: Vec<f32> = facets().iter().map(|f| f.tone).collect();
        assert!(tones.iter().cloned().fold(f32::MIN, f32::max) > 0.8);
        assert!(tones.iter().cloned().fold(f32::MAX, f32::min) < 0.2);
        assert_eq!(facets().len(), facets().len());
    }

    #[test]
    fn the_core_layers_drift_loop_and_never_expose_the_texture_corners() {
        let at_start = core_drift(0.0);
        assert_ne!(at_start, core_drift(2.0), "the flow moves");
        // The loop is seamless: the drift at the loop's end is the drift at its start.
        let wrapped = core_drift(LOOP_SECONDS);
        for (a, b) in at_start.iter().zip(wrapped.iter()) {
            assert!((a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
        }
        // The two layers move differently, so the texture shears instead of sliding.
        assert_ne!(core_drift(1.0)[0], core_drift(1.0)[1]);
        // At the largest shift the circle (radius 0.5 diameter) stays inside the texture
        // (half width 0.5 * margin diameters), so the source's corners never show.
        let mut farthest = 0.0_f32;
        for seconds in samples() {
            for [x, y] in core_drift(seconds) {
                farthest = farthest.max(x.hypot(y));
            }
        }
        assert!(
            0.5 + farthest < 0.5 * CORE_TEXTURE_MARGIN,
            "reach {}",
            0.5 + farthest
        );
    }

    #[test]
    fn the_core_texture_is_the_committed_approved_sphere() {
        let size = CORE_TEXTURE_SIZE as usize;
        assert_eq!(CORE_RGBA.len(), size * size * 4);
        let mut hash = 0x811C_9DC5_u32;
        for byte in CORE_RGBA {
            hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
        }
        // The same hash is recorded in assets/brand/orbit/fleetworks-core-texture.manifest.json.
        assert_eq!(hash, 1_864_096_320);
        assert!(CORE_RGBA.chunks(4).all(|px| px[3] == 255), "opaque");
        let luma = |x: usize, y: usize| {
            let at = (y * size + x) * 4;
            0.2126 * f32::from(CORE_RGBA[at])
                + 0.7152 * f32::from(CORE_RGBA[at + 1])
                + 0.0722 * f32::from(CORE_RGBA[at + 2])
        };
        let mean = |x0: usize, y0: usize, n: usize| {
            let mut sum = 0.0;
            for y in y0..y0 + n {
                for x in x0..x0 + n {
                    sum += luma(x, y);
                }
            }
            sum / (n * n) as f32
        };
        // A lit molten sphere in the middle, dark space and glass at the edges.
        let centre = mean(size / 2 - 40, size / 2 - 40, 80);
        let edge = mean(0, size / 2 - 40, 8);
        assert!(centre > 3.0 * edge.max(1.0), "centre {centre}, edge {edge}");
    }

    #[test]
    fn display_size_contains_the_envelope_and_scales_uniformly() {
        let ratio = ENVELOPE[0] / ENVELOPE[1];
        for window in [
            [640.0, 480.0],
            [1600.0, 900.0],
            [320.0, 568.0],
            [1180.0, 780.0],
        ] {
            let [w, h] = display_size(Some(window));
            assert!(w > 0.0 && h > 0.0);
            assert!((w / h - ratio).abs() < 1e-3);
            assert!(w <= window[0] && h <= window[1], "{window:?} -> {w}x{h}");
            assert!(h <= 440.0 + 1e-3);
        }
        let [small, _] = display_size(Some([320.0, 568.0]));
        let [large, _] = display_size(Some([1600.0, 900.0]));
        assert!(small < large);
        assert_eq!(display_size(None), display_size(Some([1180.0, 780.0])));
    }

    #[test]
    fn timing_matches_the_playback_manifest() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/brand/reference/quiet-orbit/playback.json"
        ))
        .expect("playback manifest");
        assert_eq!(
            manifest["reference"]["seconds"].as_f64(),
            Some(f64::from(LOOP_SECONDS))
        );
        let startup = &manifest["startup"];
        let ms = |key: &str| startup[key].as_f64().unwrap();
        assert!((ms("duration_ms") - crate::startup::INTRO_SECONDS * 1000.0).abs() < 1e-6);
        assert!((ms("folder_fade_ms") - crate::startup::FOLDER_FADE_SECONDS * 1000.0).abs() < 1e-6);
        assert_eq!(manifest["canvas"]["color"].as_str(), Some("#060709"));
        assert_eq!(crate::theme::BACKGROUND, "#060709");
    }
}
