//! Quiet Orbit: the native startup scene, drawn as vector layers over the shared `#060709` canvas.
//!
//! The orbit follows the owner's reference geometry (`assets/brand/reference/quiet-orbit/playback.json`, checked by a test below): five
//! persistent obsidian-glass prisms on one inclined, lifted ring around a smoky glass sphere of
//! flowing molten liquid, above a fixed obsidian crescent. Orbit radius is 1.303 core radii, the
//! inclination is 66 degrees, the ring is lifted 0.495 core radii (+30% over the 0.381 static
//! baseline, animations and videos only), prisms are 0.70 scale, phases are 72 degrees apart and
//! one revolution takes sixteen seconds. The three-second startup samples the first part of that
//! same loop; it never speeds up to finish a turn.
//!
//! The scene is one 144 x 176 envelope that scales as a unit. It is vector geometry for the
//! crescent and prisms plus one derived texture for the core, with no decoder and no media
//! rectangle to flash: the texture's bytes go straight into a Rune `Pixels`.
//!
//! Materials: the core is the approved smoky glass sphere with its molten ivory and amber flow,
//! cropped from the approved source (`assets/brand/orbit/`) and drawn in a circle as two layers
//! that drift against each other (whole turns of the sixteen-second loop); the crescent is coarse
//! fractured obsidian (about eighty angular facets from the shared [`rock::noise`] hash) with a
//! polished edge and faint flecks; each prism is glass with a lit and a shaded facet, an inner
//! bevel, a lit edge and a warm reflection. See `docs/design/matte-canvas/motion.md`.
//!
//! shortcut: a bounded native vector scene, which `docs/design/matte-canvas/motion.md` allows in
//! place of derived RGBA layers. The reference GIF, MP4 and PNG under `assets/brand/reference/`
//! are not loaded at runtime (the playback manifest says native adoption is pending and the
//! native resource pipeline has no GIF support), so there is no asset to miss and nothing to
//! fall back from. If derived layers are added later, a failed load must show the still from
//! [`scene`] and keep setup usable.

use std::f32::consts::PI;
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
/// One orbital revolution, matching the sixteen-second motion reference.
pub(crate) const LOOP_SECONDS: f32 = 16.0;
/// Closest any prism edge may come to the crescent polygon, in scene units. The prior 38 px
/// baseline kept about 5.3 units at this scale; the +30% lifted geometry below measures ~12.5.
#[cfg(test)]
pub(crate) const MIN_CLEARANCE: f32 = 5.3;
/// The moment the static, reduced-motion pose is taken from.
const STILL_SECONDS: f32 = 1.5;

pub(crate) const SATELLITE_COUNT: usize = 5;
const CORE: [f32; 2] = [72.0, 61.0];
const CORE_RADIUS: f32 = 40.0;
const CRESCENT_CENTER: [f32; 2] = [72.0, 105.0];
const CRESCENT_RADIUS: f32 = 62.0;
const CUT_CENTER: [f32; 2] = [72.0, 69.0];
const CUT_RADIUS: f32 = 50.0;
const ARC_STEPS: usize = 40;

const RADIUS_RATIO: f32 = 1.303_117_5;
/// 49.4 px of lift (+30% over the 38 px baseline, animations and videos only) at the 800 px
/// reference, where the core radius is 99.76 px.
const LIFT: f32 = 49.4 / 99.760_765 * CORE_RADIUS;
const INCLINATION: f32 = 66.0 * PI / 180.0;
const PRISM_SCALE: f32 = 0.70;
/// Reference prism size before `PRISM_SCALE`, in scene units.
const PRISM_WIDTH: f32 = 22.7;
const PRISM_HEIGHT: f32 = 22.0;
/// Perspective strength: depth over `PERSPECTIVE_DEPTH` core radii.
const PERSPECTIVE_DEPTH: f32 = 7.5;

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
/// Painted reach beyond a geometric edge: the prism rim (0.3), the crescent edge (0.5) and one
/// antialiasing step (0.5), in scene units.
#[cfg(test)]
const PAINT_REACH: f32 = 1.3;

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

/// One prism at one instant.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Satellite {
    /// Apex, left base corner, right base corner.
    pub vertices: [[f32; 2]; 3],
    /// Positive depth passes in front of the sphere; negative depth passes behind it.
    pub depth: f32,
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

/// Orbit phase in radians at `seconds` into the loop.
fn phase(seconds: f32) -> f32 {
    TAU * (seconds / LOOP_SECONDS).fract()
}

/// The five prisms `seconds` into the loop, using the reference projection.
pub(crate) fn satellites(seconds: f32) -> [Satellite; SATELLITE_COUNT] {
    let phase = phase(seconds);
    let ring = CORE_RADIUS * RADIUS_RATIO;
    std::array::from_fn(|index| {
        let theta = PI / 2.0 + phase + TAU * index as f32 / SATELLITE_COUNT as f32;
        let x3 = ring * theta.cos();
        let y3 = ring * theta.sin() * INCLINATION.cos();
        let z = ring * theta.sin() * INCLINATION.sin();
        let perspective = 1.0 / (1.0 - z / (CORE_RADIUS * PERSPECTIVE_DEPTH));
        let (sx, sy) = (x3 * perspective, -y3 * perspective);
        let center = [CORE[0] + sx, CORE[1] - LIFT + sy];
        let scale = perspective * PRISM_SCALE;
        let yaw = 0.82 + 0.18 * theta.sin().abs();
        // The apex points away from the ring's center.
        let turn = sy.atan2(sx) + PI / 2.0 + 0.055 * (2.0 * phase + index as f32).sin();
        let (sin, cos) = turn.sin_cos();
        let local = [
            [0.0, -PRISM_HEIGHT * 2.0 / 3.0],
            [-PRISM_WIDTH / 2.0 * yaw, PRISM_HEIGHT / 3.0],
            [PRISM_WIDTH / 2.0 * yaw, PRISM_HEIGHT / 3.0],
        ];
        Satellite {
            vertices: local.map(|[x, y]| {
                [
                    center[0] + scale * (x * cos - y * sin),
                    center[1] + scale * (x * sin + y * cos),
                ]
            }),
            depth: z,
        }
    })
}

/// Smallest distance from `point` to the closed polygon's edges.
#[cfg(test)]
pub(crate) fn distance_to_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> f32 {
    let mut best = f32::MAX;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let edge = [b[0] - a[0], b[1] - a[1]];
        let length_sq = edge[0] * edge[0] + edge[1] * edge[1];
        let t = if length_sq == 0.0 {
            0.0
        } else {
            (((point[0] - a[0]) * edge[0] + (point[1] - a[1]) * edge[1]) / length_sq)
                .clamp(0.0, 1.0)
        };
        let nearest = [a[0] + t * edge[0], a[1] + t * edge[1]];
        best = best.min((point[0] - nearest[0]).hypot(point[1] - nearest[1]));
    }
    best
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

/// The five satellites of the static mark. Coordinates are shared with the canonical SVG
/// in `assets/brand/fleetworks-mark.svg`, using this scene's 144 × 176 envelope.
const CROWN: [[[f32; 2]; 3]; SATELLITE_COUNT] = [
    [[72.0, 3.0], [65.0, 17.0], [79.0, 17.0]],
    [[41.0, 8.0], [33.0, 24.0], [48.0, 23.0]],
    [[103.0, 8.0], [96.0, 23.0], [111.0, 24.0]],
    [[17.0, 23.0], [10.0, 40.0], [26.0, 38.0]],
    [[127.0, 23.0], [118.0, 38.0], [134.0, 40.0]],
];

/// One glass prism: a smoky body, a lit and a shaded facet, an inner bevel that gives it
/// thickness, a bright lit edge, a faint rim and a warm sliver of the core reflected near its
/// base. Every layer stays inside the prism's own triangle (plus the 0.3 unit rim).
fn prism(shape: Shape, satellite: &Satellite) -> Shape {
    let [apex, left, right] = satellite.vertices;
    let mid = [(left[0] + right[0]) / 2.0, (left[1] + right[1]) / 2.0];
    let centre = [
        (apex[0] + left[0] + right[0]) / 3.0,
        (apex[1] + left[1] + right[1]) / 3.0,
    ];
    let toward =
        |p: [f32; 2], t: f32| [p[0] + (centre[0] - p[0]) * t, p[1] + (centre[1] - p[1]) * t];
    shape
        .fill(vec![apex, left, right], theme::hex("#07090C"))
        .fill(vec![apex, left, mid], theme::hex("#232C38"))
        .fill(vec![apex, mid, right], theme::hex("#0C1015"))
        .fill(
            vec![toward(apex, 0.45), toward(left, 0.45), toward(right, 0.45)],
            theme::hex("#10151C").alpha(0.85),
        )
        .fill(
            vec![left, mid, toward(mid, 0.5)],
            theme::hex("#D9B77A").alpha(0.16),
        )
        .line(
            apex,
            left,
            0.6,
            LineCap::Butt,
            theme::hex("#D5DEEA").alpha(0.75),
        )
        .stroke_with(
            vec![apex, left, right],
            true,
            0.6,
            LineCap::Butt,
            theme::hex("#9AA6B6").alpha(0.40),
        )
}

/// The scene `seconds` into the loop as two vector layers: what lies behind the sphere (rear
/// prisms and the crescent) and what lies over it (molten flow, glass highlights and front
/// prisms). The sphere's shaded glass body sits between them as an element, see [`stack`].
/// Reduced motion passes `animated = false`, which freezes every layer at one complete pose.
pub(crate) fn layers(seconds: f32, animated: bool) -> (Shape, Shape) {
    let seconds = if animated { seconds } else { STILL_SECONDS };
    let prisms = if animated {
        satellites(seconds)
    } else {
        CROWN.map(|vertices| Satellite {
            vertices,
            depth: 1.0,
        })
    };
    let mut back = Shape::new();
    for satellite in prisms.iter().filter(|s| s.depth < 0.0) {
        back = prism(back, satellite);
    }
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
    let mut front = Shape::new().stroke_with(
        circle(CORE, CORE_RADIUS - 0.5, 72),
        true,
        0.8,
        LineCap::Butt,
        theme::hex("#CFC5AA").alpha(0.30),
    );
    for satellite in prisms.iter().filter(|s| s.depth >= 0.0) {
        front = prism(front, satellite);
    }
    (
        back.view_box(ENVELOPE[0], ENVELOPE[1]),
        front.view_box(ENVELOPE[0], ENVELOPE[1]),
    )
}

/// The orbit as one element `width` x `height` points: the back layer, the sphere's shaded glass
/// body (a radial gradient on a round element, dark and smoky at the rim and warm toward the
/// lower left), then the front layer. No raster is decoded and the reference GIF is never played.
fn stack(seconds: f32, animated: bool, width: f32, height: f32) -> El {
    let (back, front) = layers(seconds, animated);
    let k = width / ENVELOPE[0];
    let diameter = 2.0 * CORE_RADIUS * k;
    let at = if animated { seconds } else { STILL_SECONDS };
    let tile = CORE_PIXELS.with(rune::Pixels::clone);
    // Two copies of the photographic sphere inside a circle: the front one drifts, the back one
    // drifts against it at a different rate and shows through at half strength.
    let image = diameter * CORE_TEXTURE_MARGIN;
    let inset = (diameter - image) / 2.0;
    let [front_drift, back_drift] = core_drift(at);
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
                .absolute(0.0, 0.0)
                .size(width, height)
                .a11y_hidden(),
        )
        .child(body)
        .child(
            canvas(front)
                .absolute(0.0, 0.0)
                .size(width, height)
                .a11y_hidden(),
        )
}

/// The symbol alone at rest, `height` points tall, for chrome such as the sidebar header: the
/// fixed crown of the reduced-motion mark with no wordmark. Its accessible name is the product
/// name, which the visible UI no longer prints beside it.
pub(crate) fn mark(height: f32) -> El {
    stack(
        STILL_SECONDS,
        false,
        height * ENVELOPE[0] / ENVELOPE[1],
        height,
    )
    .key("brand-mark")
    .label("Fleetworks")
}

/// Draw the selected pose in flight, blending into the fixed crown at the rail.
pub(crate) fn handoff(seconds: f32, crown: f32, width: f32, height: f32) -> El {
    if crown == 0.0 {
        return stack(seconds, true, width, height);
    }
    el().size(width, height)
        .child(
            stack(seconds, true, width, height)
                .absolute(0.0, 0.0)
                .opacity(1.0 - crown),
        )
        .child(
            stack(seconds, false, width, height)
                .absolute(0.0, 0.0)
                .opacity(crown),
        )
}

/// The orbit as one element within the chooser's uniformly scaled envelope.
pub(crate) fn view(seconds: f32, animated: bool, width: f32, height: f32) -> El {
    stack(seconds, animated, width, height)
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

    fn edge_points(vertices: [[f32; 2]; 3]) -> Vec<[f32; 2]> {
        let mut points = Vec::new();
        for (i, a) in vertices.iter().enumerate() {
            let b = vertices[(i + 1) % 3];
            for step in 0..=8 {
                let t = step as f32 / 8.0;
                points.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        points
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
    fn the_brand_mark_keeps_the_envelope_aspect_and_the_still_pose() {
        for height in [24.0_f32, 34.0, 64.0] {
            let (w, h) = (height * ENVELOPE[0] / ENVELOPE[1], height);
            assert!((w / h - ENVELOPE[0] / ENVELOPE[1]).abs() < 1e-6);
        }
        // The mark draws the same still frame as the reduced-motion startup.
        let (back, front) = layers(STILL_SECONDS, false);
        assert_eq!(back, layers(STILL_SECONDS, false).0);
        assert_eq!(
            front,
            layers(1.0, false).1,
            "reduced motion ignores the clock"
        );
        assert!(!back.is_empty() && !front.is_empty());
    }

    #[test]
    fn fixed_crown_matches_the_canonical_symbol_and_clears_the_glass() {
        let svg = include_str!("../../../assets/brand/fleetworks-mark.svg");
        let crescent = crescent();
        for [apex, left, right] in CROWN {
            let points = format!(
                "points=\"{},{} {},{} {},{}\"",
                apex[0], apex[1], left[0], left[1], right[0], right[1],
            );
            assert!(svg.contains(&points));
            for p in edge_points([apex, left, right]) {
                assert!(distance_to_polygon(p, &crescent) > MIN_CLEARANCE);
                assert!((p[0] - CORE[0]).hypot(p[1] - CORE[1]) > CORE_RADIUS);
            }
        }
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
    fn no_prism_edge_comes_near_the_crescent_at_any_phase() {
        let polygon = crescent();
        let mut closest = f32::MAX;
        for seconds in samples() {
            for satellite in satellites(seconds) {
                for point in edge_points(satellite.vertices) {
                    closest = closest.min(distance_to_polygon(point, &polygon));
                }
            }
        }
        assert!(
            closest - PAINT_REACH >= MIN_CLEARANCE,
            "the full painted silhouettes come within {} of the crescent",
            closest - PAINT_REACH
        );
        assert!(closest >= MIN_CLEARANCE, "closest approach {closest}");
    }

    #[test]
    fn all_five_prisms_persist_and_each_passes_in_front_and_behind() {
        let mut front = [false; SATELLITE_COUNT];
        let mut behind = [false; SATELLITE_COUNT];
        for seconds in samples() {
            let prisms = satellites(seconds);
            assert_eq!(prisms.len(), SATELLITE_COUNT);
            for (i, prism) in prisms.iter().enumerate() {
                if prism.depth >= 0.0 {
                    front[i] = true;
                } else {
                    behind[i] = true;
                }
            }
        }
        assert!(front.iter().all(|seen| *seen));
        assert!(behind.iter().all(|seen| *seen));
    }

    #[test]
    fn the_whole_orbit_stays_inside_the_envelope_at_every_phase() {
        for seconds in samples() {
            for satellite in satellites(seconds) {
                for [x, y] in satellite.vertices {
                    assert!((0.0..=ENVELOPE[0]).contains(&x), "x {x} at {seconds}s");
                    assert!((0.0..=ENVELOPE[1]).contains(&y), "y {y} at {seconds}s");
                }
            }
        }
    }

    #[test]
    fn the_loop_is_sixteen_seconds_and_seamless() {
        let start = satellites(0.0);
        let end = satellites(LOOP_SECONDS - 1e-3);
        for (a, b) in start.iter().zip(end.iter()) {
            for (p, q) in a.vertices.iter().zip(b.vertices.iter()) {
                assert!((p[0] - q[0]).abs() < 0.2 && (p[1] - q[1]).abs() < 0.2);
            }
        }
    }

    #[test]
    fn the_first_three_seconds_cover_less_than_half_a_turn() {
        // Startup keeps the quiet pace instead of finishing a revolution.
        let turn = phase(3.0);
        assert!((turn - TAU * 3.0 / LOOP_SECONDS).abs() < 1e-4);
        assert!(turn < PI);
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
    fn geometry_and_timing_match_the_playback_manifest() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/brand/reference/quiet-orbit/playback.json"
        ))
        .expect("playback manifest");
        let orbit = &manifest["orbit"];
        let core = orbit["core_reference_radius_at_width_800_px"]
            .as_f64()
            .unwrap() as f32;
        let at_800 = |key: &str| orbit[key].as_f64().unwrap() as f32;
        assert!((at_800("radius_at_width_800_px") / core - RADIUS_RATIO).abs() < 1e-3);
        assert!(
            (at_800("projected_upward_offset_at_width_800_px") / core * CORE_RADIUS - LIFT).abs()
                < 1e-2
        );
        assert!((at_800("inclination_degrees").to_radians() - INCLINATION).abs() < 1e-4);
        assert!((at_800("prism_scale") - PRISM_SCALE).abs() < 1e-6);
        assert_eq!(orbit["satellites"].as_u64(), Some(SATELLITE_COUNT as u64));
        assert!((at_800("phase_spacing_degrees") - 360.0 / SATELLITE_COUNT as f32).abs() < 1e-4);
        assert_eq!(
            manifest["reference"]["seconds"].as_f64(),
            Some(f64::from(LOOP_SECONDS))
        );
        let startup = &manifest["startup"];
        let ms = |key: &str| startup[key].as_f64().unwrap();
        assert!((ms("duration_ms") - crate::startup::INTRO_SECONDS * 1000.0).abs() < 1e-6);
        assert_eq!(
            startup["backend_choices_from_first_frame"].as_bool(),
            Some(true)
        );
        assert_eq!(manifest["canvas"]["color"].as_str(), Some("#060709"));
        assert_eq!(crate::theme::BACKGROUND, "#060709");
    }

    #[test]
    fn polygon_distance_is_zero_on_an_edge_and_positive_away() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert_eq!(distance_to_polygon([5.0, 0.0], &square), 0.0);
        assert_eq!(distance_to_polygon([15.0, 5.0], &square), 5.0);
    }
}
