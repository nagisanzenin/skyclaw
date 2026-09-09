//! Power Analysis for sample size estimation.
//!
//! Computes the minimum sample size needed to detect a difference
//! between two proportions with specified significance and power.

/// Validated scalar inverse CDF; no silently clamped probability endpoints.
pub(super) fn normal_quantile(p: f64) -> f64 {
    use statrs::distribution::{ContinuousCDF, Normal};
    if !p.is_finite() || p <= 0.0 || p >= 1.0 {
        return f64::NAN;
    }
    Normal::new(0.0, 1.0)
        .expect("standard normal has valid parameters")
        .inverse_cdf(p)
}

/// Approximate one-sample size for a two-sided proportion test against fixed p0.
/// Uses null and alternative variances and the ONE-sided power quantile:
/// n = [z(1-alpha/2)*sqrt(p0*(1-p0)) + z(power)*sqrt(p1*(1-p1))]^2 / (p1-p0)^2.
/// This is a normal approximation, not an exact binomial or paired A/B design.
/// Invalid inputs and zero effect return u64::MAX (no finite estimate).
pub fn min_sample_size(p0: f64, p1: f64, alpha: f64, power: f64) -> u64 {
    if ![p0, p1, alpha, power].iter().all(|x| x.is_finite())
        || !(0.0..1.0).contains(&p0)
        || p0 == 0.0
        || !(0.0..1.0).contains(&p1)
        || p1 == 0.0
        || !(0.0..1.0).contains(&alpha)
        || alpha == 0.0
        || !(0.5..1.0).contains(&power)
        || p0 == p1
    {
        return u64::MAX;
    }
    let null = (-normal_quantile(alpha / 2.0)) * (p0 * (1.0 - p0)).sqrt();
    let alternative = normal_quantile(power) * (p1 * (1.0 - p1)).sqrt();
    let estimate = ((null + alternative).powi(2) / (p1 - p0).powi(2)).ceil();
    if !estimate.is_finite() || estimate <= 0.0 {
        u64::MAX
    } else {
        estimate as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extreme_significance_cannot_round_into_zero_required_trials() {
        let ordinary = min_sample_size(0.5, 0.55, 0.05, 0.8);
        assert!(min_sample_size(0.5, 0.55, 1e-30, 0.8) > ordinary);
        assert_eq!(min_sample_size(0.5, 0.55, f64::from_bits(1), 0.8), u64::MAX);
    }

    #[test]
    fn standard_case() {
        // Independent reference: Python statistics.NormalDist().inv_cdf.
        assert!((normal_quantile(0.80) - 0.8416212336).abs() < 1e-9);
        assert!((normal_quantile(0.975) - 1.9599639845).abs() < 1e-9);
        let n = min_sample_size(0.5, 0.55, 0.05, 0.80);
        assert!((782..=784).contains(&n), "n={n}");
    }

    #[test]
    fn smaller_effect_requires_larger_n() {
        let n_large_effect = min_sample_size(0.5, 0.55, 0.05, 0.80);
        let n_small_effect = min_sample_size(0.5, 0.52, 0.05, 0.80);
        assert!(
            n_small_effect > n_large_effect,
            "smaller effect should need larger n: {} vs {}",
            n_small_effect,
            n_large_effect
        );
    }

    #[test]
    fn lower_confidence_requires_smaller_n() {
        // Lower alpha (0.10 vs 0.05) → smaller z_alpha → smaller n.
        let n_strict = min_sample_size(0.5, 0.55, 0.05, 0.80);
        let n_relaxed = min_sample_size(0.5, 0.55, 0.10, 0.80);
        assert!(
            n_relaxed < n_strict,
            "relaxed alpha should need smaller n: {} vs {}",
            n_relaxed,
            n_strict
        );
    }
}
