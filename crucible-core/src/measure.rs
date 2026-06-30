//! Uncertainty and agreement primitives for reported rates.
//!
//! Per backlog 003 (measurement rigor), every rate Crucible reports must carry
//! an interval, and a model/agentic judge unlocks only above a measured
//! judge-vs-human agreement threshold. These are the small, pure building
//! blocks that machinery rests on: a point [`proportion`], a [`wilson_interval`]
//! around it, and percent [`agreement`] between two label vectors. They take
//! counts and slices, touch no IO, and never panic — degenerate inputs (no
//! trials, empty vectors) yield a defined zero rather than `NaN` or a panic.

/// The sample proportion `successes / n`.
///
/// Returns `0.0` when `n == 0`: with no trials there is no rate, and a total
/// function spares callers a divide-by-zero guard. Callers that must tell
/// "no data" apart from a true zero rate should check `n` themselves.
///
/// Precondition: `successes <= n`. Passing `successes > n` yields a proportion
/// above `1.0` (a caller bug, surfaced rather than silently clamped).
pub fn proportion(successes: u64, n: u64) -> f64 {
    if n == 0 {
        return 0.0;
    }
    successes as f64 / n as f64
}

/// The Wilson score interval `(lower, upper)` for a binomial proportion.
///
/// `successes` of `n` trials succeeded; `z` is the standard-normal quantile for
/// the target confidence (e.g. `1.96` for 95%). Wilson is the small-n /
/// extreme-`p` choice (backlog 003): unlike the normal approximation it stays
/// within `[0, 1]` and stays well-behaved at `p̂ = 0` or `p̂ = 1`. The bounds are
/// clamped to `[0, 1]` to absorb floating-point error at those extremes.
///
/// Returns `(0.0, 0.0)` when `n == 0` — no trials, no interval.
///
/// # Examples
///
/// ```
/// let (lo, hi) = crucible_core::wilson_interval(8, 10, 1.96);
/// assert!((lo - 0.49).abs() < 0.01);
/// assert!((hi - 0.94).abs() < 0.01);
/// ```
pub fn wilson_interval(successes: u64, n: u64, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let n = n as f64;
    let p_hat = successes as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p_hat + z2 / (2.0 * n)) / denom;
    let margin = (z / denom) * (p_hat * (1.0 - p_hat) / n + z2 / (4.0 * n * n)).sqrt();
    let lower = (center - margin).max(0.0);
    let upper = (center + margin).min(1.0);
    (lower, upper)
}

/// Percent agreement between two boolean label vectors.
///
/// Returns the fraction of positions where `a` and `b` carry the same label —
/// the simplest judge-vs-human agreement signal (backlog 003 gates a judge on a
/// measured agreement threshold). This is raw agreement, not chance-corrected;
/// κ (Cohen's kappa) is a later primitive.
///
/// Returns `0.0` for empty input. The two slices are expected to be paired and
/// equal-length; if their lengths differ, only the common prefix is compared
/// and forms the denominator, so the result is always a valid `[0, 1]`
/// proportion.
pub fn agreement(a: &[bool], b: &[bool]) -> f64 {
    let pairs = a.len().min(b.len());
    if pairs == 0 {
        return 0.0;
    }
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / pairs as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Float comparison tolerance for interval bounds.
    const EPS: f64 = 1e-9;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn proportion_is_successes_over_n() {
        assert!(close(proportion(8, 10), 0.8, EPS));
        assert!(close(proportion(1, 4), 0.25, EPS));
        assert!(close(proportion(0, 7), 0.0, EPS));
        assert!(close(proportion(5, 5), 1.0, EPS));
    }

    #[test]
    fn proportion_returns_zero_for_no_trials() {
        assert_eq!(proportion(0, 0), 0.0);
    }

    #[test]
    fn wilson_matches_known_value_8_of_10() {
        // Textbook Wilson 95% interval for 8/10 is ~[0.490, 0.943].
        let (lo, hi) = wilson_interval(8, 10, 1.96);
        assert!(close(lo, 0.49, 0.01), "lower {lo} not ~0.49");
        assert!(close(hi, 0.94, 0.01), "upper {hi} not ~0.94");
    }

    #[test]
    fn wilson_matches_known_value_50_of_100() {
        // Wilson 95% interval for 50/100 is ~[0.404, 0.596].
        let (lo, hi) = wilson_interval(50, 100, 1.96);
        assert!(close(lo, 0.404, 0.01), "lower {lo} not ~0.404");
        assert!(close(hi, 0.596, 0.01), "upper {hi} not ~0.596");
    }

    #[test]
    fn wilson_returns_zero_interval_for_no_trials() {
        assert_eq!(wilson_interval(0, 0, 1.96), (0.0, 0.0));
    }

    #[test]
    fn wilson_bounds_stay_within_unit_interval_at_extremes() {
        // p̂ = 0: lower bound pinned at 0, never negative.
        let (lo, hi) = wilson_interval(0, 10, 1.96);
        assert!(lo >= 0.0 && close(lo, 0.0, EPS), "lower {lo} should be ~0");
        assert!(hi > 0.0 && hi < 1.0, "upper {hi} out of (0,1)");

        // p̂ = 1: upper bound pinned at 1, never above.
        let (lo, hi) = wilson_interval(10, 10, 1.96);
        assert!(lo > 0.0 && lo < 1.0, "lower {lo} out of (0,1)");
        assert!(hi <= 1.0 && close(hi, 1.0, EPS), "upper {hi} should be ~1");
    }

    #[test]
    fn wilson_is_ordered_and_brackets_the_point_estimate() {
        let (lo, hi) = wilson_interval(8, 10, 1.96);
        let p = proportion(8, 10);
        assert!(lo < hi, "interval not ordered");
        assert!(lo <= p && p <= hi, "point estimate {p} outside interval");
    }

    #[test]
    fn wilson_widens_as_confidence_grows() {
        // A larger z (higher confidence) yields a wider interval at the same data.
        let (lo90, hi90) = wilson_interval(8, 10, 1.645);
        let (lo99, hi99) = wilson_interval(8, 10, 2.576);
        assert!(hi99 - lo99 > hi90 - lo90, "higher confidence not wider");
    }

    #[test]
    fn agreement_is_one_when_labels_match() {
        assert_eq!(agreement(&[true, true, false], &[true, true, false]), 1.0);
    }

    #[test]
    fn agreement_is_zero_when_labels_all_differ() {
        assert_eq!(agreement(&[true, false, true], &[false, true, false]), 0.0);
    }

    #[test]
    fn agreement_is_fraction_matching() {
        // 3 of 4 positions agree.
        let a = [true, true, true, false];
        let b = [true, false, true, false];
        assert!(close(agreement(&a, &b), 0.75, EPS));
    }

    #[test]
    fn agreement_returns_zero_for_empty() {
        assert_eq!(agreement(&[], &[]), 0.0);
    }

    #[test]
    fn agreement_compares_common_prefix_when_lengths_differ() {
        // Only the first (shared) position is compared; both are `true`.
        assert_eq!(agreement(&[true, true], &[true]), 1.0);
    }
}
