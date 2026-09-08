//! Wilson Score Interval (Wilson, 1927).
//!
//! Provides confidence intervals for binomial proportions that are
//! well-behaved even with small samples or extreme proportions.

/// Two-sided standard-normal critical value for any finite confidence in (0,1).
/// Evaluating the lower tail avoids rounding (1 + confidence)/2 to exactly one.
/// Invalid confidence returns NaN; checked intervals reject it.
pub fn z_value(confidence: f64) -> f64 {
    if !confidence.is_finite() || confidence <= 0.0 || confidence >= 1.0 {
        return f64::NAN;
    }
    -super::power::normal_quantile((1.0 - confidence) / 2.0)
}

/// Compute Wilson score confidence interval for a binomial proportion.
///
/// Returns (lower, upper) bounds.
///
/// - `successes`: number of successes
/// - `total`: total number of trials (must be > 0)
/// - `confidence`: confidence level (e.g., 0.95 for 95% CI)
pub fn wilson_interval(successes: u64, total: u64, confidence: f64) -> (f64, f64) {
    try_wilson_interval(successes, total, confidence).unwrap_or((0.0, 1.0))
}

/// Validated interval. No trials, impossible counts, or invalid confidence are errors.
/// The legacy wrapper returns an uninformative [0,1] interval for these inputs.
pub fn try_wilson_interval(
    successes: u64,
    total: u64,
    confidence: f64,
) -> Result<(f64, f64), &'static str> {
    if total == 0 {
        return Err("Wilson interval requires at least one trial");
    }
    if successes > total {
        return Err("Wilson successes exceed total trials");
    }
    let z = z_value(confidence);
    if !z.is_finite() {
        return Err("Wilson confidence must be finite and strictly between zero and one");
    }
    let n = total as f64;
    let p = successes as f64 / n;
    let z2 = z * z;

    let denominator = n + z2;
    let center = (n * p + z2 / 2.0) / denominator;
    let margin = z * ((n * p * (1.0 - p) + z2 / 4.0) / (denominator * denominator)).sqrt();

    let lower = (center - margin).max(0.0);
    let upper = (center + margin).min(1.0);
    Ok((lower, upper))
}

/// Compute the lower bound of the Wilson score interval.
pub fn wilson_lower(successes: u64, total: u64, confidence: f64) -> f64 {
    wilson_interval(successes, total, confidence).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_confidence_matches_independent_normal_quantiles() {
        // Reference values generated with Python statistics.NormalDist.inv_cdf.
        for (confidence, expected) in [
            (0.1, 0.125661346855074),
            (0.5, 0.6744897501960817),
            (0.8, 1.2815515655446006),
            (0.95, 1.9599639845400534),
            (0.99, 2.5758293035489),
            (0.9999, 3.89059188641312),
            (0.999999999999, 7.130509892879272),
        ] {
            assert!((z_value(confidence) - expected).abs() < 1e-8);
        }
        let ordinary = wilson_interval(100, 100, 0.99);
        let stricter = wilson_interval(100, 100, 0.9999);
        assert!(stricter.0 < ordinary.0);
    }

    #[test]
    fn invalid_inputs_cannot_create_positive_evidence() {
        for confidence in [f64::NAN, f64::INFINITY, -0.1, 0.0, 1.0, 1.1] {
            assert!(try_wilson_interval(10, 10, confidence).is_err());
            assert_eq!(wilson_interval(10, 10, confidence), (0.0, 1.0));
        }
        assert!(try_wilson_interval(11, 10, 0.95).is_err());
        assert!(try_wilson_interval(0, 0, 0.95).is_err());
    }

    #[test]
    fn perfect_score_near_one() {
        let (lower, upper) = wilson_interval(100, 100, 0.95);
        assert!(lower > 0.95);
        assert!((upper - 1.0).abs() < 0.01);
    }

    #[test]
    fn zero_score_near_zero() {
        let (lower, upper) = wilson_interval(0, 100, 0.95);
        assert!(lower.abs() < 0.01);
        assert!(upper < 0.05);
    }

    #[test]
    fn high_proportion_at_99_ci() {
        // 95/100 at 99% CI should have lower > 0.85ish.
        let (lower, upper) = wilson_interval(95, 100, 0.99);
        assert!(lower > 0.85);
        assert!(upper > 0.95);
        assert!(upper <= 1.0);
    }

    #[test]
    fn large_sample() {
        // 950/1000 ≈ 0.95 with tighter interval.
        let (lower, upper) = wilson_interval(950, 1000, 0.95);
        assert!(lower > 0.93);
        assert!(upper < 0.97);
    }

    #[test]
    fn small_sample() {
        // 3/5 = 0.6, wide interval expected.
        let (lower, upper) = wilson_interval(3, 5, 0.95);
        assert!(lower > 0.15);
        assert!(lower < 0.40);
        assert!(upper > 0.75);
        assert!(upper < 0.95);
    }

    #[test]
    fn z_value_99_approx() {
        let z = z_value(0.99);
        assert!((z - 2.576).abs() < 0.001);
    }
}
