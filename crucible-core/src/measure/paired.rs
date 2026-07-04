//! Paired comparison of two configurations on the same fixtures, plus the
//! noise-floor verdict that refuses an indefensible delta.
//!
//! When configs A and B are run over one fixture set, each fixture is a *paired*
//! binary outcome (A correct?, B correct?). Concordant pairs (both right, both
//! wrong) carry no information about which config is better; only the discordant
//! pairs do. [`PairedComparison::mcnemar`] reduces the comparison to those two
//! discordant counts and asks whether their imbalance is more than noise — the
//! core of "refuse to report a delta you cannot defend".

use serde::{Deserialize, Serialize};

use super::normal::{erfc, inv_normal_cdf};

/// Discordant-pair count at or below which the exact binomial p-value is used
/// instead of the χ² approximation.
///
/// The χ² asymptotic is unreliable when few pairs are discordant; the
/// conventional cutover is ~25. Below it the exact binomial is both affordable
/// (a dozen terms) and correct.
const EXACT_MAX_DISCORDANT: u64 = 25;

/// McNemar's paired comparison of two configs' discordant outcomes.
///
/// Built from the two discordant counts via [`PairedComparison::mcnemar`]:
/// `b` = A-correct & B-wrong, `c` = A-wrong & B-correct. The null hypothesis is
/// that the two configs are equivalent, so each discordant pair is a fair coin:
/// `b` and `c` should be balanced. A large imbalance is evidence of a real
/// difference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairedComparison {
    /// Discordant count favoring A: A-correct & B-wrong.
    pub b: u64,
    /// Discordant count favoring B: A-wrong & B-correct.
    pub c: u64,
    /// The continuity-corrected χ² (1 df) statistic `(|b - c| - 1)² / (b + c)`,
    /// clamped at `0` for `|b - c| ≤ 1`, and `0` when there are no discordant
    /// pairs. Reported in both regimes as the descriptive test statistic, even
    /// when `p_value` comes from the exact binomial.
    pub statistic: f64,
    /// Two-sided p-value: the probability of an imbalance this extreme under the
    /// null. Exact binomial when `b + c` is small (see `EXACT_MAX_DISCORDANT`),
    /// the χ² tail of `statistic` otherwise. `1.0` when there are no discordant
    /// pairs (the configs were indistinguishable on every fixture).
    pub p_value: f64,
}

impl PairedComparison {
    /// Run McNemar's test from the two discordant counts.
    ///
    /// `b` = A-correct & B-wrong, `c` = A-wrong & B-correct. Concordant pairs
    /// are deliberately not arguments — they do not enter the test.
    ///
    /// Total and panic-free: `b + c == 0` (no discordant pairs) yields
    /// `statistic = 0`, `p_value = 1` — there is no evidence whatsoever of a
    /// difference, so the null stands.
    pub fn mcnemar(b: u64, c: u64) -> Self {
        let n = b + c;
        if n == 0 {
            return Self {
                b,
                c,
                statistic: 0.0,
                p_value: 1.0,
            };
        }
        // Edwards' continuity correction, clamped so |b - c| ≤ 1 is no evidence.
        let corrected = (b.abs_diff(c) as f64 - 1.0).max(0.0);
        let statistic = corrected * corrected / n as f64;
        let p_value = if n <= EXACT_MAX_DISCORDANT {
            exact_two_sided_p(b, c)
        } else {
            // χ² (1 df) survival: P(χ²₁ > s) = erfc(√(s / 2)). `erfc` already
            // caps at 1 at its source; the clamp here is a defensive backstop so
            // a reported p-value can never escape [0, 1] even if that tail
            // approximation is later changed.
            erfc((statistic / 2.0).sqrt()).clamp(0.0, 1.0)
        };
        Self {
            b,
            c,
            statistic,
            p_value,
        }
    }

    /// The noise-floor decision: is the observed delta defensible at `alpha`?
    ///
    /// Returns [`DeltaVerdict::Signal`] when `p_value <= alpha` and
    /// [`DeltaVerdict::InsideNoiseFloor`] otherwise. `alpha` is the significance
    /// threshold (conventionally `0.05`). This is the gate that refuses to
    /// report a delta the data cannot defend.
    pub fn verdict(&self, alpha: f64) -> DeltaVerdict {
        if self.p_value <= alpha {
            DeltaVerdict::Signal
        } else {
            DeltaVerdict::InsideNoiseFloor
        }
    }
}

/// The verdict on whether a paired delta clears the noise floor.
///
/// ```
/// use crucible_core::PairedComparison;
/// let cmp = PairedComparison::mcnemar(1, 9);
/// assert!(cmp.verdict(0.05).is_signal()); // p ≈ 0.021 < 0.05
/// assert_eq!(cmp.verdict(0.01).label(), "inside noise floor");
/// ```
///
/// Serializes snake_case (`"signal"` / `"inside_noise_floor"`) so a persisted
/// [`Aggregate`](crate::Aggregate) can record the verdict as data, not re-derive
/// it from a stored p-value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaVerdict {
    /// `p_value <= alpha`: the delta is distinguishable from noise — defensible.
    Signal,
    /// `p_value > alpha`: the delta sits inside the noise floor — refuse to
    /// report it as real.
    InsideNoiseFloor,
}

impl DeltaVerdict {
    /// Whether the delta cleared the noise floor.
    pub fn is_signal(self) -> bool {
        matches!(self, DeltaVerdict::Signal)
    }

    /// A short, stable label for reports: `"signal"` or `"inside noise floor"`.
    pub fn label(self) -> &'static str {
        match self {
            DeltaVerdict::Signal => "signal",
            DeltaVerdict::InsideNoiseFloor => "inside noise floor",
        }
    }
}

/// Matched-pairs rate difference over the same shared-task population that
/// drives McNemar's test.
///
/// `point` is `(c - b) / n`: positive values favor the right/comparison arm,
/// negative values favor the left/baseline arm. The interval is the standard
/// large-sample matched-pairs risk-difference interval, clipped to the feasible
/// `[-1, 1]` range:
///
/// `SE = sqrt((p_b + p_c - (p_c - p_b)^2) / n)`
///
/// where `p_b = b/n` and `p_c = c/n`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairedRateDeltaInterval {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
    pub confidence: f64,
}

/// Approximate confidence interval for a matched-pairs rate delta.
///
/// Returns a zero-width `0.0` interval when `n == 0`. `b + c <= n` is the
/// expected caller contract; debug builds assert it, while release builds clamp
/// the discordant count to keep the interval finite rather than leaking NaNs
/// into downstream artifacts.
pub fn paired_rate_delta_interval(
    b: u64,
    c: u64,
    n: usize,
    confidence: f64,
) -> PairedRateDeltaInterval {
    let confidence = if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.95
    };
    if n == 0 {
        return PairedRateDeltaInterval {
            point: 0.0,
            lower: 0.0,
            upper: 0.0,
            confidence,
        };
    }

    debug_assert!(
        b.saturating_add(c) <= n as u64,
        "paired_rate_delta_interval: discordant pairs ({}) exceed n ({n})",
        b.saturating_add(c)
    );
    let b = b.min(n as u64);
    let c = c.min((n as u64).saturating_sub(b));
    let n_f = n as f64;
    let b_f = b as f64;
    let c_f = c as f64;
    let point = (c_f - b_f) / n_f;
    let variance = ((b_f + c_f) / n_f - point * point) / n_f;
    let z = inv_normal_cdf(1.0 - (1.0 - confidence) / 2.0);
    let margin = z * variance.max(0.0).sqrt();
    PairedRateDeltaInterval {
        point,
        lower: (point - margin).max(-1.0),
        upper: (point + margin).min(1.0),
        confidence,
    }
}

/// Exact two-sided McNemar p-value: `2 · P(X ≤ min(b, c))` for
/// `X ~ Binomial(b + c, 0.5)`, capped at `1.0`.
///
/// The cumulative is accumulated through the binomial ratio recurrence
/// (`P(i) / P(i-1) = (n - i + 1) / i` at `p = 0.5`), so no factorial overflows
/// and the work is `min(b, c) + 1` multiplies.
fn exact_two_sided_p(b: u64, c: u64) -> f64 {
    let n = b + c;
    let k = b.min(c);
    let mut term = 0.5_f64.powi(n as i32); // P(X = 0)
    let mut cumulative = term;
    for i in 1..=k {
        term *= (n - i + 1) as f64 / i as f64;
        cumulative += term;
    }
    (2.0 * cumulative).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn mcnemar_exact_small_sample_matches_hand_value() {
        // b = 1, c = 9, n = 10 ≤ 25 → exact binomial.
        // 2 · (C(10,0) + C(10,1)) · 0.5^10 = 2 · 11/1024 = 0.021484375.
        let cmp = PairedComparison::mcnemar(1, 9);
        assert_eq!(cmp.b, 1);
        assert_eq!(cmp.c, 9);
        assert!(close(cmp.p_value, 0.021_484_375, 1e-9), "p {}", cmp.p_value);
        // statistic is the continuity-corrected χ² even on the exact path:
        // (|1-9| - 1)² / 10 = 49/10 = 4.9.
        assert!(close(cmp.statistic, 4.9, 1e-9), "stat {}", cmp.statistic);
    }

    #[test]
    fn mcnemar_chi_square_large_sample_matches_textbook() {
        // Wikipedia's example: b = 121, c = 59, n = 180 > 25 → χ².
        // (|121-59| - 1)² / 180 = 61² / 180 = 20.672.
        let cmp = PairedComparison::mcnemar(121, 59);
        assert!(close(cmp.statistic, 20.672, 1e-3), "stat {}", cmp.statistic);
        assert!(cmp.p_value < 1e-3, "p {} should be < 0.001", cmp.p_value);
    }

    #[test]
    fn mcnemar_no_discordant_pairs_is_no_evidence() {
        let cmp = PairedComparison::mcnemar(0, 0);
        assert_eq!(cmp.statistic, 0.0);
        assert_eq!(cmp.p_value, 1.0);
        assert_eq!(cmp.verdict(0.05), DeltaVerdict::InsideNoiseFloor);
    }

    #[test]
    fn mcnemar_equal_discordant_counts_is_no_evidence() {
        // Balanced discordance → p capped at 1, statistic clamped to 0.
        let cmp = PairedComparison::mcnemar(7, 7);
        assert_eq!(cmp.statistic, 0.0);
        assert_eq!(cmp.p_value, 1.0);
    }

    #[test]
    fn paired_rate_delta_interval_uses_the_shared_task_population() {
        let interval = paired_rate_delta_interval(1, 15, 24, 0.95);
        assert!(close(interval.point, 14.0 / 24.0, 1e-12));
        assert!(
            close(interval.lower, 0.354_768_109, 1e-6),
            "{}",
            interval.lower
        );
        assert!(
            close(interval.upper, 0.811_898_557, 1e-6),
            "{}",
            interval.upper
        );
        assert_eq!(interval.confidence, 0.95);
    }

    #[test]
    fn paired_rate_delta_interval_handles_degenerate_input() {
        let interval = paired_rate_delta_interval(0, 0, 0, 0.95);
        assert_eq!(
            interval,
            PairedRateDeltaInterval {
                point: 0.0,
                lower: 0.0,
                upper: 0.0,
                confidence: 0.95
            }
        );
    }

    #[test]
    fn mcnemar_balanced_large_sample_p_value_stays_a_probability() {
        // The χ² path (n > 25) with |b - c| ≤ 1 drives the survival function to
        // erfc(0), which the raw Chebyshev fit overshoots to ~1.00000003. This
        // is the "two configs indistinguishable" case the noise-floor refusal
        // hinges on: the p-value must stay a true probability — in [0, 1] and
        // ≈ 1 — never the > 1 that used to leak through here.
        for (b, c) in [(50, 50), (13, 13), (20, 20), (100, 100), (500, 500)] {
            let cmp = PairedComparison::mcnemar(b, c);
            assert!(
                b + c > EXACT_MAX_DISCORDANT,
                "({b},{c}) must take the χ² path"
            );
            assert!(
                (0.0..=1.0).contains(&cmp.p_value),
                "({b},{c}) p {} left [0,1]",
                cmp.p_value
            );
            assert!(
                close(cmp.p_value, 1.0, 1e-9),
                "({b},{c}) p {} not ≈ 1",
                cmp.p_value
            );
            assert_eq!(cmp.verdict(0.05), DeltaVerdict::InsideNoiseFloor);
        }
        // A one-apart imbalance on the χ² path is still no evidence (the
        // continuity correction clamps the statistic to 0) and stays in [0, 1].
        let cmp = PairedComparison::mcnemar(50, 51);
        assert!(cmp.b + cmp.c > EXACT_MAX_DISCORDANT);
        assert!((0.0..=1.0).contains(&cmp.p_value), "p {}", cmp.p_value);
        assert!(close(cmp.p_value, 1.0, 1e-9), "p {}", cmp.p_value);
    }

    #[test]
    fn mcnemar_five_one_sided_is_not_significant_at_05() {
        // 0 vs 5 discordant: exact two-sided p = 2 · 0.5^5 = 0.0625 > 0.05.
        // Five lopsided pairs are not yet a defensible delta.
        let cmp = PairedComparison::mcnemar(0, 5);
        assert!(close(cmp.p_value, 0.0625, 1e-9), "p {}", cmp.p_value);
        assert_eq!(cmp.verdict(0.05), DeltaVerdict::InsideNoiseFloor);
    }

    #[test]
    fn verdict_refuses_below_alpha_and_passes_above() {
        // p ≈ 0.0215: a signal at α = 0.05, refused at α = 0.01.
        let cmp = PairedComparison::mcnemar(1, 9);
        assert_eq!(cmp.verdict(0.05), DeltaVerdict::Signal);
        assert!(cmp.verdict(0.05).is_signal());
        assert_eq!(cmp.verdict(0.01), DeltaVerdict::InsideNoiseFloor);
        assert!(!cmp.verdict(0.01).is_signal());
        assert_eq!(cmp.verdict(0.01).label(), "inside noise floor");
        assert_eq!(cmp.verdict(0.05).label(), "signal");
    }

    #[test]
    fn mcnemar_is_symmetric_in_its_arguments() {
        // The two-sided test does not care which config is A.
        let ab = PairedComparison::mcnemar(3, 11);
        let ba = PairedComparison::mcnemar(11, 3);
        assert_eq!(ab.statistic, ba.statistic);
        assert_eq!(ab.p_value, ba.p_value);
    }

    #[test]
    fn delta_verdict_serializes_snake_case_and_round_trips() {
        assert_eq!(
            serde_json::to_string(&DeltaVerdict::Signal).unwrap(),
            "\"signal\""
        );
        assert_eq!(
            serde_json::to_string(&DeltaVerdict::InsideNoiseFloor).unwrap(),
            "\"inside_noise_floor\""
        );
        let back: DeltaVerdict = serde_json::from_str("\"inside_noise_floor\"").unwrap();
        assert_eq!(back, DeltaVerdict::InsideNoiseFloor);
    }
}
