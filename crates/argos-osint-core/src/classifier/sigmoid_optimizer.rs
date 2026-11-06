//! Bounded regularized Newton fit with projected monotonicity and line search.
use super::{invalid_calibration, ClassifierError};
fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}
fn objective(pairs: &[(f64, f64)], s: f64, i: f64) -> f64 {
    let mut value = 0.005 * (s * s + i * i);
    for (x, y) in pairs {
        let z = s * x + i;
        value += (z.max(0.0) - y * z + (-z.abs()).exp().ln_1p()) / pairs.len() as f64;
    }
    value
}
pub(super) fn fit(pairs: &[(f64, f64)]) -> Result<[f64; 2], ClassifierError> {
    let n = pairs.len() as f64;
    let center = pairs.iter().map(|(x, _)| x).sum::<f64>() / n;
    let scale = (pairs.iter().map(|(x, _)| (x - center).powi(2)).sum::<f64>() / n)
        .sqrt()
        .max(1.0);
    if !center.is_finite() || !scale.is_finite() {
        return Err(invalid_calibration("sigmoid_nonfinite_scale"));
    }
    let scaled: Vec<_> = pairs
        .iter()
        .map(|(x, y)| ((x - center) / scale, *y))
        .collect();
    let positives = pairs.iter().map(|(_, y)| y).sum::<f64>();
    let (mut s, mut i) = (1.0, ((positives + 1.0) / (n - positives + 1.0)).ln());
    let mut converged = false;
    for _ in 0..64 {
        let (mut gs, mut gi, mut hss, mut hii, mut hsi) = (0.01 * s, 0.01 * i, 0.01, 0.01, 0.0);
        for (x, y) in &scaled {
            let p = sigmoid(s * x + i);
            let residual = (p - y) / n;
            let weight = p * (1.0 - p) / n;
            gs += residual * x;
            gi += residual;
            hss += weight * x * x;
            hii += weight;
            hsi += weight * x;
        }
        let determinant = hss * hii - hsi * hsi;
        if !determinant.is_finite() || determinant <= 0.0 {
            return Err(invalid_calibration("sigmoid_singular_hessian"));
        }
        let (ds, di) = (
            (hii * gs - hsi * gi) / determinant,
            (hss * gi - hsi * gs) / determinant,
        );
        let old = objective(&scaled, s, i);
        let mut accepted = None;
        let mut step = 1.0;
        for _ in 0..24 {
            let (ns, ni) = (
                (s - step * ds).clamp(0.0, 50.0),
                (i - step * di).clamp(-50.0, 50.0),
            );
            if objective(&scaled, ns, ni) <= old + 1e-14 {
                accepted = Some((ns, ni));
                break;
            }
            step *= 0.5;
        }
        let Some((ns, ni)) = accepted else {
            return Err(invalid_calibration("sigmoid_line_search_failed"));
        };
        let delta = (ns - s).abs() + (ni - i).abs();
        s = ns;
        i = ni;
        if delta < 1e-8 {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(invalid_calibration("sigmoid_optimizer_not_converged"));
    }
    let parameters = [s / scale, i - s * center / scale];
    if !parameters.iter().all(|x| x.is_finite())
        || parameters[0] > 100.0
        || parameters[1].abs() > 100.0
    {
        return Err(invalid_calibration("sigmoid_parameter_bound"));
    }
    Ok(parameters)
}
