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

/// Minimum total paired sample size to resolve an OBSERVED discordant
/// imbalance `(b, c)` over `n` shared paired trials at `(alpha, power)` —
/// Kotawala's resolution diagnostic (*Resolution Diagnostics for Paired LLM
/// Evaluation*, arXiv:2605.30315): "given the paired difference this
/// comparison actually showed, was `n` even large enough to call it
/// significant at the target power?"
///
/// This is the CORRECT paired-Bernoulli formula, derived directly from the
/// discordant counts already on hand ([`super::paired::PairedComparison`]'s
/// own `b`/`c`), not the common unpaired-Cohen's-h-times-`(1-rho)` shortcut
/// Kotawala documents as wrong by roughly 2x in the close-comparison regime
/// (3 of 5 popular calculators inherit it). The per-pair variance
/// `Var(D) = (b + c)/n - ((c - b)/n)²` is the same identity
/// [`super::paired::paired_rate_delta_interval`] already uses for its
/// confidence interval — algebraically identical to the (p1, p2, rho)
/// parameterization's `p1(1-p1) + p2(1-p2) - 2·rho·sqrt(p1(1-p1)·p2(1-p2))`
/// when `p1`, `p2`, `rho` are the true marginals/correlation of the same
/// joint distribution (see this module's `required_n_paired_matches_the_correct_paired_formula_not_the_unpaired_shortcut`
/// test, pinned against the reference `llm-power` tool's published values).
///
/// Returns `None` when undefined or degenerate: `n == 0`; `alpha`/`power`
/// outside the open `(0, 1)`; `b == c` (the observed effect is exactly
/// zero — no finite sample size "resolves" a zero effect); no discordant
/// pairs at all (`b + c == 0`, the variance estimate itself is degenerate);
/// or a requirement that overflows `u64` (mirrors [`required_sample_size`]'s
/// conventions).
///
/// ```
/// use crucible_core::required_n_paired;
/// // p1=0.6, p2=0.5, rho=0 (independent) as (b=3000, c=2000, n=10000):
/// // matches llm-power's parametric_required_n_paired_binary(0.6, 0.5, rho=0.0) == 385.
/// assert_eq!(required_n_paired(3000, 2000, 10000, 0.05, 0.80), Some(385));
/// ```
pub fn required_n_paired(b: u64, c: u64, n: u64, alpha: f64, power: f64) -> Option<u64> {
    if n == 0 || alpha <= 0.0 || alpha >= 1.0 || power <= 0.0 || power >= 1.0 || b == c {
        return None;
    }
    let variance = paired_per_pair_variance(b, c, n)?;
    let n_f = n as f64;
    let point = (c as f64 - b as f64) / n_f;
    let z_alpha = inv_normal_cdf(1.0 - alpha / 2.0);
    let z_power = inv_normal_cdf(power);
    let required = (z_alpha + z_power).powi(2) * variance / (point * point);
    let required = required.ceil();
    if !required.is_finite() || required >= u64::MAX as f64 {
        return None;
    }
    Some(required as u64)
}

/// The minimum detectable effect (MDE): the smallest `|delta|` that `n`
/// paired trials, at the paired variance observed in `(b, c, n)`, could
/// resolve at `(alpha, power)` — [`required_n_paired`]'s same closed form
/// solved for the effect instead of the count. Reported alongside
/// [`required_n_paired`]'s resolution ratio so a caller can see not just
/// "was this big enough" but "what effect size *could* this have seen."
///
/// Returns `None` when undefined: `n == 0`; `alpha`/`power` outside the open
/// `(0, 1)`; or no discordant pairs at all (`b + c == 0` — the variance
/// estimate is degenerate, not usably "zero").
///
/// ```
/// use crucible_core::minimum_detectable_effect_paired;
/// let mde = minimum_detectable_effect_paired(3000, 2000, 10000, 0.05, 0.80).unwrap();
/// assert!((mde - 0.019_611_097).abs() < 1e-6, "{mde}");
/// ```
pub fn minimum_detectable_effect_paired(
    b: u64,
    c: u64,
    n: u64,
    alpha: f64,
    power: f64,
) -> Option<f64> {
    if n == 0 || alpha <= 0.0 || alpha >= 1.0 || power <= 0.0 || power >= 1.0 {
        return None;
    }
    let variance = paired_per_pair_variance(b, c, n)?;
    let n_f = n as f64;
    let z_alpha = inv_normal_cdf(1.0 - alpha / 2.0);
    let z_power = inv_normal_cdf(power);
    let mde = (z_alpha + z_power) * (variance / n_f).sqrt();
    mde.is_finite().then_some(mde)
}

/// Per-pair variance `Var(D) = (b + c)/n - ((c - b)/n)²` of the paired
/// difference `D = 1{B correct} - 1{A correct}`, shared by
/// [`required_n_paired`] and [`minimum_detectable_effect_paired`] (and
/// algebraically the same quantity [`super::paired::paired_rate_delta_interval`]
/// divides by `n` again for its confidence interval's standard error).
/// `None` when there are no discordant pairs (`b + c == 0`) — a degenerate,
/// not merely zero, variance estimate: there is no discordance to estimate
/// a nuisance parameter from.
fn paired_per_pair_variance(b: u64, c: u64, n: u64) -> Option<f64> {
    if b.saturating_add(c) == 0 {
        return None;
    }
    let n_f = n as f64;
    let point = (c as f64 - b as f64) / n_f;
    let variance = (b as f64 + c as f64) / n_f - point * point;
    (variance > 0.0).then_some(variance)
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

    // ---- required_n_paired / minimum_detectable_effect_paired ---------------
    //
    // Reference values computed against `llm-power`'s (arXiv:2605.30315)
    // `parametric_required_n_paired_binary(p1, p2, rho)` — the tool's own
    // docstring names this "the correct paired-Bernoulli formula" as opposed
    // to its sibling `parametric_required_n_proportions(paired=True)`
    // shortcut. Each (p1, p2, rho) case is converted to an equivalent
    // (b, c, n) discordant-count triple via the standard 2x2-table identity
    // for independent/correlated Bernoulli pairs (a = p1*p2 + rho*sqrt(p1(1-p1)p2(1-p2)),
    // b = p1-a, c = p2-a) so the SAME case pins both parameterizations.

    #[test]
    fn required_n_paired_matches_the_reference_tool_at_zero_correlation() {
        // p1=0.6, p2=0.5, rho=0 (independent Bernoulli): llm-power's
        // parametric_required_n_paired_binary(0.6, 0.5, rho=0.0) == 385.
        // At rho=0 this also collapses to the simple additive-variance case
        // Var(D) = p1(1-p1) + p2(1-p2) = 0.24 + 0.25 = 0.49.
        assert_eq!(required_n_paired(3000, 2000, 10000, 0.05, 0.80), Some(385));
    }

    #[test]
    fn required_n_paired_matches_the_reference_tool_with_positive_correlation() {
        // p1=0.6, p2=0.5, rho=0.6: llm-power's
        // parametric_required_n_paired_binary(0.6, 0.5, rho=0.6) == 154.
        assert_eq!(
            required_n_paired(15303, 5303, 100000, 0.05, 0.80),
            Some(154)
        );
    }

    #[test]
    fn required_n_paired_matches_the_reference_tool_not_the_unpaired_shortcut() {
        // Same (p1=0.6, p2=0.5, rho=0.6) case as above, but checked against
        // BOTH published llm-power numbers to pin the ~2x gap the card
        // names: the unpaired Cohen's-h-times-(1-rho) shortcut
        // (`parametric_required_n_proportions(paired=True)`) gives 78 for
        // this case; the correct paired formula gives 154 — almost exactly
        // double. `required_n_paired` must land on the correct (larger)
        // value, not the shortcut's.
        let correct = required_n_paired(15303, 5303, 100000, 0.05, 0.80).unwrap();
        assert_eq!(correct, 154);
        let shortcut_n = 78u64;
        assert!(
            correct > shortcut_n,
            "the correct paired N ({correct}) must exceed the unpaired shortcut's ({shortcut_n})"
        );
        assert!(
            (correct as f64 / shortcut_n as f64 - 2.0).abs() < 0.1,
            "the gap between the correct formula and the unpaired shortcut should be close to \
             the ~2x this card names, got ratio {}",
            correct as f64 / shortcut_n as f64
        );
    }

    #[test]
    fn required_n_paired_is_none_for_a_zero_observed_effect() {
        // b == c: the observed imbalance is exactly zero, so no finite N
        // resolves it — never a fabricated Some(0) or Some(u64::MAX).
        assert_eq!(required_n_paired(50, 50, 1000, 0.05, 0.80), None);
    }

    #[test]
    fn required_n_paired_is_none_with_no_discordant_pairs() {
        // b + c == 0: perfect agreement on every task, no discordance to
        // estimate a variance from.
        assert_eq!(required_n_paired(0, 0, 1000, 0.05, 0.80), None);
    }

    #[test]
    fn required_n_paired_rejects_degenerate_input() {
        assert_eq!(required_n_paired(1, 9, 0, 0.05, 0.80), None); // n = 0
        assert_eq!(required_n_paired(1, 9, 10, 0.0, 0.80), None); // alpha = 0
        assert_eq!(required_n_paired(1, 9, 10, 0.05, 0.0), None); // power = 0
    }

    #[test]
    fn minimum_detectable_effect_paired_matches_the_reference_tool() {
        let mde = minimum_detectable_effect_paired(3000, 2000, 10000, 0.05, 0.80).unwrap();
        assert!((mde - 0.019_611_097).abs() < 1e-6, "{mde}");

        let mde2 = minimum_detectable_effect_paired(15303, 5303, 100000, 0.05, 0.80).unwrap();
        assert!((mde2 - 0.003_922_820).abs() < 1e-6, "{mde2}");
    }

    #[test]
    fn minimum_detectable_effect_paired_shrinks_as_n_grows() {
        // More paired trials at the same discordance rate can resolve a
        // smaller effect — scale (b, c) up proportionally with n.
        let small_n = minimum_detectable_effect_paired(30, 20, 100, 0.05, 0.80).unwrap();
        let big_n = minimum_detectable_effect_paired(3000, 2000, 10000, 0.05, 0.80).unwrap();
        assert!(
            big_n < small_n,
            "more trials at the same discordance rate should lower the MDE: {big_n} vs {small_n}"
        );
    }

    #[test]
    fn minimum_detectable_effect_paired_is_none_with_no_discordant_pairs() {
        assert_eq!(
            minimum_detectable_effect_paired(0, 0, 1000, 0.05, 0.80),
            None
        );
    }

    #[test]
    fn minimum_detectable_effect_paired_rejects_degenerate_input() {
        assert_eq!(minimum_detectable_effect_paired(1, 9, 0, 0.05, 0.80), None);
        assert_eq!(minimum_detectable_effect_paired(1, 9, 10, 1.0, 0.80), None);
    }

    #[test]
    fn required_n_paired_and_mde_agree_at_the_boundary() {
        // Self-consistency: at exactly the required N for a given (b, c, n)'s
        // own observed effect, the MDE computed at that N should be very
        // close to the observed |point| effect itself (required_n solves N
        // given delta; MDE solves delta given N — they invert each other).
        let (b, c, n) = (3000u64, 2000u64, 10000u64);
        let required = required_n_paired(b, c, n, 0.05, 0.80).unwrap();
        // Rescale (b, c) proportionally to the required N, holding the
        // discordance RATE fixed, to ask "at this required sample size, is
        // the MDE back at (approximately) the original observed effect?"
        let scale = required as f64 / n as f64;
        let b_at_required = (b as f64 * scale).round() as u64;
        let c_at_required = (c as f64 * scale).round() as u64;
        let mde_at_required =
            minimum_detectable_effect_paired(b_at_required, c_at_required, required, 0.05, 0.80)
                .unwrap();
        let observed_point = (c as f64 - b as f64) / n as f64;
        assert!(
            (mde_at_required - observed_point.abs()).abs() < 1e-2,
            "MDE at the required N ({mde_at_required}) should recover close to the original \
             observed effect ({}), abs diff too large",
            observed_point.abs()
        );
    }
}
