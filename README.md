# playtester

A deterministic Rust CLI for playtesting card and board games with agent-driven self-play, replayable event logs, and a metrics spine for design-insight reports.

## Status

Phase 0 + Phase 1 are complete — Cribbage plays end-to-end as a Random-vs-Random simulation, logs replay deterministically, and a SQLite-backed metrics ingestion + markdown report pipeline runs at 10K-game scale. Phase 2 is in flight: ShipWreck (a second, harder game: no scoring track, multi-player, event cards) has shipped through Unit 24 — full rules, event-card resolution, metrics registry, CLI integration, and a 10K-game soak test.

See [`playtest-roadmap.md`](playtest-roadmap.md) for the full roadmap and [`docs/plans/`](docs/plans/) for active implementation plans.

## Games

| Game | Players | Status | Notes |
|------|---------|--------|-------|
| [Cribbage](crates/games/cribbage/) | 2 | Shipped | 121-point standard rules. Full show/crib/pegging/nobs/nibs scoring. |
| [ShipWreck](crates/games/shipwreck/) | 2–4 | Shipped (Phase 2, Unit 24) | Custom rescue-points card game with raft extensions, equipment upgrades, and event cards (Shark / Typhoon / FlyingFish). See [`docs/shipwreck.md`](docs/shipwreck.md). |

## Build

```bash
cargo build --workspace
cargo test --workspace
```

## Quick start — play and replay Cribbage

The Phase 0 binary simulates games between programmed agents (`RandomAgent` for now), writes a JSONL event log per game, and replays them deterministically.

> **There is no human-vs-CPU interface yet.** The `Agent` trait is async-friendly so a blocking stdin `TerminalAgent` drops in cleanly — that work is Phase 3. Today you watch Random-vs-Random matches and inspect the logs.

**Run 10 games of Cribbage, random vs. random, seeded for reproducibility:**

```bash
cargo run --release -p playtest-cli -- \
  play \
    --game cribbage \
    --agents random,random \
    --games 10 \
    --seed 42 \
    --out games/
```

**Run 10 games of ShipWreck (2–4 players):**

```bash
# 2-player:
cargo run --release -p playtest-cli -- \
  play --game shipwreck --agents random,random \
       --games 10 --seed 42 --out games/

# 4-player:
cargo run --release -p playtest-cli -- \
  play --game shipwreck --agents random,random,random,random \
       --games 10 --seed 42 --out games/
```

You'll get `games/game-0000.jsonl` through `games/game-0009.jsonl`. Each file is one complete game: a header line, one line per event (deal, discard, cut, peg, show-score, end-game), and a final-result line.

**Tail a game to eyeball it:**

```bash
head -1 games/game-0000.jsonl | jq   # header
grep '"kind":"peg_scored"' games/game-0000.jsonl | head
tail -1 games/game-0000.jsonl | jq   # final scores
```

**Replay a recorded game and see state tick-by-tick:**

```bash
cargo run --release -p playtest-cli -- replay games/game-0000.jsonl
```

Add `--tick N` to dump just the state after the Nth event (useful for bisecting).

**Scale up:** `--games 1000 --parallel` fans games across rayon workers; per-file contents are seed-determined, so serial and parallel runs produce byte-identical output.

**Determinism check:** two runs of the same command (with `--fixed-time 0` to pin the header timestamp) produce byte-for-byte identical log files. Exercised in the CLI's smoke tests.

## Architecture

- **Ports and adapters** — every external-system interaction (clock, RNG, filesystem, game event sink, LLM) crosses a port trait with four adapter variants: `stub`, `production`, `record`, `playback`. Record captures live I/O to a tape; playback replays the tape for deterministic tests.
- **Game-agnostic engine** — the `Game` trait uses associated types (`State`, `Action`, `Event`, `PublicView`, `Config`). Specific games (starting with Cribbage) live in their own crates under `crates/games/`.
- **Deterministic** — seeded `ChaCha20Rng`, no direct `SystemTime::now` or `thread_rng` outside the production `Clock` / `Rng` adapters. Every game is fully replayable from its seed and event log.
- **Events, not effects, are serialized** — agents choose `Action`s; the engine turns them into `Event`s; snapshots at any tick are derived by folding events from the initial state. No separate snapshot serialization.

## Crates

| Crate | Purpose |
|-------|---------|
| `playtest-core` | `Game` trait, `Agent` trait, `GameLoop`, `GameResult`, `PlayerId` |
| `playtest-ports` | Port traits: `Clock`, `Rng`, `FileSystem`, `GameEventSink`, `LlmClient` |
| `playtest-adapters` | `stub`, `production`, `record`, `playback` adapters per port |
| `playtest-agents` | Agent implementations: `RandomAgent`, `ScriptedAgent` (trait is in `playtest-core`) |
| `playtest-log` | JSONL event log writer, streaming reader, and replay |
| `playtest-metrics` | SQLite schema, ingestion, `MetricRegistry`, reporter *(Phase 1)* |
| `playtest-cli` | `playtest` binary: `play`, `replay` (`report` arrives in Phase 1) |
| `crates/games/cribbage` | First game — 2-player standard Cribbage, 121 points |
| `crates/games/shipwreck` | Second game — 2–4 player rescue-points card game with event cards |
| `playtest-registry` | Game + agent lookup tables shared by the CLI and server |
| `playtest-server` | HTTP server for the web spine (Phase 2) |
