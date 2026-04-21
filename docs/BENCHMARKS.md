# Phase 0 benchmarks

The three Phase 0 exit criteria from `playtest-roadmap.md`:

- **R0.9** — Can run 10,000 self-play games in under 60 seconds on one core.
- **R0.10** — Every game produces a complete, replayable log.
- **R0.11** — Zero panics over a 100,000-game soak test.

All three pass with substantial margin on the current main branch.

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
