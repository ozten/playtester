//! Bradley–Terry MLE via Hunter (2004)'s MM (majorization-minimization)
//! algorithm. Used by `playtest matchup --bradley-terry` to produce a
//! ranked agent table with strength estimates.
//!
//! Model: `P(i beats j) = θ_i / (θ_i + θ_j)`. MLE via the iterative
//! update `θ_i ← W_i / Σ_{j≠i} N_ij / (θ_i + θ_j)`, where
//! W_i = total wins by i, N_ij = total matches i vs j. Converges
//! geometrically in ~10–20 iterations for typical matchup-matrix
//! sizes; we cap at 500 iterations with `tol = 1e-8`.
//!
//! Identifiability: θ is a ratio scale. We normalize so the geometric
//! mean of all θ is 1 (equivalently, the log-θ values sum to zero) —
//! this is the symmetric choice that makes "stronger than average" vs
//! "weaker than average" readable at a glance.
//!
//! CIs: parametric bootstrap. Resample the win-matrix cell-by-cell
//! under the MLE, re-fit, collect log-θ values, take the 2.5 / 97.5
//! percentile. Default `200` resamples — enough precision for a
//! markdown report without blowing up runtime.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// MLE outcome for one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct BradleyTerryRating {
    pub name: String,
    pub theta: f64,
    /// 95% CI bounds on log-θ. `None` when the bootstrap didn't run
    /// (e.g., the agent had zero matches).
    pub log_theta_ci_low: Option<f64>,
    pub log_theta_ci_high: Option<f64>,
}

/// Inputs to [`bradley_terry_mle`]: symmetric pair-wins matrix.
/// `wins[i][j]` = total games agent i won against agent j (across
/// all seatings). `wins[i][i]` is ignored. `agent_names` is the
/// parallel name index.
#[derive(Debug, Clone)]
pub struct BradleyTerryInput {
    pub agent_names: Vec<String>,
    pub wins: Vec<Vec<u64>>,
}

impl BradleyTerryInput {
    /// Total matches between agents i and j (symmetric).
    #[must_use]
    pub fn matches(&self, i: usize, j: usize) -> u64 {
        if i == j {
            return 0;
        }
        self.wins[i][j] + self.wins[j][i]
    }

    /// Total wins by agent i (summed over all opponents).
    #[must_use]
    pub fn agent_wins(&self, i: usize) -> u64 {
        let mut sum = 0_u64;
        for (j, row_ij) in self.wins[i].iter().enumerate() {
            if j != i {
                sum += row_ij;
            }
        }
        sum
    }

    /// Total matches agent i played (against anyone).
    #[must_use]
    pub fn agent_matches(&self, i: usize) -> u64 {
        let mut sum = 0_u64;
        for j in 0..self.wins.len() {
            if j != i {
                sum += self.matches(i, j);
            }
        }
        sum
    }
}

/// Configuration for the MM loop + bootstrap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BradleyTerryOpts {
    pub tol: f64,
    pub max_iter: usize,
    pub bootstrap_samples: usize,
    pub bootstrap_seed: u64,
}

impl Default for BradleyTerryOpts {
    fn default() -> Self {
        Self {
            tol: 1e-8,
            max_iter: 500,
            bootstrap_samples: 200,
            bootstrap_seed: 0x514E_28B7_9FC1_D24E,
        }
    }
}

/// Errors from the Bradley–Terry fit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BradleyTerryError {
    #[error("empty input: need ≥ 1 agent")]
    EmptyInput,
    #[error("agent_names length {names} does not match wins matrix dim {dim}")]
    DimensionMismatch { names: usize, dim: usize },
    #[error("wins matrix is not square: row {row} has {cols} columns")]
    NotSquare { row: usize, cols: usize },
}

/// Fit Bradley–Terry strengths via MM, then bootstrap a 95% CI on
/// log-θ. Output is ordered the same as `input.agent_names`; the
/// reporter can sort by θ̂ descending downstream.
///
/// # Errors
/// - `EmptyInput` when `agent_names` is empty.
/// - `DimensionMismatch` when `wins` doesn't match `agent_names`.
/// - `NotSquare` when rows aren't all the same length.
pub fn bradley_terry_mle(
    input: &BradleyTerryInput,
    opts: &BradleyTerryOpts,
) -> Result<Vec<BradleyTerryRating>, BradleyTerryError> {
    let n = input.agent_names.len();
    if n == 0 {
        return Err(BradleyTerryError::EmptyInput);
    }
    if input.wins.len() != n {
        return Err(BradleyTerryError::DimensionMismatch {
            names: n,
            dim: input.wins.len(),
        });
    }
    for (row_idx, row) in input.wins.iter().enumerate() {
        if row.len() != n {
            return Err(BradleyTerryError::NotSquare {
                row: row_idx,
                cols: row.len(),
            });
        }
    }

    let theta = mm_fit(&input.wins, opts.tol, opts.max_iter);
    let cis = if opts.bootstrap_samples > 0 {
        Some(bootstrap_log_theta_ci(input, &theta, opts))
    } else {
        None
    };

    let mut out: Vec<BradleyTerryRating> = Vec::with_capacity(n);
    for i in 0..n {
        let (low, high) = cis.as_ref().map_or((None, None), |cs| cs[i]);
        out.push(BradleyTerryRating {
            name: input.agent_names[i].clone(),
            theta: theta[i],
            log_theta_ci_low: low,
            log_theta_ci_high: high,
        });
    }
    Ok(out)
}

/// Core MM iteration. Always normalizes output so log-θ sums to 0
/// (geometric mean = 1).
fn mm_fit(wins: &[Vec<u64>], tol: f64, max_iter: usize) -> Vec<f64> {
    let n = wins.len();
    if n == 0 {
        return Vec::new();
    }
    let mut theta = vec![1.0_f64; n];
    // Agent wins totals (time-invariant).
    let agent_wins: Vec<f64> = (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let w = (0..n).filter(|j| *j != i).map(|j| wins[i][j]).sum::<u64>() as f64;
            w
        })
        .collect();

    for _ in 0..max_iter {
        let mut next = vec![0.0_f64; n];
        let mut converged = true;
        for i in 0..n {
            let w_i = agent_wins[i];
            if w_i == 0.0 {
                // Zero wins → θ_i collapses to 0 in the limit; pin
                // to a tiny positive value so subsequent iterations
                // don't divide by zero. Report as the minimum θ.
                next[i] = 1e-12;
                continue;
            }
            let mut denom = 0.0_f64;
            for j in 0..n {
                if j == i {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                let n_ij = (wins[i][j] + wins[j][i]) as f64;
                if n_ij == 0.0 {
                    continue;
                }
                denom += n_ij / (theta[i] + theta[j]);
            }
            if denom == 0.0 {
                next[i] = theta[i];
            } else {
                next[i] = w_i / denom;
            }
        }
        // Normalize: geometric mean = 1.
        let log_sum: f64 = next.iter().map(|t| t.max(1e-300).ln()).sum();
        #[allow(clippy::cast_precision_loss)]
        let shift = log_sum / n as f64;
        for t in &mut next {
            *t /= shift.exp();
        }
        for (new, old) in next.iter().zip(theta.iter()) {
            if (new - old).abs() > tol * old.abs().max(1.0) {
                converged = false;
            }
        }
        theta = next;
        if converged {
            break;
        }
    }
    theta
}

/// Parametric bootstrap: resample each `wins[i][j]` as
/// Binomial(n_ij, p̂_ij) where p̂_ij is the MLE win probability under
/// the fitted θ. For each resample, re-fit θ, collect log-θ_i, take
/// 2.5 / 97.5 percentiles.
fn bootstrap_log_theta_ci(
    input: &BradleyTerryInput,
    theta_mle: &[f64],
    opts: &BradleyTerryOpts,
) -> Vec<(Option<f64>, Option<f64>)> {
    let n = input.agent_names.len();
    let mut rng = StdRng::seed_from_u64(opts.bootstrap_seed);
    let mut log_thetas: Vec<Vec<f64>> = vec![Vec::with_capacity(opts.bootstrap_samples); n];

    for _ in 0..opts.bootstrap_samples {
        // Resample. For every ordered pair (i, j), i != j, sample
        // W*[i][j] ~ Binomial(wins[i][j] + wins[j][i], p̂_ij) and
        // split with W*[j][i] = total − W*[i][j].
        // We only iterate unordered pairs and assign both sides.
        let mut resampled = vec![vec![0_u64; n]; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let total = input.matches(i, j);
                if total == 0 {
                    continue;
                }
                let p = theta_mle[i] / (theta_mle[i] + theta_mle[j]).max(f64::EPSILON);
                // Inverse-CDF binomial sampling — slow but correct.
                let mut wins_ij = 0_u64;
                for _ in 0..total {
                    if rng.random::<f64>() < p {
                        wins_ij += 1;
                    }
                }
                resampled[i][j] = wins_ij;
                resampled[j][i] = total - wins_ij;
            }
        }
        let fit = mm_fit(&resampled, opts.tol, opts.max_iter);
        for (i, theta_i) in fit.iter().enumerate() {
            log_thetas[i].push(theta_i.max(1e-300).ln());
        }
    }

    // Percentiles.
    let mut out = Vec::with_capacity(n);
    for (i, theta_vec) in log_thetas.into_iter().enumerate() {
        if theta_vec.is_empty() || input.agent_matches(i) == 0 {
            out.push((None, None));
            continue;
        }
        let mut sorted = theta_vec;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        // Percentiles: (n · 0.025) and (n · 0.975). Bounded by [0, n-1].
        let n = sorted.len();
        let lo_idx = percentile_index(n, 0.025);
        let hi_idx = percentile_index(n, 0.975);
        out.push((Some(sorted[lo_idx]), Some(sorted[hi_idx])));
    }
    out
}

/// Integer index into a sorted vector of size `n` for the `q`-quantile
/// (0 < q < 1). Clamped to `[0, n-1]`. Safe for small `n` where
/// `n as f64 * q` could underflow past zero.
fn percentile_index(n: usize, q: f64) -> usize {
    if n == 0 {
        return 0;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "bootstrap sample counts stay well under 2^52"
    )]
    let nf = n as f64;
    let raw = (nf * q).floor();
    if !raw.is_finite() || raw <= 0.0 {
        return 0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "raw >= 0 guard above; sample counts fit in usize"
    )]
    let idx = raw as usize;
    idx.min(n - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(names: &[&str], wins: Vec<Vec<u64>>) -> BradleyTerryInput {
        BradleyTerryInput {
            agent_names: names.iter().map(|s| (*s).to_owned()).collect(),
            wins,
        }
    }

    fn opts_no_bootstrap() -> BradleyTerryOpts {
        BradleyTerryOpts {
            bootstrap_samples: 0,
            ..BradleyTerryOpts::default()
        }
    }

    #[test]
    fn three_symmetric_agents_yield_equal_theta() {
        // Every pair plays 100 games split 50/50. θ_A = θ_B = θ_C
        // (all ≈ 1 after geometric-mean normalization).
        let wins = vec![vec![0, 50, 50], vec![50, 0, 50], vec![50, 50, 0]];
        let ratings =
            bradley_terry_mle(&input(&["A", "B", "C"], wins), &opts_no_bootstrap()).unwrap();
        for r in &ratings {
            assert!(
                (r.theta - 1.0).abs() < 1e-6,
                "expected θ ≈ 1 for {}, got {}",
                r.name,
                r.theta
            );
        }
    }

    #[test]
    fn strict_dominance_chain_produces_monotonic_theta() {
        // A beats B beats C beats A is intransitive — not what we
        // want. Use a consistent dominance: A beats everyone, then B,
        // then C. Counts: A vs B 100/0, A vs C 100/0, B vs C 100/0.
        let wins = vec![vec![0, 100, 100], vec![0, 0, 100], vec![0, 0, 0]];
        let ratings =
            bradley_terry_mle(&input(&["A", "B", "C"], wins), &opts_no_bootstrap()).unwrap();
        assert!(ratings[0].theta > ratings[1].theta);
        assert!(ratings[1].theta > ratings[2].theta);
    }

    #[test]
    fn zero_wins_agent_gets_minimum_theta() {
        let wins = vec![vec![0, 100, 100], vec![0, 0, 100], vec![0, 0, 0]];
        let ratings =
            bradley_terry_mle(&input(&["A", "B", "C"], wins), &opts_no_bootstrap()).unwrap();
        // Agent C has zero wins — θ should be ~0 (pinned to 1e-12).
        assert!(ratings[2].theta < 1e-6);
    }

    #[test]
    fn empty_input_returns_error() {
        let empty = BradleyTerryInput {
            agent_names: vec![],
            wins: vec![],
        };
        let err = bradley_terry_mle(&empty, &opts_no_bootstrap()).unwrap_err();
        assert_eq!(err, BradleyTerryError::EmptyInput);
    }

    #[test]
    fn name_dim_mismatch_returns_error() {
        let bad = BradleyTerryInput {
            agent_names: vec!["A".into(), "B".into()],
            wins: vec![vec![0, 50, 50], vec![50, 0, 50], vec![50, 50, 0]],
        };
        let err = bradley_terry_mle(&bad, &opts_no_bootstrap()).unwrap_err();
        assert_eq!(
            err,
            BradleyTerryError::DimensionMismatch { names: 2, dim: 3 }
        );
    }

    #[test]
    fn non_square_row_returns_error() {
        let bad = BradleyTerryInput {
            agent_names: vec!["A".into(), "B".into()],
            wins: vec![vec![0, 50, 50], vec![50, 0]], // row 0 has 3 cols, row 1 has 2
        };
        let err = bradley_terry_mle(&bad, &opts_no_bootstrap()).unwrap_err();
        assert_eq!(err, BradleyTerryError::NotSquare { row: 0, cols: 3 });
    }

    #[test]
    fn bootstrap_produces_ci_bounds_for_each_agent() {
        // Use a modest number of samples to keep the test fast but
        // still exercise the code path.
        let wins = vec![vec![0, 70, 80], vec![30, 0, 60], vec![20, 40, 0]];
        let opts = BradleyTerryOpts {
            bootstrap_samples: 50,
            ..BradleyTerryOpts::default()
        };
        let ratings =
            bradley_terry_mle(&input(&["A", "B", "C"], wins), &opts).unwrap();
        for r in &ratings {
            assert!(r.log_theta_ci_low.is_some(), "{}: missing CI low", r.name);
            assert!(r.log_theta_ci_high.is_some(), "{}: missing CI high", r.name);
            assert!(
                r.log_theta_ci_low.unwrap() <= r.log_theta_ci_high.unwrap(),
                "{}: CI order violated",
                r.name
            );
        }
    }

    #[test]
    fn input_helpers_compute_symmetric_totals() {
        let wins = vec![vec![0, 60, 80], vec![40, 0, 50], vec![20, 50, 0]];
        let inp = input(&["A", "B", "C"], wins);
        // A vs B: 60 + 40 = 100 matches.
        assert_eq!(inp.matches(0, 1), 100);
        assert_eq!(inp.matches(1, 0), 100);
        // A's total wins: 60 + 80 = 140.
        assert_eq!(inp.agent_wins(0), 140);
        // A's total matches: (60+40) + (80+20) = 200.
        assert_eq!(inp.agent_matches(0), 200);
    }
}
