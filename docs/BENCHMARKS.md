# Phase 0 + 1 benchmarks

The exit criteria from `playtest-roadmap.md` tracked here:

- **R0.9**  — Can run 10,000 self-play games in under 60 seconds on one core.
- **R0.10** — Every game produces a complete, replayable log.
- **R0.11** — Zero panics over a 100,000-game soak test.
- **R1.5** — Per-card design-insight metrics surface on a fixed-deck game.
- **R1.6** — `playtest report` over 10,000 games completes in under 30 seconds.

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
cargo test --release -p playtest-cli --test soak_10k   -- --ignored --nocapture
cargo test --release -p playtest-cli --test soak_100k  -- --ignored --nocapture
```

## CI

- `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, and `cargo test
  --workspace` on every push and PR. This includes the determinism
  audit but not the `#[ignore]` soaks.
- `.github/workflows/soak.yml` runs the ignored soak tests nightly on
  a scheduled cron. A soak-job failure does not block PRs.
