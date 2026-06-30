//! Design-time adequacy: the [`required_sample_size`] to detect an effect and a
//! [`power_warning`] that flags an underpowered fixture set.
//!
//! Backlog 003's rigor cuts both ways. Before reporting a delta you refuse the
//! ones inside the noise floor ([`super::paired`]); before *running* a fixture
//! set you check it is large enough to surface the effect you care about at all.
//! A too-small fixture set does not produce wrong numbers — it produces
//! confident-looking "no difference" verdicts that are really just silence.

use super::normal::inv_normal_cdf;

/// Minimum number of trials to detect an absolute change in a proportion.
///
/// For a one-sample test of a proportion (Fleiss), the normal-approximation
/// sample size to detect a shift from `baseline` to `baseline ± effect` is
///
/// ```text
/// n = ( z_{1-α/2}·√(p₀q₀) + z_{power}·√(p₁q₁) )² / effect²
/// ```
///
/// rounded up, where `p₀ = baseline`, `p₁ = baseline ± effect`, `q = 1 - p`, and
/// the `z`'s are normal quantiles from `alpha` (two-sided) and `power`. `effect`
/// is the absolute rate difference to detect; its magnitude is what matters, so
/// its sign is ignored and the feasible direction from the baseline is used.
///
/// Returns `None` on degenerate or infeasible input — `baseline` outside
/// `[0, 1]`, a non-positive `effect`, an `alpha` or `power` outside the open
/// `(0, 1)`, an `effect` so large that neither `baseline + effect` nor
/// `baseline - effect` lands in `[0, 1]`, or an `effect` so small that the
/// required count is non-finite or exceeds a `u64` — rather than a `NaN`, a
/// saturated `u64::MAX`, or a panic.
///
/// ```
/// use crucible_core::required_sample_size;
/// // p₀ = 0.5, detect a 0.1 shift, α = 0.05, power = 0.80 → 194 trials.
/// assert_eq!(required_sample_size(0.5, 0.1, 0.05, 0.80), Some(194));
/// assert_eq!(required_sample_size(0.5, 0.0, 0.05, 0.80), None); // no effect to detect
/// ```
pub fn required_sample_size(baseline: f64, effect: f64, alpha: f64, power: f64) -> Option<u64> {
    let delta = effect.abs();
    if !(0.0..=1.0).contains(&baseline)
        || delta <= 0.0
        || alpha <= 0.0
        || alpha >= 1.0
        || power <= 0.0
        || power >= 1.0
    {
        return None;
    }
    // Take the effect in whichever direction is feasible from the baseline.
    let p1 = if baseline + delta <= 1.0 {
        baseline + delta
    } else {
        baseline - delta
    };
    if !(0.0..=1.0).contains(&p1) {
        return None;
    }
    let z_alpha = inv_normal_cdf(1.0 - alpha / 2.0);
    let z_power = inv_normal_cdf(power);
    let sd0 = (baseline * (1.0 - baseline)).sqrt();
    let sd1 = (p1 * (1.0 - p1)).sqrt();
    let numerator = z_alpha * sd0 + z_power * sd1;
    let n = (numerator * numerator / (delta * delta)).ceil();
    // A positive but vanishing `effect` sends the requirement to +∞ (`delta²`
    // underflows to 0) or past `u64::MAX`; report it as undefined rather than a
    // saturated `Some(u64::MAX)` masquerading as a finite, achievable count.
    if !n.is_finite() || n >= u64::MAX as f64 {
        return None;
    }
    Some(n as u64)
}

/// A flag that a fixture set is too small to detect the effect of interest.
///
/// Produced by [`power_warning`] only when the set is actually underpowered, so
/// its presence *is* the warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerWarning {
    /// The fixture count that was checked.
    pub actual_n: u64,
    /// The minimum count [`required_sample_size`] computed for the same effect.
    pub required_n: u64,
}

impl PowerWarning {
    /// How many additional fixtures the set is short of adequate power.
    pub fn shortfall(self) -> u64 {
        self.required_n.saturating_sub(self.actual_n)
    }
}

/// Flag an underpowered fixture set, or `None` when it is adequately powered.
///
/// Computes the [`required_sample_size`] for the same `baseline` / `effect` /
/// `alpha` / `power` and returns `Some(PowerWarning)` when `actual_n` falls
/// short of it. Returns `None` when the set is large enough, and also when the
/// parameters are degenerate (no requirement can be computed) — the absence of
/// a warning never depends on guessing a requirement from bad input.
pub fn power_warning(
    actual_n: u64,
    baseline: f64,
    effect: f64,
    alpha: f64,
    power: f64,
) -> Option<PowerWarning> {
    let required_n = required_sample_size(baseline, effect, alpha, power)?;
    if actual_n < required_n {
        Some(PowerWarning {
            actual_n,
            required_n,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_sample_size_matches_textbook() {
        // One-sample proportion, p₀ = 0.5 vs p₁ = 0.6, α = 0.05 two-sided,
        // power 0.80. The formula yields 193.85 → 194.
        assert_eq!(required_sample_size(0.5, 0.1, 0.05, 0.80), Some(194));
    }

    #[test]
    fn required_sample_size_grows_as_the_effect_shrinks() {
        let big = required_sample_size(0.5, 0.2, 0.05, 0.80).unwrap();
        let small = required_sample_size(0.5, 0.05, 0.05, 0.80).unwrap();
        assert!(
            small > big,
            "smaller effect needs more samples: {small} vs {big}"
        );
    }

    #[test]
    fn required_sample_size_grows_with_power() {
        let p80 = required_sample_size(0.3, 0.1, 0.05, 0.80).unwrap();
        let p95 = required_sample_size(0.3, 0.1, 0.05, 0.95).unwrap();
        assert!(p95 > p80, "more power needs more samples: {p95} vs {p80}");
    }

    #[test]
    fn required_sample_size_ignores_effect_sign() {
        // A baseline of 0.5 is symmetric, so ±0.1 must give the same n.
        assert_eq!(
            required_sample_size(0.5, 0.1, 0.05, 0.80),
            required_sample_size(0.5, -0.1, 0.05, 0.80)
        );
    }

    #[test]
    fn required_sample_size_rejects_degenerate_input() {
        assert_eq!(required_sample_size(0.5, 0.0, 0.05, 0.80), None); // no effect
        assert_eq!(required_sample_size(1.5, 0.1, 0.05, 0.80), None); // baseline > 1
        assert_eq!(required_sample_size(-0.1, 0.1, 0.05, 0.80), None); // baseline < 0
        assert_eq!(required_sample_size(0.5, 0.1, 0.0, 0.80), None); // alpha = 0
        assert_eq!(required_sample_size(0.5, 0.1, 1.0, 0.80), None); // alpha = 1
        assert_eq!(required_sample_size(0.5, 0.1, 0.05, 0.0), None); // power = 0
        assert_eq!(required_sample_size(0.5, 0.1, 0.05, 1.0), None); // power = 1
        assert_eq!(required_sample_size(0.5, 0.6, 0.05, 0.80), None); // infeasible effect
    }

    #[test]
    fn required_sample_size_returns_none_when_the_requirement_overflows() {
        // A positive but vanishing effect sends n → +∞ (δ² underflows to 0):
        // undefined, not a saturated Some(u64::MAX).
        assert_eq!(required_sample_size(0.5, 1e-300, 0.05, 0.80), None);
        // A merely tiny effect keeps n finite but past u64::MAX (~1.8e19): the
        // count overflows the return type, so still None rather than u64::MAX.
        assert_eq!(required_sample_size(0.5, 1e-10, 0.05, 0.80), None);
    }

    #[test]
    fn power_warning_flags_an_underpowered_set() {
        // 194 required, 50 supplied → underpowered by 144.
        let w = power_warning(50, 0.5, 0.1, 0.05, 0.80).expect("should warn");
        assert_eq!(w.actual_n, 50);
        assert_eq!(w.required_n, 194);
        assert_eq!(w.shortfall(), 144);
    }

    #[test]
    fn power_warning_is_silent_when_adequately_powered() {
        assert_eq!(power_warning(200, 0.5, 0.1, 0.05, 0.80), None);
        // Exactly the requirement is adequate (not strictly less than).
        assert_eq!(power_warning(194, 0.5, 0.1, 0.05, 0.80), None);
    }

    #[test]
    fn power_warning_is_silent_on_degenerate_parameters() {
        // No requirement can be computed, so there is nothing to warn about.
        assert_eq!(power_warning(10, 0.5, 0.0, 0.05, 0.80), None);
    }
}
