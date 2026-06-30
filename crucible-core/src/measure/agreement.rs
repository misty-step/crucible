//! Inter-rater agreement between two boolean label vectors: raw [`agreement`]
//! and chance-corrected [`cohen_kappa`].
//!
//! Both gate a judge against a human (backlog 003), so both refuse a misaligned
//! pair: a judge/human comparison whose vectors differ in length — or are empty
//! — returns `None`, never a number that could silently cross an unlock
//! threshold.

/// Raw percent agreement between two boolean label vectors.
///
/// `Some(fraction)` of positions where `a` and `b` carry the same label — the
/// simplest judge-vs-human agreement signal (backlog 003 gates a judge on a
/// measured agreement threshold). This is raw agreement, not chance-corrected;
/// [`cohen_kappa`] is the chance-corrected companion.
///
/// Returns `None` when the slices differ in length **or** are empty. A
/// misaligned judge/human pair is a data bug, not a measurement: comparing only
/// the common prefix would invent a denominator and could hand an unlock gate a
/// fabricated rate. `None` forces the caller to handle the misalignment instead
/// of reading a silent number.
pub fn agreement(a: &[bool], b: &[bool]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    Some(matches as f64 / a.len() as f64)
}

/// Cohen's κ: chance-corrected agreement between two boolean raters.
///
/// `κ = (pₒ - pₑ) / (1 - pₑ)`, where `pₒ` is the observed [`agreement`] and `pₑ`
/// is the agreement expected if both raters labelled independently at their own
/// marginal rates. κ answers what raw agreement cannot: how much the raters
/// agree *beyond chance*. `κ = 1` is perfect agreement, `κ = 0` is exactly
/// chance, and `κ < 0` is worse than chance.
///
/// Returns `None` when the slices differ in length or are empty — the same
/// refusal as [`agreement`] — **and** when `pₑ = 1` (both raters used a single
/// label, e.g. all-`true`). There chance agreement is already total, `1 - pₑ`
/// is zero, and κ is the undefined `0 / 0`: reporting `None` keeps a degenerate
/// fixture from yielding a `NaN` or a fabricated number at an unlock gate.
///
/// ```
/// use crucible_core::cohen_kappa;
/// // Perfect agreement with both labels present → κ = 1.
/// assert_eq!(cohen_kappa(&[true, false, true], &[true, false, true]), Some(1.0));
/// // Misaligned lengths → no number that could cross a threshold.
/// assert_eq!(cohen_kappa(&[true], &[true, false]), None);
/// ```
pub fn cohen_kappa(a: &[bool], b: &[bool]) -> Option<f64> {
    // Reuse `agreement` for both the alignment refusal and the observed rate,
    // so "observed agreement" means exactly position-wise equality.
    let observed = agreement(a, b)?;
    let n = a.len() as f64;
    let a_true = a.iter().filter(|&&x| x).count() as f64 / n;
    let b_true = b.iter().filter(|&&x| x).count() as f64 / n;
    let expected = a_true * b_true + (1.0 - a_true) * (1.0 - b_true);
    let denom = 1.0 - expected;
    if denom <= 0.0 {
        // pₑ = 1: a single label was used by both raters; κ is undefined.
        return None;
    }
    Some((observed - expected) / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// Build a paired boolean dataset from a 2×2 confusion of counts.
    fn paired(
        both_true: usize,
        a_only: usize,
        b_only: usize,
        both_false: usize,
    ) -> (Vec<bool>, Vec<bool>) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for _ in 0..both_true {
            a.push(true);
            b.push(true);
        }
        for _ in 0..a_only {
            a.push(true);
            b.push(false);
        }
        for _ in 0..b_only {
            a.push(false);
            b.push(true);
        }
        for _ in 0..both_false {
            a.push(false);
            b.push(false);
        }
        (a, b)
    }

    #[test]
    fn agreement_is_one_when_labels_match() {
        assert_eq!(
            agreement(&[true, true, false], &[true, true, false]),
            Some(1.0)
        );
    }

    #[test]
    fn agreement_is_zero_when_labels_all_differ() {
        assert_eq!(
            agreement(&[true, false, true], &[false, true, false]),
            Some(0.0)
        );
    }

    #[test]
    fn agreement_is_fraction_matching() {
        // 3 of 4 positions agree.
        let a = [true, true, true, false];
        let b = [true, false, true, false];
        assert!(close(agreement(&a, &b).unwrap(), 0.75, EPS));
    }

    #[test]
    fn agreement_is_none_for_empty() {
        assert_eq!(agreement(&[], &[]), None);
    }

    #[test]
    fn agreement_is_none_on_length_mismatch() {
        // A misaligned pair must not silently produce a number: no common-prefix
        // comparison, no fabricated denominator.
        assert_eq!(agreement(&[true, true], &[true]), None);
        assert_eq!(agreement(&[true], &[true, false, true]), None);
    }

    #[test]
    fn cohen_kappa_matches_textbook_value() {
        // Classic 50-rater example (both-yes 20, A-only 5, B-only 10,
        // both-no 15): pₒ = 0.70, pₑ = 0.50, κ = 0.40.
        let (a, b) = paired(20, 5, 10, 15);
        let k = cohen_kappa(&a, &b).expect("aligned, non-degenerate");
        assert!(close(k, 0.40, 1e-9), "kappa {k} not ~0.40");
    }

    #[test]
    fn cohen_kappa_is_one_for_perfect_agreement() {
        // Both labels present and every position agrees → κ = 1.
        let (a, b) = paired(20, 0, 0, 30);
        assert_eq!(cohen_kappa(&a, &b), Some(1.0));
    }

    #[test]
    fn cohen_kappa_is_zero_at_chance() {
        // Independent raters each positive half the time: observed agreement
        // equals expected, so κ = 0. both-true 25, a-only 25, b-only 25,
        // both-false 25 over 100 items gives pₒ = 0.5 = pₑ.
        let (a, b) = paired(25, 25, 25, 25);
        let k = cohen_kappa(&a, &b).expect("aligned, non-degenerate");
        assert!(close(k, 0.0, 1e-9), "kappa {k} not ~0");
    }

    #[test]
    fn cohen_kappa_is_negative_below_chance() {
        // Systematic disagreement drives κ below zero.
        let (a, b) = paired(5, 20, 20, 5);
        let k = cohen_kappa(&a, &b).expect("aligned, non-degenerate");
        assert!(k < 0.0, "kappa {k} should be below chance");
    }

    #[test]
    fn cohen_kappa_is_none_for_single_label_marginal() {
        // Both raters all-true: chance agreement is total, κ is undefined.
        let a = [true, true, true];
        let b = [true, true, true];
        assert_eq!(cohen_kappa(&a, &b), None);
    }

    #[test]
    fn cohen_kappa_refuses_misaligned_pairs() {
        assert_eq!(cohen_kappa(&[], &[]), None);
        assert_eq!(cohen_kappa(&[true, false], &[true]), None);
    }
}
