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
//!
//! A single seeded interval is reproducible but not *seed-invariant*: when the
//! resample distribution has an atom at a decision boundary (a paired delta with
//! mass piled exactly at `0` is the case that bites a leaderboard), the percentile
//! bound can land on either side of that atom depending on the seed, so a
//! verdict read off "does the interval exclude 0" flips with the seed. That is a
//! reproducibility-of-the-wrong-thing trap. [`bootstrap_envelope`] closes it by
//! taking the conservative envelope over an ensemble of seeds, so a directional
//! "excludes 0" decision becomes unanimous across the ensemble by construction
//! and therefore stable across *which* seed the caller picked.

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

/// A seed-robust bootstrap interval: the conservative envelope of an ensemble of
/// independently seeded percentile intervals.
///
/// Same shape as [`BootstrapInterval`], but [`lower`](Self::lower) /
/// [`upper`](Self::upper) are the *widest* bounds seen across the ensemble (the
/// minimum lower and maximum upper), and [`seeds`](Self::seeds) records how many
/// members were folded in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnsembleInterval {
    /// The metric on the original sample — seed-free, identical to every member.
    pub point: f64,
    /// The *minimum* lower bound across the ensemble (the widest interval's floor).
    pub lower: f64,
    /// The *maximum* upper bound across the ensemble (the widest interval's ceiling).
    pub upper: f64,
    /// How many independently seeded members were folded into the envelope.
    pub seeds: usize,
    /// Resamples drawn per member (echoed for reporting).
    pub resamples: usize,
}

/// Deterministic, seed-robust bootstrap interval: the conservative envelope over
/// `ensemble` independently seeded percentile intervals.
///
/// Runs [`bootstrap_interval`] under `ensemble` distinct seeds — derived
/// deterministically from `base_seed`, so the whole result is reproducible from
/// `base_seed` alone — and returns the union of their intervals: `lower` is the
/// MINIMUM lower bound any member produced and `upper` the MAXIMUM upper bound.
///
/// # Why the envelope, not one interval
///
/// The envelope excludes `0` (or any threshold) on a side **iff every member
/// does**: `lower > 0` means the smallest member-lower is still positive, i.e.
/// all members put their lower bound above `0`. So a directional "excludes 0"
/// decision read off this envelope is unanimous across the ensemble *by
/// construction*. A borderline interval that some seeds would exclude and others
/// would not collapses to "includes 0" (a refusal) — and it does so for *any*
/// `base_seed`, because no ensemble of that size will be unanimous on a genuinely
/// borderline atom. That converts seeded-but-flippy into seed-invariant: the
/// published directional verdict no longer depends on which seed was chosen. The
/// price is conservatism (the envelope is wider than any single member), which is
/// the right bias for a "refuse a delta you cannot defend" gate.
///
/// Returns `None` on the same degenerate inputs as [`bootstrap_interval`] (empty
/// `data`, zero `resamples`, `confidence` outside `(0, 1)`) and additionally when
/// `ensemble == 0` — never a `NaN` or a panic.
///
/// ```
/// use crucible_core::bootstrap_envelope;
/// let data = [0.2, 0.4, 0.6, 0.8, 1.0];
/// let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
/// let env = bootstrap_envelope(&data, mean, 500, 0.95, 7, 32).unwrap();
/// // Reproducible from the base seed, and at least as wide as any one member.
/// assert_eq!(env, bootstrap_envelope(&data, mean, 500, 0.95, 7, 32).unwrap());
/// assert!(env.lower <= env.point && env.point <= env.upper);
/// ```
pub fn bootstrap_envelope<T, F>(
    data: &[T],
    metric: F,
    resamples: usize,
    confidence: f64,
    base_seed: u64,
    ensemble: usize,
) -> Option<EnsembleInterval>
where
    T: Clone,
    F: Fn(&[T]) -> f64,
{
    if ensemble == 0 {
        return None;
    }
    // Sub-seeds are SplitMix64-derived from `base_seed` (not `base_seed + i`), so
    // two nearby base seeds produce *decorrelated* ensembles — a stricter
    // stability guarantee than overlapping windows would give.
    let mut seeder = SplitMix64::new(base_seed);
    let mut point = None;
    let mut lower = f64::INFINITY;
    let mut upper = f64::NEG_INFINITY;
    for _ in 0..ensemble {
        let seed = seeder.next_u64();
        // Any member is degenerate iff all are (same data/args), so propagate.
        let member = bootstrap_interval(data, &metric, resamples, confidence, seed)?;
        point = Some(member.point);
        lower = lower.min(member.lower);
        upper = upper.max(member.upper);
    }
    point.map(|point| EnsembleInterval {
        point,
        lower,
        upper,
        seeds: ensemble,
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

    // ----- The seed-robust envelope -----------------------------------------

    #[test]
    fn envelope_is_deterministic_for_a_base_seed() {
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let a = bootstrap_envelope(&data, mean, 1000, 0.95, 42, 32).unwrap();
        let b = bootstrap_envelope(&data, mean, 1000, 0.95, 42, 32).unwrap();
        assert_eq!(a, b, "same base seed must reproduce the envelope exactly");
        assert_eq!(a.seeds, 32);
        assert_eq!(a.point, mean(&data));
    }

    #[test]
    fn envelope_contains_every_member_interval() {
        // The envelope is the union of its members, so it is at least as wide as
        // any single member's interval — the conservatism the verdict relies on.
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let env = bootstrap_envelope(&data, mean, 2000, 0.95, 7, 64).unwrap();
        // The first member uses the first SplitMix64 draw off the base seed.
        let mut seeder = SplitMix64::new(7);
        let member = bootstrap_interval(&data, mean, 2000, 0.95, seeder.next_u64()).unwrap();
        assert!(env.lower <= member.lower, "envelope floor not below member");
        assert!(
            env.upper >= member.upper,
            "envelope ceiling not above member"
        );
        assert!(env.lower <= env.point && env.point <= env.upper);
    }

    #[test]
    fn envelope_refuses_a_zero_atom_that_a_single_seed_would_exclude() {
        // A paired-delta shape with ~2.3% of resample mass piled exactly on 0:
        // three "+1" buckets and five "0" buckets, ratio-of-sums metric. A single
        // seed's 2.5th percentile can land on either side of that atom (the
        // leaderboard seed-flip bug). The envelope's floor must sit at 0 — i.e.
        // include 0, a stable refusal — because some member always lands on the
        // atom. And it must do so for several unrelated base seeds.
        let data = [
            (1.0_f64, 1u64),
            (1.0, 1),
            (1.0, 1),
            (0.0, 1),
            (0.0, 1),
            (0.0, 1),
            (0.0, 1),
            (0.0, 1),
        ];
        let ratio = |s: &[(f64, u64)]| {
            let (sum, n) = s.iter().fold((0.0, 0u64), |(a, c), &(x, k)| (a + x, c + k));
            sum / n as f64
        };
        for base in [1u64, 7, 99, 2024, 0xDEAD_BEEF] {
            let env = bootstrap_envelope(&data, ratio, 4000, 0.95, base, 64).unwrap();
            assert!(
                env.lower <= 0.0,
                "base {base}: envelope floor {} should include the zero-atom",
                env.lower
            );
            assert!((env.point - 0.375).abs() < 1e-12, "point {}", env.point);
        }
    }

    #[test]
    fn envelope_keeps_a_clear_exclusion_across_seeds() {
        // A real, well-separated positive metric: every member excludes 0 below,
        // so the envelope floor stays above 0 for any base seed — a stable
        // signal, not a stable refusal.
        let data = [0.7, 0.8, 0.9, 1.0, 0.75, 0.85, 0.95, 0.65];
        for base in [1u64, 7, 99, 2024] {
            let env = bootstrap_envelope(&data, mean, 4000, 0.95, base, 64).unwrap();
            assert!(env.lower > 0.0, "base {base}: floor {} left 0", env.lower);
        }
    }

    #[test]
    fn envelope_collapses_on_constant_data() {
        let data = [3.0, 3.0, 3.0];
        let env = bootstrap_envelope(&data, mean, 200, 0.95, 5, 16).unwrap();
        assert_eq!((env.point, env.lower, env.upper), (3.0, 3.0, 3.0));
    }

    #[test]
    fn envelope_rejects_degenerate_args() {
        let data = [1.0, 2.0, 3.0];
        assert!(bootstrap_envelope::<f64, _>(&[], mean, 100, 0.95, 1, 8).is_none());
        assert!(bootstrap_envelope(&data, mean, 0, 0.95, 1, 8).is_none());
        assert!(bootstrap_envelope(&data, mean, 100, 0.0, 1, 8).is_none());
        assert!(bootstrap_envelope(&data, mean, 100, 1.0, 1, 8).is_none());
        assert!(bootstrap_envelope(&data, mean, 100, 0.95, 1, 0).is_none());
    }
}
