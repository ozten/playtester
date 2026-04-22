# Claude working notes — playtester

Project-level guidance for AI coding assistants working in this repo.

## Build profile: release only

**Disk is tight on this machine. Use `--release` for every cargo invocation.** Do not produce `target/debug/` artifacts.

- `cargo build --release`
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`
- `cargo check --release` for quick validation without a binary
- Soak tests (`#[ignore]`'d — `random_self_play`, `soak_10k`, `heuristic_beats_random`, `ismcts_beats_heuristic`) only make sense with `--release` anyway

If `target/` grows uncontrolled, run `cargo clean` (or `cargo clean --profile dev` to drop only debug artifacts). Prefer that over `rm -rf target/`.

## Branching: trunk-based, work on main

Do not create feature branches. Commit each implementation unit directly to `main`. Separate small concerns into separate commits (one commit per unit is the norm; docs updates can stack). This matches the user's preferred workflow.

## Architecture invariants

Detail lives in `docs/plans/` and the crate-level doc comments. The load-bearing rules:

1. **Ports and adapters (hexagonal).** Every external-system interaction — clock, RNG, filesystem, game event sink, LLM — crosses a port trait defined in `crates/playtest-ports/`. Each port has four adapter variants: `stub`, `production`, `record`, `playback`. Input ports (Clock, Rng, FileSystem, LlmClient) use all four variants. `GameEventSink` is the one output port — `record` aliases `production`, `playback` is a read-only no-op.
2. **Determinism.** Single seeded `ChaCha20Rng` per game; no `SystemTime::now()` or `thread_rng()` outside the production adapters. Every game replayable byte-for-byte from its seed and event log.
3. **Game-agnostic harness.** The `Game` trait in `playtest-core` uses associated types (`State`, `Action`, `Event`, `PublicView`, `Config`). Harness code never imports game crates. Games (Cribbage, ShipWreck) live under `crates/games/`.
4. **Events, not effects, are the serialized unit.** Agents choose `Action`s; engine produces `Event`s via `apply_action`; snapshots at tick N come from folding events 0..N into `initial_state(seed)`. No separate snapshot serialization.
5. **`playtest-server` holds no game-specific code.** Dispatch to games goes through `playtest-registry`, which both the CLI and the server depend on. The invariant is grep-enforced: `grep -rn 'cribbage\|shipwreck' crates/playtest-server/src/` returns zero matches.
6. **Agents return indices into the legal-actions slice**, not fabricated `Action`s — prevents illegal-move invention and keeps the stdio protocol (Phase 3) compatible.
7. **`Agent::choose` receives `view`, `legal`, and `state`.** The `state` arg was added in Unit 25 so planning agents (GreedyAgent, HeuristicAgent, ISMCTSAgent) can simulate one ply forward. Well-behaved agents that don't need planning ignore it; hidden-info discipline is enforced via `Game::determinize` for search-based agents.
8. **The determinize invariant:** `public_view(determinize(s, p, rng), p) == public_view(s, p)`. Property-tested for every game that implements `determinize`.

## Workspace layout

| Crate | Purpose |
|-------|---------|
| `playtest-core` | `Game` trait, `Agent` trait, `GameLoop`, `GameResult`, `PlayerId`, `Actor` |
| `playtest-ports` | Port traits: `Clock`, `Rng`, `FileSystem`, `GameEventSink`, `LlmClient` |
| `playtest-adapters` | Four-variant adapters per port (stub/production/record/playback) + `BroadcastGameEventSink` |
| `playtest-agents` | `RandomAgent`, `ScriptedAgent`, `GreedyAgent`, `HeuristicAgent`, `ISMCTSAgent` |
| `playtest-log` | JSONL writer, streaming reader, replay (schema v2 with `finished_at`) |
| `playtest-metrics` | SQLite ingestion, `MetricRegistry` trait, markdown reporter |
| `playtest-api` | HTTP + SSE wire types (zero workspace-internal deps); consumed by SvelteKit via `docs/openapi.json` |
| `playtest-server` | axum HTTP + SSE server; `playtest serve` subcommand delegates here |
| `playtest-registry` | Shared dispatch point for games and agents; used by both the CLI and the server |
| `playtest-cli` | `playtest` binary: `play`, `replay`, `report`, `serve`, `api-schema` subcommands |
| `crates/games/cribbage` | 2-player standard Cribbage (121 points, full pegging + show + crib + nibs + nobs) |
| `crates/games/shipwreck` | 2–4 player raft-building game; spec in `docs/shipwreck.md` |

## Plans and shipped work

Active plans live in `docs/plans/`:
- `2026-04-21-001-feat-playtester-phases-0-1-plan.md` — shipped (Units 1–15)
- `2026-04-21-002-feat-web-spine-shipwreck-phase-2-plan.md` — in progress (Units 16–25 shipped; 26 partial, 27 open)

Benchmarks and exit-criteria results live in `docs/BENCHMARKS.md`. Wire contract for the SvelteKit frontend lives in `docs/api-contract.md` and `docs/openapi.json`.

Documented learnings (bugs, best practices, architecture patterns) live in `docs/solutions/`, organized by category with YAML frontmatter (`module`, `tags`, `problem_type`). Relevant when implementing or debugging in documented areas.

## Testing policy

- **Unit + integration tests run in CI** (`cargo test --release --workspace`).
- **Soak tests and R2.x / R9.x benchmarks are `#[ignore]`'d.** Run with `cargo test --release -- --ignored` for the specific test. Always include them in the same commit as the code they validate.
- **No mocking of the database, filesystem, or RNG at the integration boundary.** Use the production adapters through port-trait dependency injection. The stub adapters are for cheap unit tests.

## Things to avoid

- Don't add `target/debug/` artifacts — release only.
- Don't propose feature branches — commit to `main`.
- Don't mock internal collaborators; inject via ports.
- Don't expand implementation units into per-step TDD choreography in plans.
- Don't use `--no-verify` or skip commit hooks unless explicitly asked.
- Don't introduce `SystemTime::now()` or `thread_rng()` outside production adapters; the determinism audit test will catch it.
