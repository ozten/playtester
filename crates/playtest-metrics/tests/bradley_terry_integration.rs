//! Integration-level sanity tests for Bradley–Terry ratings —
//! larger matchup matrices than the in-module unit tests, plus
//! numerical-stability checks.

use playtest_metrics::{
    BradleyTerryInput, BradleyTerryOpts, bradley_terry_mle,
};

fn input(names: &[&str], wins: Vec<Vec<u64>>) -> BradleyTerryInput {
    BradleyTerryInput {
        agent_names: names.iter().map(|s| (*s).to_owned()).collect(),
        wins,
    }
}

fn fast_opts() -> BradleyTerryOpts {
    BradleyTerryOpts {
        bootstrap_samples: 0,
        ..BradleyTerryOpts::default()
    }
}

#[test]
fn four_agent_strict_ordering_produces_monotonic_theta() {
    // A > B > C > D in a strict dominance chain.
    let wins = vec![
        vec![0, 80, 90, 95],
        vec![20, 0, 70, 85],
        vec![10, 30, 0, 70],
        vec![5, 15, 30, 0],
    ];
    let ratings =
        bradley_terry_mle(&input(&["A", "B", "C", "D"], wins), &fast_opts()).unwrap();
    assert!(ratings[0].theta > ratings[1].theta);
    assert!(ratings[1].theta > ratings[2].theta);
    assert!(ratings[2].theta > ratings[3].theta);
}

#[test]
fn bootstrap_ci_width_shrinks_with_more_games() {
    // Two agents, A beats B 70/30. Repeat at n=100 and n=1000;
    // the n=1000 CI on log-θ should be tighter.
    let small = bradley_terry_mle(
        &input(&["A", "B"], vec![vec![0, 70], vec![30, 0]]),
        &BradleyTerryOpts {
            bootstrap_samples: 200,
            bootstrap_seed: 42,
            ..BradleyTerryOpts::default()
        },
    )
    .unwrap();
    let large = bradley_terry_mle(
        &input(&["A", "B"], vec![vec![0, 700], vec![300, 0]]),
        &BradleyTerryOpts {
            bootstrap_samples: 200,
            bootstrap_seed: 42,
            ..BradleyTerryOpts::default()
        },
    )
    .unwrap();
    let small_width =
        small[0].log_theta_ci_high.unwrap() - small[0].log_theta_ci_low.unwrap();
    let large_width =
        large[0].log_theta_ci_high.unwrap() - large[0].log_theta_ci_low.unwrap();
    assert!(
        large_width < small_width,
        "10× more games must produce a tighter CI: small={small_width:.3}, large={large_width:.3}"
    );
}

#[test]
fn mm_converges_on_dense_8_agent_tournament() {
    // 8-agent round-robin with all pairs playing 50 games. θ ≈ 1
    // for everyone since the matrix is perfectly symmetric.
    let n = 8;
    let mut wins = vec![vec![0_u64; n]; n];
    for (i, row) in wins.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            if i != j {
                *cell = 25; // 50 games split evenly
            }
        }
    }
    let names: Vec<String> = (0..n).map(|i| format!("agent{i}")).collect();
    let input = BradleyTerryInput {
        agent_names: names,
        wins,
    };
    let ratings = bradley_terry_mle(&input, &fast_opts()).unwrap();
    for r in &ratings {
        assert!(
            (r.theta - 1.0).abs() < 1e-5,
            "{}: θ = {}, expected ≈ 1",
            r.name,
            r.theta
        );
    }
}

#[test]
fn normalization_yields_log_theta_sum_near_zero() {
    // After geometric-mean normalization, Σlog(θ) = 0.
    let wins = vec![
        vec![0, 80, 90],
        vec![20, 0, 60],
        vec![10, 40, 0],
    ];
    let ratings =
        bradley_terry_mle(&input(&["A", "B", "C"], wins), &fast_opts()).unwrap();
    let log_sum: f64 = ratings.iter().map(|r| r.theta.ln()).sum();
    assert!(log_sum.abs() < 1e-6, "log θ sum = {log_sum}");
}
