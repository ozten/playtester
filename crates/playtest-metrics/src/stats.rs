//! In-house statistical primitives for Phase 6 compare.
//!
//! Four pure functions with uniform shape: Welch's t-test,
//! two-proportion z-test, Benjamini–Hochberg FDR, Bonferroni. All
//! operate via the standard-normal CDF (Abramowitz & Stegun 26.2.17
//! rational approximation, max error ≈ 7.5e-8). p-values from Welch
//! use a Normal approximation to the t-distribution — accurate for
//! n ≥ 30 per cohort, which is where R6.8 lands at 10K games.
//!
//! The primitives are deliberately untypewrapped around game data:
//! they accept plain `&[f64]` / `u64` and emit a `TestOutcome` struct.
//! The compare engine (Unit 3) converts SQLite rows into these shapes
//! and back.

use serde::{Deserialize, Serialize};

/// Uniform test-outcome shape used by both Welch and the two-
/// proportion z-test. Consumers read whichever fields are meaningful
/// for their test family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestOutcome {
    /// Sample mean (Welch) or sample proportion (z-test).
    pub mean_a: f64,
    pub mean_b: f64,
    /// `mean_a - mean_b`. Positive means A is larger.
    pub delta: f64,
    /// Standard error of the delta.
    pub std_err: f64,
    /// Normal-approximation z-statistic.
    pub z_stat: f64,
    /// Two-sided p-value under the null hypothesis of equal means /
    /// proportions.
    pub p_value: f64,
    /// 95% confidence interval on `delta`. `None` when either cohort
    /// is too small for a meaningful Normal-approx CI (n < 5).
    pub ci_95_low: Option<f64>,
    pub ci_95_high: Option<f64>,
    pub n_a: u64,
    pub n_b: u64,
}

/// Errors the stats primitives can return. Conditions that cannot
/// produce a meaningful test statistic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatsError {
    #[error("cohort A is empty")]
    EmptySampleA,
    #[error("cohort B is empty")]
    EmptySampleB,
    #[error("cohort A has zero trials (n_a = 0)")]
    ZeroTrialsA,
    #[error("cohort B has zero trials (n_b = 0)")]
    ZeroTrialsB,
    #[error("wins exceed trials in cohort {cohort}: {wins} > {trials}")]
    WinsExceedTrials {
        cohort: char,
        wins: u64,
        trials: u64,
    },
    #[error("cohort A variance is zero and B is non-zero (or vice-versa); cannot compute Welch")]
    ZeroVariance,
}

/// Standard-normal CDF via Abramowitz & Stegun 26.2.17. Accurate to
/// ≈ 7.5e-8 across the real line.
///
/// `Φ(x) = 1 − φ(x)·(a₁k + a₂k² + a₃k³ + a₄k⁴ + a₅k⁵)` for x ≥ 0,
/// where k = 1/(1 + 0.2316419·x) and φ is the standard-normal pdf.
/// For x < 0 we use symmetry: Φ(x) = 1 − Φ(−x).
// Constants for Abramowitz & Stegun 26.2.17 rational approximation
// to the standard-normal CDF. Max absolute error ≈ 7.5e-8.
const CDF_A1: f64 = 0.319_381_530;
const CDF_A2: f64 = -0.356_563_782;
const CDF_A3: f64 = 1.781_477_937;
const CDF_A4: f64 = -1.821_255_978;
const CDF_A5: f64 = 1.330_274_429;
const CDF_B: f64 = 0.231_641_9;

#[must_use]
pub fn standard_normal_cdf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let abs_x = x.abs();
    let k = 1.0 / (1.0 + CDF_B * abs_x);
    let pdf = (1.0 / (2.0 * core::f64::consts::PI).sqrt()) * (-0.5 * abs_x * abs_x).exp();
    let cdf_pos = 1.0
        - pdf
            * (CDF_A1 * k
                + CDF_A2 * k * k
                + CDF_A3 * k.powi(3)
                + CDF_A4 * k.powi(4)
                + CDF_A5 * k.powi(5));
    if x >= 0.0 { cdf_pos } else { 1.0 - cdf_pos }
}

/// Two-sided p-value from a z-statistic under the standard Normal.
/// `p = 2 · (1 − Φ(|z|))`.
#[must_use]
pub fn two_sided_p_from_z(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    2.0 * (1.0 - standard_normal_cdf(z.abs()))
}

/// Welch's t-test on two samples. Uses a Normal approximation to the
/// t-distribution for the p-value (accurate at n ≥ 30 per cohort).
///
/// # Errors
/// Returns `StatsError::EmptySampleA` / `EmptySampleB` if either
/// sample has zero elements.
pub fn welch_t_test(a: &[f64], b: &[f64]) -> Result<TestOutcome, StatsError> {
    if a.is_empty() {
        return Err(StatsError::EmptySampleA);
    }
    if b.is_empty() {
        return Err(StatsError::EmptySampleB);
    }
    let n_a = a.len() as u64;
    let n_b = b.len() as u64;
    #[allow(clippy::cast_precision_loss)]
    let nfa = n_a as f64;
    #[allow(clippy::cast_precision_loss)]
    let nfb = n_b as f64;
    let mean_a = a.iter().sum::<f64>() / nfa;
    let mean_b = b.iter().sum::<f64>() / nfb;

    // Sample variance with n-1 denominator. Singleton samples get
    // variance 0 by convention; std_err degenerates to sqrt(var_other/n)
    // in that case, and the p-value is still well-defined.
    let var_a = if n_a > 1 {
        let ss: f64 = a.iter().map(|x| (x - mean_a).powi(2)).sum();
        ss / (nfa - 1.0)
    } else {
        0.0
    };
    let var_b = if n_b > 1 {
        let ss: f64 = b.iter().map(|x| (x - mean_b).powi(2)).sum();
        ss / (nfb - 1.0)
    } else {
        0.0
    };

    let delta = mean_a - mean_b;
    let std_err = (var_a / nfa + var_b / nfb).sqrt();
    let (z_stat, p_value) = if std_err == 0.0 {
        // Both samples constant. p = 1 if identical, 0 if different.
        let p = if delta == 0.0 { 1.0 } else { 0.0 };
        (0.0, p)
    } else {
        let t = delta / std_err;
        (t, two_sided_p_from_z(t))
    };

    // 95% CI on delta via Normal approximation. Suppress when either
    // cohort is too small for the approximation to be meaningful.
    let (ci_low, ci_high) = if n_a >= 5 && n_b >= 5 && std_err > 0.0 {
        let half = 1.96 * std_err;
        (Some(delta - half), Some(delta + half))
    } else {
        (None, None)
    };

    Ok(TestOutcome {
        mean_a,
        mean_b,
        delta,
        std_err,
        z_stat,
        p_value,
        ci_95_low: ci_low,
        ci_95_high: ci_high,
        n_a,
        n_b,
    })
}

/// Two-proportion z-test. Null hypothesis: `p_a == p_b`. Uses a
/// pooled standard error under the null for the z-statistic and an
/// unpooled SE for the CI on the delta (standard practice).
///
/// # Errors
/// - `StatsError::ZeroTrialsA` / `ZeroTrialsB` if either trial count
///   is zero.
/// - `StatsError::WinsExceedTrials` if `wins > trials` in either
///   cohort.
pub fn two_proportion_z_test(
    wins_a: u64,
    n_a: u64,
    wins_b: u64,
    n_b: u64,
) -> Result<TestOutcome, StatsError> {
    if n_a == 0 {
        return Err(StatsError::ZeroTrialsA);
    }
    if n_b == 0 {
        return Err(StatsError::ZeroTrialsB);
    }
    if wins_a > n_a {
        return Err(StatsError::WinsExceedTrials {
            cohort: 'A',
            wins: wins_a,
            trials: n_a,
        });
    }
    if wins_b > n_b {
        return Err(StatsError::WinsExceedTrials {
            cohort: 'B',
            wins: wins_b,
            trials: n_b,
        });
    }

    #[allow(clippy::cast_precision_loss)]
    let (nfa, nfb) = (n_a as f64, n_b as f64);
    #[allow(clippy::cast_precision_loss)]
    let (wa, wb) = (wins_a as f64, wins_b as f64);
    let p_a = wa / nfa;
    let p_b = wb / nfb;
    let delta = p_a - p_b;

    // Pooled SE under the null for the z-statistic.
    let p_pool = (wa + wb) / (nfa + nfb);
    let pooled_se = (p_pool * (1.0 - p_pool) * (1.0 / nfa + 1.0 / nfb)).sqrt();
    let (z_stat, p_value) = if pooled_se == 0.0 {
        // Both cohorts are all-win or all-loss identically → p=1.
        let p = if delta == 0.0 { 1.0 } else { 0.0 };
        (0.0, p)
    } else {
        let z = delta / pooled_se;
        (z, two_sided_p_from_z(z))
    };

    // Unpooled SE for the CI on delta (no null assumption).
    let unpooled_se = (p_a * (1.0 - p_a) / nfa + p_b * (1.0 - p_b) / nfb).sqrt();
    let (ci_low, ci_high) = if n_a >= 5 && n_b >= 5 && unpooled_se > 0.0 {
        let half = 1.96 * unpooled_se;
        (Some(delta - half), Some(delta + half))
    } else {
        (None, None)
    };

    Ok(TestOutcome {
        mean_a: p_a,
        mean_b: p_b,
        delta,
        std_err: pooled_se,
        z_stat,
        p_value,
        ci_95_low: ci_low,
        ci_95_high: ci_high,
        n_a,
        n_b,
    })
}

/// Benjamini–Hochberg FDR correction. Returns a boolean per input
/// p-value, aligned with input order: `true` means the null is
/// rejected at FDR = α.
///
/// Algorithm: sort p-values ascending; find the largest k such that
/// `p_(k) ≤ k · α / m`; reject every p-value at rank ≤ k.
#[must_use]
pub fn benjamini_hochberg(p_values: &[f64], alpha: f64) -> Vec<bool> {
    let m = p_values.len();
    if m == 0 {
        return Vec::new();
    }
    // Rank-by-index: (original_idx, p_value), sorted ascending by p.
    let mut ranked: Vec<(usize, f64)> = p_values.iter().copied().enumerate().collect();
    ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal));

    // Find the largest k with p_(k) <= k * alpha / m (1-indexed).
    #[allow(clippy::cast_precision_loss)]
    let m_f = m as f64;
    let mut k_max: Option<usize> = None;
    for (rank_0, (_, p)) in ranked.iter().enumerate() {
        let rank_1 = rank_0 + 1;
        #[allow(clippy::cast_precision_loss)]
        let threshold = (rank_1 as f64) * alpha / m_f;
        if *p <= threshold {
            k_max = Some(rank_1);
        }
    }

    let mut reject = vec![false; m];
    if let Some(k) = k_max {
        for (_, (orig_idx, _)) in ranked.iter().enumerate().take(k) {
            reject[*orig_idx] = true;
        }
    }
    reject
}

/// Bonferroni correction. Returns `p_i < α / m` per test.
#[must_use]
pub fn bonferroni(p_values: &[f64], alpha: f64) -> Vec<bool> {
    let m = p_values.len();
    if m == 0 {
        return Vec::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let threshold = alpha / (m as f64);
    p_values.iter().map(|p| *p < threshold).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: float-equality up to N decimal places.
    fn approx_eq(a: f64, b: f64, sig: i32) -> bool {
        let tol = 10f64.powi(-sig);
        (a - b).abs() < tol
    }

    // -----------------------------------------------------------------
    // Normal CDF reference values (Abramowitz & Stegun 26.2.17)
    // -----------------------------------------------------------------

    #[test]
    fn standard_normal_cdf_reference_values() {
        // Known reference values to ≥ 5 decimal places.
        assert!(approx_eq(standard_normal_cdf(0.0), 0.5, 5));
        assert!(approx_eq(standard_normal_cdf(1.0), 0.841_344_7, 5));
        assert!(approx_eq(standard_normal_cdf(1.96), 0.975_002, 5));
        assert!(approx_eq(standard_normal_cdf(-1.96), 0.024_998, 5));
        assert!(approx_eq(standard_normal_cdf(2.5758), 0.995, 4));
        assert!(approx_eq(standard_normal_cdf(-2.0), 0.022_750, 4));
    }

    #[test]
    fn two_sided_p_value_at_common_z_values() {
        // z=1.96 → p≈0.05; z=2.5758 → p≈0.01.
        assert!(approx_eq(two_sided_p_from_z(1.96), 0.05, 3));
        assert!(approx_eq(two_sided_p_from_z(2.5758), 0.01, 3));
        assert!(approx_eq(two_sided_p_from_z(0.0), 1.0, 5));
    }

    // -----------------------------------------------------------------
    // Welch's t-test reference values
    // -----------------------------------------------------------------

    #[test]
    fn welch_identical_vectors_return_p_one() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = a.clone();
        let out = welch_t_test(&a, &b).unwrap();
        assert!(approx_eq(out.delta, 0.0, 10));
        assert!(approx_eq(out.p_value, 1.0, 5));
        assert_eq!(out.n_a, 5);
        assert_eq!(out.n_b, 5);
    }

    #[test]
    fn welch_with_large_n_matches_normal_approx() {
        // Two cohorts, each n=100, means differ by roughly 1 sd_pooled.
        // Construct samples with known mean and variance: a = 0, b = 0.2,
        // both with variance 1 (approximately).
        // With n=100 per side, SE_delta = sqrt(1/100 + 1/100) = sqrt(0.02)
        // ≈ 0.1414. z = 0.2 / 0.1414 ≈ 1.414. p ≈ 2*(1 − Φ(1.414)) ≈ 0.157.
        let a: Vec<f64> = (0..100)
            .map(|i| {
                let x = f64::from(i) - 49.5;
                x / 28.577_380_332_470_415 // rescale to approximately unit variance
            })
            .collect();
        let b: Vec<f64> = a.iter().map(|x| x + 0.2).collect();
        let out = welch_t_test(&a, &b).unwrap();
        assert!(approx_eq(out.mean_a, 0.0, 5));
        assert!(approx_eq(out.mean_b, 0.2, 5));
        assert!(approx_eq(out.delta, -0.2, 5));
        // SE should be close to sqrt(2/100) ≈ 0.1414.
        assert!(approx_eq(out.std_err, 0.141_42, 2));
        // z near −1.414, p near 0.157.
        assert!(out.z_stat < -1.3 && out.z_stat > -1.6);
        assert!(out.p_value > 0.1 && out.p_value < 0.2);
        assert!(out.ci_95_low.is_some());
        assert!(out.ci_95_high.is_some());
    }

    #[test]
    fn welch_clear_signal_produces_small_p() {
        // Two cohorts with mean delta >> SE: cohort A has 100 values
        // around 0, cohort B has 100 values around 1 (unit variance).
        // SE_delta ≈ sqrt(2/100) = 0.1414; z ≈ −7.07; p essentially 0.
        let a: Vec<f64> = (0..100).map(|i| (f64::from(i) - 49.5) / 28.577).collect();
        let b: Vec<f64> = a.iter().map(|x| x + 1.0).collect();
        let out = welch_t_test(&a, &b).unwrap();
        assert!(out.p_value < 0.001);
    }

    #[test]
    fn welch_small_n_suppresses_ci() {
        // n=3 per side; CI must be None.
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let out = welch_t_test(&a, &b).unwrap();
        assert!(out.ci_95_low.is_none());
        assert!(out.ci_95_high.is_none());
    }

    #[test]
    fn welch_empty_a_returns_error() {
        let err = welch_t_test(&[], &[1.0, 2.0]).unwrap_err();
        assert!(matches!(err, StatsError::EmptySampleA));
    }

    #[test]
    fn welch_empty_b_returns_error() {
        let err = welch_t_test(&[1.0, 2.0], &[]).unwrap_err();
        assert!(matches!(err, StatsError::EmptySampleB));
    }

    // -----------------------------------------------------------------
    // Two-proportion z-test reference values
    // -----------------------------------------------------------------

    #[test]
    fn two_proportion_z_reference_60_vs_80() {
        // Hand-computed: p_pool = 140/200 = 0.7; SE = sqrt(0.7·0.3·(1/100+1/100))
        //               = sqrt(0.0042) ≈ 0.06481.
        // delta = 0.6 − 0.8 = −0.2; z = −0.2 / 0.06481 ≈ −3.086.
        // Two-sided p ≈ 0.00203.
        let out = two_proportion_z_test(60, 100, 80, 100).unwrap();
        assert!(approx_eq(out.mean_a, 0.6, 5));
        assert!(approx_eq(out.mean_b, 0.8, 5));
        assert!(approx_eq(out.delta, -0.2, 5));
        assert!(approx_eq(out.std_err, 0.064_81, 3));
        assert!(approx_eq(out.z_stat, -3.086, 2));
        assert!(out.p_value < 0.003 && out.p_value > 0.001);
        assert!(out.ci_95_low.is_some());
    }

    #[test]
    fn two_proportion_z_equal_proportions_yield_p_one() {
        let out = two_proportion_z_test(50, 100, 50, 100).unwrap();
        assert!(approx_eq(out.delta, 0.0, 10));
        assert!(approx_eq(out.p_value, 1.0, 5));
    }

    #[test]
    fn two_proportion_z_zero_trials_returns_error() {
        let err = two_proportion_z_test(0, 0, 50, 100).unwrap_err();
        assert!(matches!(err, StatsError::ZeroTrialsA));
        let err = two_proportion_z_test(50, 100, 0, 0).unwrap_err();
        assert!(matches!(err, StatsError::ZeroTrialsB));
    }

    #[test]
    fn two_proportion_z_wins_exceed_trials_returns_error() {
        let err = two_proportion_z_test(200, 100, 50, 100).unwrap_err();
        match err {
            StatsError::WinsExceedTrials { cohort, wins, trials } => {
                assert_eq!(cohort, 'A');
                assert_eq!(wins, 200);
                assert_eq!(trials, 100);
            }
            _ => panic!("expected WinsExceedTrials"),
        }
    }

    // -----------------------------------------------------------------
    // BH / Bonferroni reference values
    // -----------------------------------------------------------------

    fn textbook_pvalues() -> Vec<f64> {
        vec![0.001, 0.008, 0.039, 0.041, 0.042, 0.060, 0.074, 0.205]
    }

    #[test]
    fn benjamini_hochberg_textbook_example_rejects_first_two() {
        // p = [0.001, 0.008, 0.039, ...], m = 8, alpha = 0.05.
        // Thresholds at ranks (k/m)·α: 0.00625, 0.0125, 0.01875, ...
        // k=1: p=0.001 ≤ 0.00625 ✓
        // k=2: p=0.008 ≤ 0.0125 ✓
        // k=3: p=0.039 ≤ 0.01875 ✗
        // Largest satisfying k = 2; rejections = ranks 1..=2 = [0.001, 0.008].
        let reject = benjamini_hochberg(&textbook_pvalues(), 0.05);
        assert_eq!(reject, vec![true, true, false, false, false, false, false, false]);
    }

    #[test]
    fn benjamini_hochberg_preserves_input_order_with_unsorted_input() {
        // The same set of p-values, but permuted on input. Rejections
        // should map back to the same *original* indices.
        let p = vec![0.041, 0.001, 0.060, 0.008, 0.042, 0.205, 0.039, 0.074];
        let reject = benjamini_hochberg(&p, 0.05);
        // Only p=0.001 (index 1) and p=0.008 (index 3) are rejected.
        assert_eq!(reject, vec![false, true, false, true, false, false, false, false]);
    }

    #[test]
    fn benjamini_hochberg_empty_input_returns_empty() {
        assert_eq!(benjamini_hochberg(&[], 0.05), Vec::<bool>::new());
    }

    #[test]
    fn benjamini_hochberg_rejects_none_when_all_p_large() {
        let p = vec![0.5, 0.7, 0.9];
        let reject = benjamini_hochberg(&p, 0.05);
        assert_eq!(reject, vec![false, false, false]);
    }

    #[test]
    fn benjamini_hochberg_rejects_all_when_all_p_tiny() {
        let p = vec![0.0001, 0.0002, 0.0003];
        let reject = benjamini_hochberg(&p, 0.05);
        assert_eq!(reject, vec![true, true, true]);
    }

    #[test]
    fn bonferroni_textbook_example_rejects_only_first() {
        // α/m = 0.05 / 8 = 0.00625. Only p=0.001 < 0.00625.
        let reject = bonferroni(&textbook_pvalues(), 0.05);
        assert_eq!(reject, vec![true, false, false, false, false, false, false, false]);
    }

    #[test]
    fn bonferroni_empty_input_returns_empty() {
        assert_eq!(bonferroni(&[], 0.05), Vec::<bool>::new());
    }
}
