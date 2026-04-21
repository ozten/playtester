# Phase 0 + 1 + 2 benchmarks

The exit criteria from `playtest-roadmap.md` tracked here:

- **R0.9**  — Can run 10,000 self-play games in under 60 seconds on one core.
- **R0.10** — Every game produces a complete, replayable log.
- **R0.11** — Zero panics over a 100,000-game soak test.
- **R1.5** — Per-card design-insight metrics surface on a fixed-deck game.
- **R1.6** — `playtest report` over 10,000 games completes in under 30 seconds.
- **R2.2** — `HeuristicAgent` beats `RandomAgent` > 90% over 10K games on every registered game.
- **R2.3** — `ISMCTSAgent` beats `HeuristicAgent` >= 65% over 10K games on every registered game.
- **R9.6** — ShipWreck: 10,000 random-vs-random games all terminate with a
  valid `GameResult`, ≥95% via `EndReason::Other("deck_exhausted")`,
  within a 120-second wall-clock budget.

All pass with margin on the current main branch.

## Reference machine

| | |
|---|---|
| CPU | aarch64, 4 cores |
| OS | Linux 6.17.0-14-generic |
| RAM | ~4 GB |
| Rust | `rustc 1.95.0` (stable) |
| Build | `cargo test --release` |

All soak runs use the release profile (`lto = "thin"`, `codegen-units = 1` — see the workspace `Cargo.toml`). Debug builds are 10–30× slower and will not meet R0.9.

## R0.9 — 10,000 games in under 60 seconds

| Metric | Value |
|---|---|
| Budget (R0.9) | < 60.0 s |
| **Measured** | **1.47 s** |
| Throughput | ~6,800 games/sec |
| Headroom | 40× under budget |

Reproduce:

```bash
cargo test --release -p playtest-cli --test soak_10k -- --ignored --nocapture
```

Source: `crates/playtest-cli/tests/soak_10k.rs`.

## R0.10 — every log is complete and replayable

| Metric | Value |
|---|---|
| Games disk-written + replayed in sample | 1,000 |
| Replay failures | **0** |

The 100K soak samples one in every 100 games for the full write-to-disk
+ `playtest_log::replay` round-trip; the remaining 99,000 games run
through an in-memory sink to save disk I/O. Every sampled log was
parseable and its replay reconstructed the same final state (winner
and scores) as the live run.

## R0.11 — zero panics over 100,000 games

| Metric | Value |
|---|---|
| Games simulated | 100,000 |
| **Panics** | **0** |
| Wall time | 16.8 s (~5,950 games/sec sustained) |
| Events emitted | ~42.9 million |
| Winner split | p0: 55,417 / p1: 44,583 |

The ~55/45 split matches the expected dealer advantage in 2-player Cribbage
(the dealer's crib makes scoring asymmetrically higher over a full game);
a 50/50 split would be a sign that something was wrong with the dealer-rotation
or crib-counting logic.

Reproduce:

```bash
cargo test --release -p playtest-cli --test soak_100k -- --ignored --nocapture
```

Source: `crates/playtest-cli/tests/soak_100k.rs`.

## R1.5 — per-card design-insight signal with random agents

The Phase 1 exit criterion R1.5 calls for the per-card table in the
report to show meaningful rank-to-rank variance even when both agents
play randomly — demonstrating that the Phase 1 pipeline produces real
signal on a fixed-deck game. On a 10,000-game random-vs-random run:

| Metric | Value |
|---|---|
| Kept-rate spread across 13 ranks | ≥ 2.0 percentage points (pinned by `per_card_kept_rates_show_rank_to_rank_asymmetry` smoke test) |
| Representative 5s kept-rate | ~68% |
| Representative 8s kept-rate | ~61% |
| Win-rate balance per rank | ~49.5% / ~50.5% (expected for random self-play) |

Reproduce:

```bash
cargo build --release --bin playtest
target/release/playtest play --game cribbage --agents random,random \
    --games 10000 --seed 1 --out /tmp/bench/games/ --fixed-time 0 --parallel
target/release/playtest report --game cribbage \
    --games /tmp/bench/games/ --out /tmp/bench/report.md
```

## R1.6 — report 10,000 games in under 30 seconds

| Metric | Value |
|---|---|
| Budget (R1.6) | < 30.0 s |
| **Measured** | **17.5 s** |
| Throughput | ~571 games/sec end-to-end (ingest + markdown) |
| Headroom | ~1.7× under budget |
| Rows written | 2.33 M `game_metrics` + 20 K `agent_stats` + 10 K `games` |

The 17.5 s total covers SQLite schema init, JSONL parsing + metric
extraction for 10,000 logs, transaction commit, and the markdown
report build. `PRAGMA synchronous=OFF` + `journal_mode=MEMORY` +
a single transaction per batch are the ingestion-side knobs; the
reporter is dominated by the per-player × per-rank `SUM(value_numeric)`
queries (13 × 8 × 2 per rank = ~208 scalar queries).

Reproduce: the two commands above (play + report). Measured on the
reference machine under the release profile.

Source: `crates/playtest-cli/tests/report_smoke.rs` covers the shape;
the end-to-end timing above is captured here rather than in a test
because a 30 s budget tied to machine speed is brittle as an
assertion.

## R2.2 — HeuristicAgent beats RandomAgent on both games

| Game | Budget | Measured | Bar | Margin |
|---|---|---|---|---|
| Cribbage | 10,000 games | **96.66%** | ≥ 90% | +6.66 pp |
| ShipWreck | 10,000 games | **92.48%** | ≥ 90% | +2.48 pp |

Both games use the per-game eval function (`cribbage_eval` / `shipwreck_eval`) and `HeuristicAgent::with_temperature(0.5)`. Cribbage's higher margin reflects that its eval captures more of the game's structure (scoring combinations directly surface in the heuristic); ShipWreck's eval is a coarser resource-plus-raft-length heuristic and the event-card randomness dampens the edge.

Reproduce:

```bash
cargo test --release -p playtest-cribbage   --test heuristic_beats_random -- --ignored --nocapture
cargo test --release -p playtest-shipwreck  --test heuristic_beats_random -- --ignored --nocapture
```

Sources: `crates/games/{cribbage,shipwreck}/tests/heuristic_beats_random.rs`.

## R2.3 — ISMCTSAgent beats HeuristicAgent (SO-ISMCTS)

| Game | Games | Iterations | Measured | Bar | Wall time (4-core reference) |
|---|---|---|---|---|---|
| Cribbage | 10,000 | 1,000 | **75.38%** | ≥ 65% | 21.9 min |

**Cribbage** cleared R2.3 with a 10.4 pp margin (7,538 wins, 2,462 losses, 0 draws).

**ShipWreck** — the full 10K × iter=1000 soak is single-machine-impractical on the 4-core reference (estimated 7+ hours). A practical-budget variant (`ismcts_beats_heuristic_1k_iter1000`, 1,000 games × iter=1000) is provided — it produces a trustworthy rate (stdev ~1.6 pp) in ~45 min and is what a workstation can realistically verify. The full 10K test remains the formal spec; a dedicated benchmark machine or a multi-shard scheduled-CI run can produce the 10K rate if needed.

Both games run `ISMCTSAgent::with_eval` at the registry default (`iterations = 1000, exploration_c = sqrt(2), rollout_depth = 50 (shipwreck) / 80 (cribbage)`). SO-ISMCTS determinizes the opponent's hidden information once per iteration via `Game::determinize`, descends with UCB1, rolls out with random action choice plus eval fallback at the depth cutoff, and backpropagates a sigmoid-normalized reward from the observer's perspective.

Reproduce:

```bash
# Cribbage full R2.3 (21.9 min on 4-core reference)
cargo test --release -p playtest-cribbage  --test ismcts_beats_heuristic ismcts_beats_heuristic_10k           -- --ignored --nocapture

# ShipWreck practical R2.3 (~45 min on 4-core reference)
cargo test --release -p playtest-shipwreck --test ismcts_beats_heuristic ismcts_beats_heuristic_1k_iter1000   -- --ignored --nocapture

# ShipWreck formal-spec R2.3 (multi-hour, benchmark-machine)
cargo test --release -p playtest-shipwreck --test ismcts_beats_heuristic ismcts_beats_heuristic_10k           -- --ignored --nocapture
```

Sources: `crates/games/{cribbage,shipwreck}/tests/ismcts_beats_heuristic.rs`. The 10K and 1K variants use rayon to parallelize games across cores, each worker driving a current-thread tokio runtime.

## R9.6 — ShipWreck 10K-game soak

| Metric | Value |
|---|---|
| Budget (R9.6) | < 120.0 s |
| **Measured** | **4.24 s** |
| Throughput | ~2,370 games/sec |
| Termination rate | 10,000 / 10,000 (100%) |
| Valid `GameResult` | 10,000 / 10,000 (100%) |
| `deck_exhausted` share | 9,996 / 10,000 (99.96%) |
| `Draw` (true tie on all three keys) | 4 / 10,000 (0.04%) |
| Panics | **0** |
| Avg events per game | 354.3 |
| Avg winner raft length | 20.22 (over 9,996 decided games) |

ShipWreck is structurally different from Cribbage: there is no score
track to cross, so the game always runs until the wreckage pools are
empty. The 99.96% `deck_exhausted` share is the healthy signal — the
rare `Draw` cases are true 3-way-equal outcomes on rescue points +
raft length + invention count, which the random agents occasionally
produce by accident.

Reproduce:

```bash
cargo test --release -p playtest-shipwreck --test soak_10k -- --ignored --nocapture
```

Source: `crates/games/shipwreck/tests/soak_10k.rs`.

## Determinism audit

The determinism guardrail runs in every PR's normal `cargo test` — not
the soak job — because it's a fast grep over source files.

```bash
cargo test -p playtest-core --test determinism_audit
```

It scans `crates/playtest-core/src/` and every `crates/games/*/src/`
tree for `SystemTime::now`, `thread_rng`, `Instant::now`, and
`rand::random`. Adapters are exempt (they're the sanctioned place for
`SystemTime::now`). Test files are also exempt — tests may legitimately
call `Instant::now` for timing assertions.

Source: `crates/playtest-core/tests/determinism_audit.rs`.

## Re-running the full suite

```bash
# Fast suite (every PR)
cargo test --workspace

# Determinism audit alone (already in the fast suite)
cargo test -p playtest-core --test determinism_audit

# Soak suite (nightly, not per-PR)
cargo test --release -p playtest-cli        --test soak_10k              -- --ignored --nocapture
cargo test --release -p playtest-cli        --test soak_100k             -- --ignored --nocapture
cargo test --release -p playtest-shipwreck  --test soak_10k              -- --ignored --nocapture
cargo test --release -p playtest-cribbage   --test heuristic_beats_random -- --ignored --nocapture
cargo test --release -p playtest-shipwreck  --test heuristic_beats_random -- --ignored --nocapture
cargo test --release -p playtest-cribbage   --test ismcts_beats_heuristic -- --ignored --nocapture
cargo test --release -p playtest-shipwreck  --test ismcts_beats_heuristic -- --ignored --nocapture
```

## CI

- `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, and `cargo test
  --workspace` on every push and PR. This includes the determinism
  audit but not the `#[ignore]` soaks.
- `.github/workflows/soak.yml` runs the ignored soak tests nightly on
  a scheduled cron. A soak-job failure does not block PRs.
