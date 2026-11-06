//! Calibration-only sweep over relevance thresholds and discriminative support.
//! Each relevance group is consumed once per support candidate. No repeated
//! curve fitting/lookup inside a threshold-by-row-by-knot loop.
use super::{
    score, ClassifierError, DomainCalibration, LabelMembership, ScoredFamily, DISABLED_THRESHOLD,
};
use std::collections::BTreeSet;

pub(super) fn select(
    rows: &[ScoredFamily],
    domain: usize,
    curve: &DomainCalibration,
) -> Result<(f64, usize), ClassifierError> {
    let mut scored = Vec::new();
    let mut supports = BTreeSet::from([1usize]);
    for row in rows {
        let count = row.raw[domain].known_feature_count;
        if count == 0 {
            continue;
        }
        supports.insert(count);
        scored.push((
            score(row.raw[domain].raw_log_odds, curve)?,
            count,
            row.membership[domain] == LabelMembership::Present,
        ));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (mut best_threshold, mut best_support, mut best_count, mut best_tp) =
        (DISABLED_THRESHOLD, 1, 0u64, 0u64);
    for support in supports {
        let (mut accepted, mut tp) = (0u64, 0u64);
        for (index, (relevance, count, positive)) in scored.iter().enumerate() {
            if *count >= support {
                accepted += 1;
                tp += u64::from(*positive);
            }
            if scored
                .get(index + 1)
                .is_some_and(|next| next.0 == *relevance)
            {
                continue;
            }
            if accepted > 0
                && 10 * tp >= 9 * accepted
                && (accepted > best_count
                    || (accepted == best_count
                        && (tp > best_tp || (tp == best_tp && *relevance > best_threshold))))
            {
                best_count = accepted;
                best_tp = tp;
                best_threshold = *relevance;
                best_support = support;
            }
        }
    }
    Ok((best_threshold, best_support))
}
