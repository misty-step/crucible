//! A deterministic, seeded bootstrap interval for a composite/derived metric.
//!
//! Wilson ([`super::rate`]) gives an interval for a single proportion in closed
//! form. A *derived* metric — a ratio of sums, an F-score, an agreement rate
//! over re-sampled fixtures — has no such formula, so its uncertainty is
//! estimated by resampling: draw many bootstrap samples (with replacement),
//! recompute the metric on each, and read percentiles off the resulting
//! distribution. The catch is reproducibility, which backlog 003 requires of
//! every reported number — so the resampling runs on a **seeded** PRNG and the
//! `seed` is a parameter: the same seed always yields byte-identical bounds.
//!
//! The interval is the percentile bootstrap. The bias-corrected and accelerated
//! (BCa) variant is intentionally *not* shipped: its bias and acceleration
//! terms are themselves undefined for degenerate resample distributions (e.g.
//! constant data, where the bias correction is `Φ⁻¹(0)`), which would reintroduce
//! exactly the `NaN`/`∞` hazards these primitives exist to avoid. The percentile
//! interval is the robust, total choice; BCa is an additive follow-up if a
//! skewed metric ever needs the extra accuracy.

/// SplitMix64: a tiny, seeded, reproducible `u64` stream.
///
/// One `wrapping_add` of the golden-ratio increment plus two avalanche rounds.
/// Good enough to scatter resample indices uniformly; the point is determinism,
/// not cryptographic quality.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A percentile bootstrap interval around a derived metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BootstrapInterval {
    /// The metric evaluated once on the original sample.
    pub point: f64,
    /// Lower percentile bound at the requested confidence.
    pub lower: f64,
    /// Upper percentile bound at the requested confidence.
    pub upper: f64,
    /// The number of bootstrap resamples drawn (echoed for reporting).
    pub resamples: usize,
}

/// Deterministic percentile bootstrap interval for `metric` over `data`.
///
/// Draws `resamples` samples of `data` (with replacement, same length as
/// `data`), evaluates `metric` on each, and returns the central
/// `confidence`-level percentile interval together with the metric on the
/// original sample. `metric` is any function of the sample, so the same call
/// covers a derived metric — a ratio of sums, an F-score — not just a mean.
///
/// Reproducible by construction: all randomness comes from a `SplitMix64`
/// seeded with `seed`, so identical arguments (including `seed`) yield a
/// byte-identical interval.
///
/// Returns `None` on degenerate input — empty `data`, zero `resamples`, or a
/// `confidence` outside the open `(0, 1)` — rather than a `NaN` or a panic.
///
/// ```
/// use crucible_core::bootstrap_interval;
/// let data = [0.2, 0.4, 0.6, 0.8, 1.0];
/// let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
/// // Same seed → byte-identical interval.
/// let a = bootstrap_interval(&data, mean, 500, 0.95, 7).unwrap();
/// let b = bootstrap_interval(&data, mean, 500, 0.95, 7).unwrap();
/// assert_eq!(a, b);
/// assert!(a.lower <= a.point && a.point <= a.upper);
/// ```
pub fn bootstrap_interval<T, F>(
    data: &[T],
    metric: F,
    resamples: usize,
    confidence: f64,
    seed: u64,
) -> Option<BootstrapInterval>
where
    T: Clone,
    F: Fn(&[T]) -> f64,
{
    if data.is_empty() || resamples == 0 || confidence <= 0.0 || confidence >= 1.0 {
        return None;
    }
    let n = data.len();
    let mut rng = SplitMix64::new(seed);
    let mut stats = Vec::with_capacity(resamples);
    let mut sample = Vec::with_capacity(n);
    for _ in 0..resamples {
        sample.clear();
        for _ in 0..n {
            // Modulo over the full u64 range: the index bias is negligible for
            // any realistic `n`, and the draw stays fully determined by `seed`.
            let idx = (rng.next_u64() % n as u64) as usize;
            sample.push(data[idx].clone());
        }
        stats.push(metric(&sample));
    }
    // total_cmp gives a deterministic order even if a metric ever returns NaN.
    stats.sort_by(f64::total_cmp);
    let alpha = 1.0 - confidence;
    Some(BootstrapInterval {
        point: metric(data),
        lower: percentile(&stats, alpha / 2.0),
        upper: percentile(&stats, 1.0 - alpha / 2.0),
        resamples,
    })
}

/// Linear-interpolated quantile of a sorted, non-empty slice (`q ∈ [0, 1]`).
fn percentile(sorted: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty(), "percentile of an empty slice");
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = rank - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mean(s: &[f64]) -> f64 {
        s.iter().sum::<f64>() / s.len() as f64
    }

    #[test]
    fn bootstrap_is_deterministic_for_a_seed() {
        let data = [0.2, 0.4, 0.6, 0.8, 1.0];
        let a = bootstrap_interval(&data, mean, 1000, 0.95, 12345).unwrap();
        let b = bootstrap_interval(&data, mean, 1000, 0.95, 12345).unwrap();
        assert_eq!(a, b, "same seed must reproduce the interval exactly");
    }

    #[test]
    fn bootstrap_varies_with_the_seed() {
        // Determinism is one half of the contract; the other half is that the
        // seed actually drives the resampling. Different seeds must yield
        // different bounds on data with spread to resample over — otherwise the
        // PRNG is not feeding the draw at all. (The point estimate is seed-free,
        // so the structs differ only in their bounds.)
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let a = bootstrap_interval(&data, mean, 1000, 0.95, 1).unwrap();
        let b = bootstrap_interval(&data, mean, 1000, 0.95, 2).unwrap();
        assert_ne!(a, b, "different seeds gave identical intervals");
    }

    #[test]
    fn bootstrap_point_is_seed_independent_and_bracketed() {
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let s1 = bootstrap_interval(&data, mean, 1000, 0.95, 1).unwrap();
        let s2 = bootstrap_interval(&data, mean, 1000, 0.95, 2).unwrap();
        // The point estimate is the metric on the original sample: seed-free.
        assert_eq!(s1.point, s2.point);
        assert_eq!(s1.point, 5.5);
        for s in [s1, s2] {
            assert!(
                s.lower <= s.point && s.point <= s.upper,
                "point outside interval"
            );
            assert!(s.lower < s.upper, "interval collapsed on spread data");
        }
    }

    #[test]
    fn bootstrap_collapses_on_constant_data() {
        // Every resample of constant data has the same mean, so the interval is
        // a point — and exactly that constant, with no NaN.
        let data = [3.0, 3.0, 3.0];
        let r = bootstrap_interval(&data, mean, 200, 0.95, 99).unwrap();
        assert_eq!(r.point, 3.0);
        assert_eq!(r.lower, 3.0);
        assert_eq!(r.upper, 3.0);
    }

    #[test]
    fn bootstrap_handles_a_composite_ratio_metric() {
        // A derived metric: ratio of summed hits to summed trials over paired
        // fixtures — exactly the case Wilson cannot express in closed form.
        let data = [(1u32, 1u32), (0, 1), (1, 1), (1, 1)];
        let ratio = |s: &[(u32, u32)]| {
            let hits: u32 = s.iter().map(|p| p.0).sum();
            let trials: u32 = s.iter().map(|p| p.1).sum();
            hits as f64 / trials as f64
        };
        let r = bootstrap_interval(&data, ratio, 1000, 0.90, 7).unwrap();
        assert!((r.point - 0.75).abs() < 1e-12, "point {}", r.point);
        assert!(r.lower <= r.point && r.point <= r.upper);
        assert!(
            r.lower >= 0.0 && r.upper <= 1.0,
            "ratio interval left [0,1]"
        );
        assert_eq!(r.resamples, 1000);
    }

    #[test]
    fn bootstrap_widens_with_confidence() {
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let lo = bootstrap_interval(&data, mean, 2000, 0.80, 5).unwrap();
        let hi = bootstrap_interval(&data, mean, 2000, 0.99, 5).unwrap();
        assert!(
            hi.upper - hi.lower >= lo.upper - lo.lower,
            "higher confidence not wider"
        );
    }

    #[test]
    fn bootstrap_rejects_degenerate_args() {
        let data = [1.0, 2.0, 3.0];
        assert!(bootstrap_interval::<f64, _>(&[], mean, 100, 0.95, 1).is_none());
        assert!(bootstrap_interval(&data, mean, 0, 0.95, 1).is_none());
        assert!(bootstrap_interval(&data, mean, 100, 0.0, 1).is_none());
        assert!(bootstrap_interval(&data, mean, 100, 1.0, 1).is_none());
    }
}
