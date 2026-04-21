# playtester

A deterministic Rust CLI for playtesting card and board games with agent-driven self-play, replayable event logs, and a metrics spine for design-insight reports.

## Status

Early development. Phases 0 (engine foundations) and 1 (analytics spine) are in progress against Cribbage as the first game.

See [`playtest-roadmap.md`](playtest-roadmap.md) for the overall roadmap and [`docs/plans/`](docs/plans/) for active implementation plans.

## Build

```bash
cargo build --workspace
cargo test --workspace
```

## Architecture

- **Ports and adapters** — every external-system interaction (clock, RNG, filesystem, LLM) crosses a port trait with four adapter variants: `stub`, `production`, `record`, `playback`.
- **Game-agnostic engine** — the `Game` trait uses associated types (`State`, `Action`, `Event`, `PublicView`, `Config`). Specific games (starting with Cribbage) live in their own crates under `crates/games/`.
- **Deterministic** — seeded `ChaCha20Rng`, no direct system-time or thread-rng access outside ports. Every game is fully replayable from its seed and event log.

## Crates

| Crate | Purpose |
|-------|---------|
| `playtest-core` | `Game` trait, `GameLoop`, `GameResult`, `PlayerId` |
| `playtest-ports` | Port traits: `Clock`, `Rng`, `FileSystem`, `EventSink`, `LlmClient` |
| `playtest-adapters` | `stub`, `production`, `record`, `playback` adapters per port |
| `playtest-agents` | `Agent` trait, `RandomAgent`, `ScriptedAgent` |
| `playtest-log` | JSONL event log writer/reader + replay |
| `playtest-metrics` | SQLite schema, ingestion, `MetricRegistry`, reporter |
| `playtest-cli` | `playtest` binary: `play` / `replay` / `report` |
| `crates/games/cribbage` | First game — 2-player standard Cribbage |
