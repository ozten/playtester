//! `playtest matchup` — play round-robin matches between a pool of
//! agents and emit a markdown win-rate matrix.
//!
//! For N agents and `--games-per-pair K`, runs `N * N * K` games total:
//! for every ordered pair `(i, j)` (including self-play on the diagonal),
//! agents[i] plays seat 0 and agents[j] plays seat 1 for K games. The
//! matrix cell `[row=i, col=j]` reports agents[i]'s raw win rate across
//! those K games. Seat-bias shows up as asymmetry between `[i][j]` and
//! `[j][i]` — we report raw rates, not Wilson-score confidence intervals
//! (that's Phase 6 territory).
//!
//! Games within a pair run in parallel via rayon with a per-worker
//! tokio current-thread runtime, matching the ismcts_beats_heuristic
//! 10K soak pattern.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_greatgyre::{GreatGyreConfig, GreatGyreGame};
use playtest_registry::agent_registry::{
    AgentBuildCtx, build_cribbage_agent, build_greatgyre_agent, build_shipwreck_agent,
    is_known_agent,
};
use playtest_registry::game_registry::{
    RegisteredGame, lookup as lookup_game, player_count_range,
};
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame};
use rayon::prelude::*;
use tokio::runtime::Builder as TokioBuilder;

/// CLI flags for the `matchup` subcommand.
#[derive(Debug, ClapArgs)]
pub struct MatchupArgs {
    /// Game name (e.g. `cribbage`, `shipwreck`).
    #[arg(long)]
    pub game: String,

    /// Comma-separated agent specs, e.g. `random,heuristic-cribbage,ismcts-cribbage:iter=500`.
    #[arg(long)]
    pub agents: String,

    /// Number of games per ordered `(i, j)` pair. Total games run is
    /// `N * N * games_per_pair` where `N = agents.len()`.
    #[arg(long, default_value_t = 100)]
    pub games_per_pair: u32,

    /// Master seed. Per-game seeds derive as `seed ^ f(pair_index, game_index)`
    /// so two runs with the same seed produce identical matrices.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,

    /// Output markdown file. Overwritten on each run.
    #[arg(long)]
    pub out: PathBuf,

    /// Phase 6: append a Bradley–Terry ratings table below the
    /// win-rate matrix. MLE strengths via the MM algorithm; 95% CIs
    /// on log-θ via parametric bootstrap (200 resamples).
    #[arg(long, default_value_t = false)]
    pub bradley_terry: bool,
}

/// Run the `matchup` command.
///
/// # Errors
/// Returns an error if the game or any agent spec is unknown, if the
/// game's player-count range doesn't admit 2 players, or if the output
/// file can't be written.
pub fn run(args: &MatchupArgs) -> Result<()> {
    let game = lookup_game(&args.game)?;
    let (min, max) = player_count_range(&game);
    if min > 2 || max < 2 {
        bail!(
            "matchup supports 2-player games only; `{}` needs {min}..={max} agents",
            args.game
        );
    }

    let agent_specs: Vec<String> = args.agents.split(',').map(str::trim).map(str::to_owned).collect();
    if agent_specs.len() < 2 {
        bail!(
            "matchup needs at least 2 agents, got {}: `{}`",
            agent_specs.len(),
            args.agents
        );
    }
    for spec in &agent_specs {
        if !is_known_agent(spec) {
            bail!("unknown agent: {spec}");
        }
    }
    if args.games_per_pair == 0 {
        bail!("--games-per-pair must be positive");
    }

    let n = agent_specs.len();
    let mut matrix = vec![vec![0.0f64; n]; n];
    let mut draws_matrix = vec![vec![0u32; n]; n];
    // Raw win counts for Bradley-Terry. `wins[i][j]` holds total i
    // wins against j summed across both seatings.
    let mut wins_matrix = vec![vec![0u64; n]; n];

    for i in 0..n {
        for j in 0..n {
            let pair_seed = args
                .seed
                .wrapping_add(u64::try_from(i * n + j).expect("pair index fits"))
                .wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let (i_wins, _j_wins, draws) = run_pair(
                &game,
                &agent_specs[i],
                &agent_specs[j],
                args.games_per_pair,
                pair_seed,
            )?;
            matrix[i][j] = f64::from(i_wins) / f64::from(args.games_per_pair);
            draws_matrix[i][j] = draws;
            wins_matrix[i][j] = u64::from(i_wins);
        }
    }

    let mut md = render_matrix(
        &args.game,
        &agent_specs,
        args.games_per_pair,
        &matrix,
        &draws_matrix,
    );
    if args.bradley_terry {
        md.push_str(&render_bradley_terry(&agent_specs, &wins_matrix));
    }
    std::fs::write(&args.out, md)
        .with_context(|| format!("writing matchup matrix to {}", args.out.display()))?;
    println!(
        "matchup {} {} agents × {} games each → {}",
        args.game,
        n,
        args.games_per_pair,
        args.out.display()
    );
    Ok(())
}

/// Play `games` games of `game` with `seat0` as player 0 and `seat1` as
/// player 1. Returns `(seat0_wins, seat1_wins, draws)`. Games run in
/// parallel; each worker spins up a current-thread tokio runtime.
fn run_pair(
    game: &RegisteredGame,
    seat0: &str,
    seat1: &str,
    games: u32,
    seed: u64,
) -> Result<(u32, u32, u32)> {
    let results: Vec<Result<Option<u8>>> = (0..games)
        .into_par_iter()
        .map(|i| {
            let per_seed = seed.wrapping_add(u64::from(i).wrapping_mul(0xBF58_476D_1CE4_E5B9));
            let rt = TokioBuilder::new_current_thread()
                .enable_all()
                .build()
                .context("building tokio runtime")?;
            rt.block_on(play_one_game(game, seat0, seat1, per_seed))
        })
        .collect();

    let mut s0 = 0u32;
    let mut s1 = 0u32;
    let mut dr = 0u32;
    for r in results {
        match r? {
            Some(0) => s0 += 1,
            Some(1) => s1 += 1,
            Some(other) => bail!("unexpected winner seat {other} (expected 0 or 1)"),
            None => dr += 1,
        }
    }
    Ok((s0, s1, dr))
}

async fn play_one_game(
    game: &RegisteredGame,
    seat0: &str,
    seat1: &str,
    seed: u64,
) -> Result<Option<u8>> {
    // Per-seat seed derivation matches the registry convention so agents
    // that consume RNG (HeuristicAgent temperature, RandomAgent choices,
    // ISMCTS tree/rollout streams) see independent bit streams.
    let mix = 0x9E37_79B9_7F4A_7C15u64;
    let s0_seed = seed ^ mix;
    let s1_seed = seed ^ mix.wrapping_mul(2);

    match game {
        RegisteredGame::Cribbage(g) => {
            let g = *g;
            let cfg = CribbageConfig;
            let a0 = build_cribbage_agent(seat0, &AgentBuildCtx::cli(s0_seed, 0))?;
            let a1 = build_cribbage_agent(seat1, &AgentBuildCtx::cli(s1_seed, 1))?;
            let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![a0, a1];
            let mut loop_ = GameLoop::new(&g, g.initial_state(seed, &cfg));
            let mut chance_rng = ProductionRng::from_seed(seed);
            let mut sink = StubGameEventSink::new();
            let result = loop_
                .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
                .await
                .map_err(|e| anyhow::anyhow!("cribbage game-loop error at seed {seed}: {e}"))?;
            Ok(result.winner)
        }
        RegisteredGame::ShipWreck(g) => {
            let g = *g;
            let cfg = ShipWreckConfig::new(2)
                .expect("2-player ShipWreckConfig is always valid");
            let a0 = build_shipwreck_agent(seat0, &AgentBuildCtx::cli(s0_seed, 0))?;
            let a1 = build_shipwreck_agent(seat1, &AgentBuildCtx::cli(s1_seed, 1))?;
            let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = vec![a0, a1];
            let mut loop_ = GameLoop::new(&g, g.initial_state(seed, &cfg));
            let mut chance_rng = ProductionRng::from_seed(seed);
            let mut sink = StubGameEventSink::new();
            let result = loop_
                .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
                .await
                .map_err(|e| anyhow::anyhow!("shipwreck game-loop error at seed {seed}: {e}"))?;
            Ok(result.winner)
        }
        RegisteredGame::GreatGyre(g) => {
            let g = *g;
            let cfg = GreatGyreConfig::new(2).expect("2-player GreatGyreConfig is always valid");
            let a0 = build_greatgyre_agent(seat0, &AgentBuildCtx::cli(s0_seed, 0))?;
            let a1 = build_greatgyre_agent(seat1, &AgentBuildCtx::cli(s1_seed, 1))?;
            let mut agents: Vec<Box<dyn Agent<GreatGyreGame>>> = vec![a0, a1];
            let mut loop_ = GameLoop::new(&g, g.initial_state(seed, &cfg));
            let mut chance_rng = ProductionRng::from_seed(seed);
            let mut sink = StubGameEventSink::new();
            let result = loop_
                .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
                .await
                .map_err(|e| anyhow::anyhow!("greatgyre game-loop error at seed {seed}: {e}"))?;
            Ok(result.winner)
        }
    }
}

/// Render a markdown table of win rates. Rows are row-agent (seat 0),
/// columns are column-agent (seat 1). Cell `[i][j]` is row-agent's win
/// rate as seat 0 against column-agent as seat 1.
fn render_matrix(
    game: &str,
    agents: &[String],
    games_per_pair: u32,
    matrix: &[Vec<f64>],
    draws: &[Vec<u32>],
) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Matchup matrix — {game}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Each cell is the row agent's win rate (as seat 0) against the column agent (as seat 1) over {games_per_pair} games. Raw win rates; no confidence intervals."
    );
    let _ = writeln!(out);

    // Header row.
    out.push_str("| seat-0 \\\\ seat-1 |");
    for col in agents {
        let _ = write!(out, " {col} |");
    }
    out.push('\n');
    out.push_str("|---|");
    for _ in agents {
        out.push_str("---|");
    }
    out.push('\n');

    // Data rows.
    for (i, row) in agents.iter().enumerate() {
        let _ = write!(out, "| {row} |");
        for j in 0..agents.len() {
            let rate = matrix[i][j];
            if draws[i][j] > 0 {
                let _ = write!(out, " {:.1}% ({}d) |", rate * 100.0, draws[i][j]);
            } else {
                let _ = write!(out, " {:.1}% |", rate * 100.0);
            }
        }
        out.push('\n');
    }

    out.push('\n');
    out.push_str("Cells marked `(Nd)` include N draws, excluded from the win-rate numerator.\n");
    out
}

use playtest_metrics::{BradleyTerryInput, BradleyTerryOpts, bradley_terry_mle};

/// Render a Bradley–Terry ratings table from a symmetric win matrix.
/// Agents are sorted by `θ̂` descending; ties broken by name asc.
/// Returns a markdown fragment ready to append to the matrix output.
fn render_bradley_terry(agents: &[String], wins: &[Vec<u64>]) -> String {
    use core::fmt::Write as _;

    // Collapse wins[i][j] across both seatings.
    let n = agents.len();
    let mut symmetric = vec![vec![0u64; n]; n];
    for i in 0..n {
        for j in 0..n {
            if i != j {
                symmetric[i][j] = wins[i][j];
            }
        }
    }
    let input = BradleyTerryInput {
        agent_names: agents.to_vec(),
        wins: symmetric,
    };
    let ratings = match bradley_terry_mle(&input, &BradleyTerryOpts::default()) {
        Ok(r) => r,
        Err(e) => {
            return format!("\n### Bradley–Terry ratings\n\n*Bradley–Terry fit failed: {e}*\n");
        }
    };

    // Sort by θ̂ desc, ties by name asc.
    let mut sorted = ratings;
    sorted.sort_by(|a, b| {
        b.theta
            .partial_cmp(&a.theta)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut out = String::new();
    out.push_str("\n### Bradley–Terry ratings\n\n");
    out.push_str("Strengths θ̂ (geometric mean normalized to 1). 95% CI is on **log θ** via parametric bootstrap (200 resamples).\n\n");
    out.push_str("| rank | agent | θ̂ | log θ 95% CI |\n");
    out.push_str("| ---- | ----- | -- | ------------ |\n");
    for (rank, r) in sorted.iter().enumerate() {
        let ci = match (r.log_theta_ci_low, r.log_theta_ci_high) {
            (Some(lo), Some(hi)) => format!("[{lo:+.2}, {hi:+.2}]"),
            _ => "—".to_owned(),
        };
        let _ = writeln!(
            out,
            "| {rank} | {name} | {theta:.3} | {ci} |",
            rank = rank + 1,
            name = r.name,
            theta = r.theta,
        );
    }
    out
}
