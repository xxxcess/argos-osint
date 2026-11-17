//! Quiet Orbit: the native startup scene, drawn as vector layers over the shared `#060709` canvas.
//!
//! The orbit follows the owner's reference geometry (`assets/brand/brand-manifest.json`): five
//! persistent obsidian-glass prisms on one inclined, lifted ring around a smoky glass sphere of
//! flowing molten liquid, above a fixed obsidian crescent. Orbit radius is 1.303 core radii, the
//! inclination is 66 degrees, the ring is lifted 0.381 core radii, prisms are 0.70 scale, phases
//! are 72 degrees apart and one revolution takes eight seconds. The three-second startup samples
//! the first part of that same loop; it never speeds up to finish a turn.
//!
//! The scene is one 144 x 176 envelope that scales as a unit. Everything here is vector geometry
//! with no raster, so there is no media rectangle to flash and nothing to decode at launch.
//!
//! shortcut: a vector approximation of the reference materials. The reference rasters are kept as
//! canonical references and are not shipped at runtime; replace [`scene`] with bounded RGBA
//! layers if the derived layer pack is added to `assets/brand/splash/`.

use std::f32::consts::PI;
use std::f32::consts::TAU;

use rune::canvas;
use rune::El;
use rune::LineCap;
use rune::Shape;

use crate::theme;

/// Scene envelope in scene units; the element scales it uniformly.
pub(crate) const ENVELOPE: [f32; 2] = [144.0, 176.0];
/// One orbital revolution, matching the eight-second motion reference.
pub(crate) const LOOP_SECONDS: f32 = 8.0;
/// Closest any prism edge may come to the crescent polygon, in scene units. The reference scene
/// keeps about 5.3 units at this scale; the geometry below measures about 8.
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
/// Angular rock fragments inside the crescent, each as three vertices.
const ROCK: [[[f32; 2]; 3]; 5] = [
    [[40.0, 137.0], [62.0, 129.0], [56.0, 151.0]],
    [[68.0, 133.0], [94.0, 131.0], [84.0, 153.0]],
    [[100.0, 139.0], [120.0, 135.0], [102.0, 151.0]],
    [[28.0, 129.0], [44.0, 123.0], [36.0, 143.0]],
    [[60.0, 159.0], [84.0, 161.0], [72.0, 145.0]],
];

/// One prism at one instant.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Satellite {
    /// Apex, left base corner, right base corner.
    pub vertices: [[f32; 2]; 3],
    /// Positive depth passes in front of the sphere; negative depth passes behind it.
    pub depth: f32,
}

/// One molten lobe: center, radius and warmth (0 amber, 1 ivory).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lobe {
    pub center: [f32; 2],
    pub radius: f32,
    pub warmth: f32,
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

/// Molten lobes drifting inside the sphere. Their motion is independent of the orbit's turn.
pub(crate) fn lobes(seconds: f32) -> [Lobe; 5] {
    let phase = phase(seconds);
    std::array::from_fn(|i| {
        let k = i as f32;
        let angle = TAU * k / 5.0 + 0.9 * (phase + k).sin();
        let reach = CORE_RADIUS * (0.30 + 0.06 * (2.0 * phase + k * 1.7).cos());
        let radius = CORE_RADIUS * (0.24 + 0.03 * (phase + k * 2.3).sin());
        Lobe {
            center: [
                CORE[0] + reach * angle.cos(),
                CORE[1] + reach * 0.9 * angle.sin(),
            ],
            radius,
            warmth: if i % 2 == 0 { 0.2 } else { 0.9 },
        }
    })
}

/// Smallest distance from `point` to the closed polygon's edges.
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

fn prism(shape: Shape, satellite: &Satellite) -> Shape {
    let [apex, left, right] = satellite.vertices;
    let mid = [(left[0] + right[0]) / 2.0, (left[1] + right[1]) / 2.0];
    shape
        .fill(vec![apex, left, right], theme::hex("#0B0E13"))
        .fill(vec![apex, left, mid], theme::hex("#1A2029"))
        .stroke_with(
            vec![apex, left, right],
            true,
            0.7,
            LineCap::Butt,
            theme::hex("#9AA6B6").alpha(0.45),
        )
}

fn molten(lobe: &Lobe) -> (Vec<[f32; 2]>, rune::Color) {
    let color = if lobe.warmth < 0.5 {
        theme::hex("#C98A3E").alpha(0.50)
    } else {
        theme::hex("#E6D9B8").alpha(0.40)
    };
    (circle(lobe.center, lobe.radius, 28), color)
}

/// The whole scene `seconds` into the loop. Reduced motion passes `animated = false`, which
/// freezes every layer at one complete pose.
pub(crate) fn scene(seconds: f32, animated: bool) -> Shape {
    let seconds = if animated { seconds } else { STILL_SECONDS };
    let prisms = satellites(seconds);
    let mut shape = Shape::new();
    for satellite in prisms.iter().filter(|s| s.depth < 0.0) {
        shape = prism(shape, satellite);
    }
    // Obsidian crescent: polished glass edge around coarse angular rock.
    shape = shape.fill(crescent(), theme::hex("#0E1116")).stroke_with(
        crescent(),
        true,
        1.0,
        LineCap::Butt,
        theme::hex("#8A94A4").alpha(0.55),
    );
    for (i, rock) in ROCK.iter().enumerate() {
        let tone = if i % 2 == 0 { "#1D232D" } else { "#151A22" };
        shape = shape.fill(rock.to_vec(), theme::hex(tone)).stroke_with(
            rock.to_vec(),
            true,
            0.6,
            LineCap::Butt,
            theme::hex("#05070A").alpha(0.9),
        );
    }
    for fleck in FLECKS {
        shape = shape.fill(
            circle(fleck, FLECK_RADIUS, 6),
            theme::hex("#C9D2E0").alpha(0.35),
        );
    }
    // Sphere: smoky glass shell around flowing molten lobes, with one restrained reflection.
    shape = shape.fill(circle(CORE, CORE_RADIUS, 72), theme::hex("#191612"));
    for lobe in lobes(seconds) {
        let (points, color) = molten(&lobe);
        shape = shape.fill(points, color);
    }
    shape = shape
        .fill(
            circle(CORE, CORE_RADIUS * 0.34, 36),
            theme::hex("#F0E2BE").alpha(0.28),
        )
        .stroke_with(
            circle(CORE, CORE_RADIUS - 0.5, 72),
            true,
            1.0,
            LineCap::Butt,
            theme::hex("#B8B09C").alpha(0.40),
        )
        .stroke(
            arc(CORE, CORE_RADIUS - 4.0, PI * 1.12, PI * 1.45, 12),
            1.2,
            theme::hex("#F3EEE4").alpha(0.35),
        );
    for satellite in prisms.iter().filter(|s| s.depth >= 0.0) {
        shape = prism(shape, satellite);
    }
    shape.view_box(ENVELOPE[0], ENVELOPE[1])
}

/// The orbit as one element sized for `window`.
pub(crate) fn view(seconds: f32, animated: bool, window: Option<[f32; 2]>) -> El {
    let [width, height] = display_size(window);
    canvas(scene(seconds, animated))
        .key("startup-orbit")
        .label("Fleetworks")
        .w(width)
        .h(height)
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
    fn rock_and_flecks_sit_inside_the_crescent_glass() {
        let inside = |p: [f32; 2], margin: f32| {
            (p[0] - CRESCENT_CENTER[0]).hypot(p[1] - CRESCENT_CENTER[1]) + margin < CRESCENT_RADIUS
                && (p[0] - CUT_CENTER[0]).hypot(p[1] - CUT_CENTER[1]) - margin > CUT_RADIUS
        };
        for fleck in FLECKS {
            assert!(inside(fleck, FLECK_RADIUS), "{fleck:?}");
        }
        for rock in ROCK {
            for vertex in rock {
                assert!(inside(vertex, 1.0), "{vertex:?}");
            }
        }
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
    fn molten_lobes_flow_and_stay_inside_the_sphere() {
        let first = lobes(0.0);
        let later = lobes(2.0);
        assert!(first
            .iter()
            .zip(later.iter())
            .any(|(a, b)| a.center != b.center));
        for seconds in samples() {
            for lobe in lobes(seconds) {
                let reach =
                    (lobe.center[0] - CORE[0]).hypot(lobe.center[1] - CORE[1]) + lobe.radius;
                assert!(reach < CORE_RADIUS * 0.9, "{reach} at {seconds}s");
            }
        }
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
    fn polygon_distance_is_zero_on_an_edge_and_positive_away() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert_eq!(distance_to_polygon([5.0, 0.0], &square), 0.0);
        assert_eq!(distance_to_polygon([15.0, 5.0], &square), 5.0);
    }
}
