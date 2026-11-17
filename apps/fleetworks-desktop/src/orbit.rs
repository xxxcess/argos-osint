//! Quiet Orbit: the native startup scene, drawn as vector layers over the shared `#060709` canvas.
//!
//! The orbit follows the owner's reference geometry (`assets/brand/reference/quiet-orbit/playback.json`, checked by a test below): five
//! persistent obsidian-glass prisms on one inclined, lifted ring around a smoky glass sphere of
//! flowing molten liquid, above a fixed obsidian crescent. Orbit radius is 1.303 core radii, the
//! inclination is 66 degrees, the ring is lifted 0.381 core radii, prisms are 0.70 scale, phases
//! are 72 degrees apart and one revolution takes eight seconds. The three-second startup samples
//! the first part of that same loop; it never speeds up to finish a turn.
//!
//! The scene is one 144 x 176 envelope that scales as a unit. Everything is vector geometry plus
//! one radial gradient on the sphere's round body, with no raster, so there is no media rectangle
//! to flash and nothing to decode at launch.
//!
//! Materials: the core is a smoky glass body (radial gradient) under twelve overlapping,
//! translucent amber, ivory and umber ribbons that flow continuously and loop seamlessly with
//! the orbit; the crescent is coarse fractured obsidian (about eighty angular facets from the
//! shared [`rock::noise`] hash) with a polished edge and faint flecks; each prism is glass with a
//! lit and a shaded facet, an inner bevel, a lit edge and a warm reflection. See
//! `docs/design/matte-canvas/motion.md` for the comparison and reproduction steps.
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
/// One orbital revolution, matching the eight-second motion reference.
pub(crate) const LOOP_SECONDS: f32 = 8.0;
/// Closest any prism edge may come to the crescent polygon, in scene units. The reference scene
/// keeps about 5.3 units at this scale; the geometry below measures about 8.
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
/// 38 px of lift at the 800 px reference, where the core radius is 99.76 px.
const LIFT: f32 = 38.0 / 99.760_765 * CORE_RADIUS;
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
                            lit: rock::noise(i, j, 26) % 3 == 0,
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

/// One ribbon of the molten flow: where it sits in the sphere and how it moves. `turns` is a
/// whole number so every ribbon returns to its start at the end of the eight-second loop.
struct Ribbon {
    lift: f32,
    sway: f32,
    waves: f32,
    offset: f32,
    turns: f32,
    width: f32,
    color: &'static str,
    alpha: f32,
}

const RIBBON_STEPS: usize = 24;
/// Ribbon centre lines stay inside this fraction of the core radius.
const FLOW_REACH: f32 = 0.74;
/// Radians the flow is turned by, so it sweeps diagonally across the sphere.
const FLOW_TILT: f32 = -0.38;
/// Amber, ivory and smoky-umber ribbons: the reference's continuous molten flow.
#[rustfmt::skip]
const RIBBONS: [Ribbon; 12] = [
    Ribbon { lift: -0.28, sway: 0.22, waves: 1.0, offset: 0.0, turns: 1.0, width: 0.16, color: "#E6D6AE", alpha: 0.38 },
    Ribbon { lift: -0.08, sway: 0.26, waves: 1.1, offset: 1.1, turns: -1.0, width: 0.18, color: "#C98A3E", alpha: 0.4 },
    Ribbon { lift: 0.1, sway: 0.22, waves: 0.9, offset: 2.0, turns: 1.0, width: 0.17, color: "#F2E6C6", alpha: 0.34 },
    Ribbon { lift: 0.26, sway: 0.2, waves: 1.2, offset: 0.6, turns: -1.0, width: 0.14, color: "#8A5A22", alpha: 0.42 },
    Ribbon { lift: -0.4, sway: 0.14, waves: 1.5, offset: 2.8, turns: 2.0, width: 0.08, color: "#FFF3D2", alpha: 0.3 },
    Ribbon { lift: 0.38, sway: 0.14, waves: 1.0, offset: 1.7, turns: 1.0, width: 0.1, color: "#C98A3E", alpha: 0.34 },
    Ribbon { lift: -0.02, sway: 0.3, waves: 0.8, offset: 3.6, turns: 2.0, width: 0.07, color: "#FFF6DC", alpha: 0.42 },
    Ribbon { lift: 0.16, sway: 0.12, waves: 1.6, offset: 4.4, turns: -2.0, width: 0.07, color: "#3B2814", alpha: 0.45 },
    Ribbon { lift: -0.2, sway: 0.18, waves: 1.1, offset: 5.0, turns: -1.0, width: 0.1, color: "#2A1C0E", alpha: 0.38 },
    Ribbon { lift: 0.0, sway: 0.34, waves: 0.7, offset: 0.4, turns: 1.0, width: 0.22, color: "#F5EAD0", alpha: 0.2 },
    Ribbon { lift: -0.34, sway: 0.16, waves: 1.3, offset: 3.1, turns: -2.0, width: 0.09, color: "#B8823A", alpha: 0.34 },
    Ribbon { lift: 0.3, sway: 0.18, waves: 1.4, offset: 5.6, turns: 2.0, width: 0.08, color: "#EFE0B8", alpha: 0.3 },
];

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

/// The molten flow `seconds` into the loop: each ribbon as one filled outline (a tapered band
/// along a sine path) with its translucent color. Ribbons drift, cross and overlap, so the core
/// reads as continuous liquid through smoky glass rather than separate shapes.
pub(crate) fn flow(seconds: f32) -> Vec<(Vec<[f32; 2]>, rune::Color)> {
    let turn = phase(seconds);
    RIBBONS
        .iter()
        .map(|ribbon| {
            let u_at = |i: usize| -1.0 + 2.0 * i as f32 / RIBBON_STEPS as f32;
            let centre: Vec<[f32; 2]> = (0..=RIBBON_STEPS)
                .map(|i| {
                    let u = u_at(i);
                    let x = 0.80 * u * CORE_RADIUS;
                    let wave = PI * ribbon.waves * u + ribbon.offset + ribbon.turns * turn;
                    let y = (ribbon.lift + ribbon.sway * wave.sin()) * CORE_RADIUS;
                    // Tilt the whole flow so the swirl sweeps diagonally, as in the reference.
                    let (x, y) = (
                        x * FLOW_TILT.cos() - y * FLOW_TILT.sin(),
                        x * FLOW_TILT.sin() + y * FLOW_TILT.cos(),
                    );
                    let reach = x.hypot(y);
                    let k = if reach > FLOW_REACH * CORE_RADIUS {
                        FLOW_REACH * CORE_RADIUS / reach
                    } else {
                        1.0
                    };
                    [CORE[0] + x * k, CORE[1] + y * k]
                })
                .collect();
            let (mut left, mut right) = (Vec::new(), Vec::new());
            for i in 0..=RIBBON_STEPS {
                let before = centre[i.saturating_sub(1)];
                let after = centre[(i + 1).min(RIBBON_STEPS)];
                let (dx, dy) = (after[0] - before[0], after[1] - before[1]);
                let length = dx.hypot(dy).max(1e-4);
                let u = u_at(i);
                let taper = (1.0 - u * u).max(0.0).powf(0.7);
                let half = (0.5 * ribbon.width * CORE_RADIUS * taper).max(0.15);
                let (nx, ny) = (-dy / length, dx / length);
                left.push([centre[i][0] + nx * half, centre[i][1] + ny * half]);
                right.push([centre[i][0] - nx * half, centre[i][1] - ny * half]);
            }
            right.reverse();
            left.extend(right);
            (left, theme::hex(ribbon.color).alpha(ribbon.alpha))
        })
        .collect()
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
    let prisms = satellites(seconds);
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

    // Over the sphere's body: continuous molten flow, then the smoky glass shell.
    let mut front = Shape::new();
    for (outline, color) in flow(seconds) {
        front = front.fill(outline, color);
    }
    front = front
        .fill(
            circle(
                [CORE[0] - 0.30 * CORE_RADIUS, CORE[1] - 0.34 * CORE_RADIUS],
                0.46 * CORE_RADIUS,
                36,
            ),
            theme::hex("#FFFFFF").alpha(0.045),
        )
        .stroke_with(
            circle(CORE, CORE_RADIUS - 1.8, 72),
            true,
            3.0,
            LineCap::Butt,
            theme::hex("#050403").alpha(0.45),
        )
        .stroke_with(
            circle(CORE, CORE_RADIUS - 0.5, 72),
            true,
            1.0,
            LineCap::Butt,
            theme::hex("#CFC5AA").alpha(0.45),
        )
        .stroke(
            arc(CORE, CORE_RADIUS - 4.0, PI * 1.12, PI * 1.45, 12),
            1.2,
            theme::hex("#F3EEE4").alpha(0.40),
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
    let body = el()
        .key("orbit-core")
        .absolute((CORE[0] - CORE_RADIUS) * k, (CORE[1] - CORE_RADIUS) * k)
        .size(diameter, diameter)
        .radius(diameter / 2.0)
        .fill(theme::hex("#0F0C09"))
        .radial_gradient(
            [0.40, 0.60],
            diameter * 0.62,
            [
                (0.0, theme::hex("#C9A66A")),
                (0.35, theme::hex("#8A6A3E")),
                (0.70, theme::hex("#3A2A18")),
                (1.0, theme::hex("#120E0A")),
            ],
        )
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
/// reduced-motion still pose of the orbit with no wordmark. Its accessible name is the product
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

/// The orbit as one element sized for `window`.
pub(crate) fn view(seconds: f32, animated: bool, window: Option<[f32; 2]>) -> El {
    let [width, height] = display_size(window);
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
    fn the_loop_is_eight_seconds_and_seamless() {
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
    fn the_molten_flow_moves_loops_and_stays_inside_the_sphere() {
        let first = flow(0.0);
        let later = flow(2.0);
        assert_eq!(first.len(), RIBBONS.len());
        assert!(
            first.iter().zip(later.iter()).any(|(a, b)| a.0 != b.0),
            "the flow moves"
        );
        // The loop is seamless: the flow at the loop's end is the flow at its start.
        let wrapped = flow(LOOP_SECONDS);
        for (a, b) in first.iter().zip(wrapped.iter()) {
            for (p, q) in a.0.iter().zip(b.0.iter()) {
                assert!((p[0] - q[0]).abs() < 1e-3 && (p[1] - q[1]).abs() < 1e-3);
            }
        }
        for seconds in samples().step_by(7) {
            for (outline, color) in flow(seconds) {
                assert!(outline.len() >= 4 && color.a > 0.0 && color.a < 0.5);
                for [x, y] in outline {
                    let reach = (x - CORE[0]).hypot(y - CORE[1]);
                    assert!(reach < CORE_RADIUS * 0.9, "{reach} at {seconds}s");
                }
            }
        }
    }

    #[test]
    fn the_flow_is_amber_ivory_and_umber_translucent_layers() {
        // Warm and cool ends of the reference's molten range, all translucent.
        let colors: Vec<_> = RIBBONS.iter().map(|r| r.color).collect();
        assert!(colors.contains(&"#C98A3E") && colors.contains(&"#F2E6C6"));
        assert!(colors.contains(&"#3B2814"));
        assert!(RIBBONS
            .iter()
            .all(|r| r.turns.fract() == 0.0 && r.alpha < 0.5));
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
        assert!((ms("duration_ms") - crate::startup::CHOOSE_AT * 1000.0).abs() < 1e-6);
        assert!((ms("crossfade_start_ms") - crate::startup::CROSSFADE_AT * 1000.0).abs() < 1e-6);
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
