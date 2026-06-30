//! Internal numeric kernels: the normal-distribution functions the rigor
//! primitives rest on.
//!
//! These are `pub(crate)` building blocks, not public API — a caller wants a
//! p-value ([`super::paired`]) or a sample size ([`super::power`]), not a raw
//! `erfc`. They are pure, total, and never panic. Accuracy is sized to
//! p-value / sample-size work, not to a numerics library: roughly seven
//! significant figures for the tail, nine for the quantile — far finer than the
//! decisions that consume them. `std` ships neither function, so they live here
//! rather than pulling in a dependency.

/// Complementary error function `erfc(x) = 1 - erf(x)`.
///
/// Numerical Recipes' Chebyshev approximation; fractional error below `1.2e-7`
/// for every `x`. Used to turn a χ² (1 df) statistic into a tail probability:
/// `P(χ²₁ > s) = erfc(√(s / 2))`.
pub(crate) fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    // Horner form of the NR `erfcc` polynomial, inside the exponential.
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    // `tail` approximates erfc(z) for z = |x| ≥ 0, whose exact range is [0, 1].
    // The Chebyshev fit overshoots 1 by ~3e-8 right at z = 0, so cap it back
    // into the function's own range: this keeps erfc(0) exactly 1 and stops a χ²
    // survival probability built on it ([`super::paired`]) from exceeding 1.
    let tail = (t * poly.exp()).min(1.0);
    if x >= 0.0 {
        tail
    } else {
        2.0 - tail
    }
}

/// Inverse standard-normal CDF (probit): the `z` with `Φ(z) = p`.
///
/// Acklam's rational approximation. The bound it guarantees is on *relative*
/// error: below `1.15e-9` across the open interval `(0, 1)` (measured max
/// `1.1e-9`); the absolute error grows toward the tails, reaching ≈`3.3e-9`. A
/// single Halley refinement step would reach full precision, but only against a
/// high-accuracy `erf` — paired with the `~1.2e-7` [`erfc`] above it the step
/// *loses* accuracy, so it is deliberately omitted. Returns `-∞` / `+∞` at the
/// closed ends `0` / `1` rather than panicking. Used to turn a confidence level
/// and a power into the `z` quantiles a normal-approximation sample size needs.
pub(crate) fn inv_normal_cdf(p: f64) -> f64 {
    // Acklam's coefficients for the central (A, B) and tail (C, D) regions.
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383_577_518_672_69e2,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    /// Below this tail probability the rational tail branch is used.
    const P_LOW: f64 = 0.024_25;
    const P_HIGH: f64 = 1.0 - P_LOW;

    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= P_HIGH {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn erfc_matches_known_values() {
        // erfc(0) = 1, erfc(1) ≈ 0.1572992, and erfc is symmetric about 1 via
        // erfc(-x) = 2 - erfc(x).
        assert!(close(erfc(0.0), 1.0, 1e-6));
        assert!(close(erfc(1.0), 0.157_299_2, 1e-6));
        assert!(close(erfc(-1.0), 2.0 - 0.157_299_2, 1e-6));
        // Far tail stays tiny and non-negative.
        assert!(erfc(3.0) > 0.0 && erfc(3.0) < 1e-4);
    }

    #[test]
    fn inv_normal_cdf_matches_known_quantiles() {
        assert!(close(inv_normal_cdf(0.5), 0.0, 1e-9));
        assert!(close(inv_normal_cdf(0.975), 1.959_963_985, 1e-6));
        assert!(close(inv_normal_cdf(0.8), 0.841_621_234, 1e-6));
        // Symmetry: Φ⁻¹(p) = -Φ⁻¹(1 - p).
        assert!(close(inv_normal_cdf(0.025), -1.959_963_985, 1e-6));
    }

    #[test]
    fn inv_normal_cdf_is_total_at_closed_ends() {
        assert_eq!(inv_normal_cdf(0.0), f64::NEG_INFINITY);
        assert_eq!(inv_normal_cdf(1.0), f64::INFINITY);
    }

    #[test]
    fn erfc_stays_within_its_range_for_nonnegative_x() {
        // erfc(z) ∈ [0, 1] for z ≥ 0, with erfc(0) = 1 exactly. The raw
        // Chebyshev fit overshoots 1 near 0 (~1.00000003); the cap pins it so
        // the χ² survival probability built on it never exceeds 1.
        assert_eq!(erfc(0.0), 1.0);
        for i in 0..=600 {
            let z = f64::from(i) * 0.01;
            let v = erfc(z);
            assert!((0.0..=1.0).contains(&v), "erfc({z}) = {v} left [0,1]");
        }
        // The mirror identity then keeps erfc(x < 0) ∈ [1, 2].
        assert!(erfc(-0.5) >= 1.0 && erfc(-3.0) >= 1.0);
    }

    #[test]
    fn inv_normal_cdf_relative_error_matches_documented_bound() {
        // The doc claims relative error < 1.15e-9 (Acklam's real guarantee,
        // measured ~1.1e-9). Check it against high-precision quantiles spanning
        // centre and tail — this is what the bare approximation achieves with no
        // Halley refinement (which our low-accuracy `erfc` would only degrade).
        let refs = [
            (0.75, 0.674_489_750_196_081_7),
            (0.975, 1.959_963_984_540_054),
            (0.995, 2.575_829_303_548_900_4),
            (0.999, 3.090_232_306_167_813),
            (0.001, -3.090_232_306_167_813),
        ];
        for (p, z) in refs {
            let rel = (inv_normal_cdf(p) - z).abs() / z.abs();
            assert!(
                rel < 1.15e-9,
                "Φ⁻¹({p}) relative error {rel:e} exceeds 1.15e-9"
            );
        }
    }
}
