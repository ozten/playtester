---
title: "feat: Playtester engine foundations + analytics spine (Cribbage, Rust)"
type: feat
status: active
date: 2026-04-21
origin: playtest-roadmap.md
---

# feat: Playtester engine foundations + analytics spine (Cribbage, Rust)

## Overview

Build Phases 0 and 1 of the card-game playtesting CLI: a deterministic Rust engine that can play Cribbage end-to-end against itself with random/scripted agents, plus the analytics spine that turns batches of game logs into a human-readable markdown report.

The engine is built as a multi-crate Cargo workspace with **clean (hexagonal) architecture**: any interaction with an external system — clock, RNG, filesystem, LLM — crosses a **port** interface, with four adapter variants (`stub`, `production`, `record`, `playback`). The `record` and `playback` adapters give us deterministic, bit-for-bit reproducible end-to-end tests without fixtures drifting.

The engine is deliberately decoupled from any specific game. Cribbage ships first in its own crate (`playtest-cribbage`) and exercises the full harness. A later board game (raft cards, resources, event cards, constructed items) will plug into the same `Game` trait without engine changes.

## Problem Frame

The roadmap's guiding principle is that a deterministic Rust engine is non-negotiable infrastructure, and retrofitting determinism is painful. Phases 2–8 all compound on this foundation. We therefore have one chance to get three things right:

1. **Game abstraction** — rules, turns, end conditions, scoring live in the harness; specific effects live in the game crate. Wrong-sized here and the second game (the raft-and-resources board game) will require engine surgery.
2. **Port/adapter discipline** — the distinction between stub/production/record/playback is load-bearing for testability. Halfway-hexagonal always collapses into "just use the production adapter everywhere" and loses the whole benefit.
3. **Determinism** — single seeded RNG, no system-time leaks, fully replayable event log. Any stochastic source that bypasses the port is a latent Heisenbug.

Phase 1's metrics layer must be built on top of the same event log and must not couple itself to Cribbage-specific vocabulary. Metric definitions live with the game crate; the aggregation/storage/report machinery lives in the harness.

## Requirements Trace

Mapped from `playtest-roadmap.md` (origin) and the user's architectural constraints:

- **R0.1** Deterministic, seeded Rust engine plays a game end-to-end against itself with a `RandomAgent`
- **R0.2** `ScriptedAgent` with priority-list configuration
- **R0.3** Narrow `Agent` trait: given state + legal actions, return one action. Async-friendly so a blocking human/LLM adapter fits later without re-plumbing.
- **R0.4** Legal-move enumeration; engine refuses illegal actions
- **R0.5** Seeded RNG mediates all stochasticity through a `Rng` port
- **R0.6** Structured event log per turn; JSON snapshot of state at any tick
- **R0.7** `cargo test` suite covers rule correctness
- **R0.8** `playtest replay <log>` replays any past game tick-by-tick
- **R0.9** Can run **10,000 self-play games in under 60 seconds on one core**
- **R0.10** Every game produces a complete, replayable log
- **R0.11** **Zero panics over a 100K-game soak test**
- **R0.12** Cribbage 2-player standard: 6-card deal, 121 points, pegging + show + crib + nibs (his heels) + nobs, full scoring (15s/pairs/runs/flushes/jacks)
- **R0.13** Clean-architecture discipline: every external-system interaction goes through a port with four adapter variants (stub, production, record, playback)
- **R0.14** Game trait is abstract enough that a second, structurally different game (raft cards, resources, event cards, constructed items) plugs in without harness changes
- **R1.1** Ingest game logs into queryable storage (JSONL + SQLite)
- **R1.2** Universal per-game metrics: length, decision count per player, winner, end reason, wall-clock time
- **R1.3** Per-agent metrics: win rate, avg game length, avg final score
- **R1.4** Game-registered metrics (Cribbage: avg hand/crib/pegging score, show-vs-pegging-win ratio, lead changes, final-score margin)
- **R1.5** Per-card design-insight metrics (Cribbage-reframed equivalents of the roadmap's CCG-shaped "per-card" metrics): `card_kept_rate[rank]`, `card_discarded_to_own_crib_rate[rank]`, `card_discarded_to_opp_crib_rate[rank]`, `win_rate_when_card_in_hand[rank]`, `win_rate_when_card_in_crib[rank]`
- **R1.6** `playtest report --games 10000 --config <...>` produces a readable markdown report in **under 30 seconds**
- **R1.7** Every later phase's data flows through this layer unchanged, and adding a new game (with or without deck construction) requires no harness changes

## Scope Boundaries

- **No LLM integration in this plan.** The `LlmClient` port *is* defined so record/playback infrastructure covers it symmetrically, but no production adapter is wired. That is Phase 3.
- **No ISMCTS, no heuristic evaluation.** `ScriptedAgent` is the ceiling for this plan. That is Phase 2.
- **No TerminalAgent / human player.** The `Agent` trait is async-friendly so a blocking stdin adapter drops in later. That is Phase 3.
- **No multi-player Cribbage.** 2-player only. 3/4-player variants require crib-building rule changes and are out of scope.
- **No match play.** Single game ends at 121 points; no skunks-in-matches tracking. (The Phase 1 metrics framework will not preclude adding it later.)
- **No archetype clustering / HDBSCAN / Shannon entropy.** Those are deck-construction metrics that require a deck-building mechanic to generate useful co-occurrence data. A fixed 52-card deck makes them degenerate. The `MetricRegistry` abstraction (Unit 12) is designed so the future deckbuilding game can register its own archetype metric without harness changes — we carry the concept forward structurally but do not build it now.
- **No roadmap "per-deck" metrics (per-deck win rate, avg turn-of-concede).** Cribbage has no deck construction and no concession mechanic; these degenerate to constants. Defer until a deckbuilding game lands.
- **No "mulligan keep rate".** Cribbage has no mulligan. The analogous design signal — which cards are preferentially kept vs discarded to the crib — is captured by `card_kept_rate` / `card_discarded_to_crib_rate` (R1.5) instead.
- **No DuckDB/Parquet.** JSONL + SQLite instead (per user decision). Migration to DuckDB later is plausible; the ingestion boundary isolates the storage choice.
- **No web UI.** CLI-only. That is Phase 8.

### Deferred to Separate Tasks

- **ISMCTS + heuristic agents**: Phase 2, next plan
- **LLM agent + stdio protocol + scratch buffer**: Phase 3
- **Personas**: Phase 4
- **Post-game critique**: Phase 5
- **Compare/counterfactual subcommand**: Phase 6
- **Archetype/entropy metrics**: re-introduce when the second game (deck-building board game) is scoped

## Context & Research

### Relevant Code and Patterns

The repository is greenfield: only `playtest-roadmap.md` exists today. No existing Rust code, no workspace, no prior conventions. All architectural decisions in this plan are load-bearing for everything that follows.

### Institutional Learnings

None yet (`docs/solutions/` does not exist). This plan will likely be the first source of institutional knowledge for the project; its key technical decisions below are candidates for later `ce-compound` writeups once they survive contact with implementation.

### External References

- Cowling/Powley/Whitehouse 2012 — **ISMCTS** (referenced for Phase 2, not built here but the `Agent` trait must not preclude tree-reuse across turns)
- CommunicationMod pattern from Slay the Spire — **stdio protocol** (referenced for Phase 3; the event-log schema designed here should generalize to it)
- **Cribbage rules** — standard 2-player / 6-card / 121-point / full show + crib + nibs (heels, 2 pts to dealer on jack cut) + nobs (1 pt to holder of jack of cut suit during show). See e.g. Hoyle / ACC rules for scoring edge cases (flush-in-crib requires 5-card match, runs of 3+ in any order, etc.)
- **Rust determinism** — use `rand_chacha::ChaCha20Rng` (portable, not platform-dependent like `rand::thread_rng`). Never use `SystemTime::now()` outside the `Clock` port.
- **`serde` + `serde_json`** for JSONL event log. **`rusqlite`** (bundled) for SQLite ingestion.

## Key Technical Decisions

- **Multi-crate Cargo workspace.** Engine/ports/adapters/agents/metrics/CLI each live in their own crate under `crates/`. Games live under `crates/games/`. Rationale: forces the decoupling; crate-level visibility is the only honest way to enforce "harness doesn't know about Cribbage."
- **`Game` trait over associated types, not dyn-typed state.** Each game defines its own `State`, `Action`, `Event`, and `PublicView` types. Rationale: type safety, zero-cost, and keeps the harness ignorant of game details. Trade-off: can't hold heterogeneous games in one collection, but we don't need to.
- **Events, not effects, are the serialized unit.** An `Action` is what an agent chose; the engine turns it into a sequence of `Event`s, which are the atomic things written to the log and used to reconstruct state. Rationale: records the *observable* game history without the "effect DSL" coupling that's CCG-flavored.
- **Snapshot = replay.** State snapshot at tick `N` is produced by applying events `0..N` to `initial_state(seed)`. No separate snapshot serialization to keep in sync. Rationale: single source of truth. Trade-off: replay cost grows with game length — acceptable for games <500 events.
- **All stochasticity flows through the `Rng` port.** `ChaCha20Rng` in production. Rationale: determinism is the central architectural invariant; any direct `rand::random()` call is a bug.
- **Four-variant adapter discipline, from day one.** Every port has `stub`, `production`, `record`, `playback` adapters. `record` wraps another adapter (usually `production`) and tees every input/output pair to disk; `playback` reads that tee and returns the stored values. Rationale: makes end-to-end tests cheap and deterministic; catches non-determinism the moment it appears.
- **Async `Agent` trait via `async_trait`.** Even though `RandomAgent` is sync and Phase 0–1 agents never suspend, the trait returns `impl Future`. Rationale: a human TerminalAgent (Phase 3) needs blocking I/O; an LLMAgent needs network I/O; retrofitting sync→async later is painful. Sync agents just return ready futures — zero runtime overhead.
- **Single-threaded game loop; parallelism at the batch level.** One game runs on one thread. `playtest play --games N` fans games out across cores via `rayon`. Rationale: determinism-per-game is easy; determinism across parallel games requires only per-game seeds.
- **JSONL event log, one file per game.** Filename: `<timestamp>-<seed>-<game_id>.jsonl`. First line is a header (game type, version, seed, config hash). Subsequent lines are events. Rationale: human-readable, grep-able, diffable, trivially streamable, no schema-migration headaches in Phase 0.
- **SQLite for aggregates (Phase 1), not for raw events.** Raw events stay in JSONL; an ingestion pass fills SQLite tables (`games`, `agent_stats`, `game_metrics`). Rationale: JSONL is the source of truth; SQLite is a query cache. Re-ingestion is always safe.
- **Game-registered metric definitions.** Each game crate exposes a `MetricRegistry` listing the metrics it emits. The harness knows how to persist any metric; games define which metrics exist. Rationale: keeps Cribbage-specific names (e.g., "pegging_score") out of the harness; second game registers its own set.

## Open Questions

### Resolved During Planning

- **Storage stack**: JSONL + SQLite (per user answer; DuckDB deferred).
- **Cribbage scope**: 2-player standard, full rules, single game (not match).
- **Human player architecture**: async-friendly `Agent` trait, no TerminalAgent in these phases.
- **Parallelism**: rayon over game batches; single-threaded per-game.
- **RNG crate**: `rand_chacha::ChaCha20Rng` (portable, deterministic across platforms).
- **Serialization**: `serde` + `serde_json` for JSONL; `rusqlite` (bundled feature) for SQLite.

### Deferred to Implementation

- **Exact `Event` taxonomy for Cribbage** — will emerge during Unit 6–9 construction. Target surface: `DealCard`, `DiscardToCrib`, `CutStarter`, `PegPlay`, `PegScore{reason, points}`, `ShowScore{reason, points}`, `PassGo`, `EndGame{winner, reason}`. Finalize when hand-counting tests make the right seams obvious.
- **Snapshot caching strategy** — if `cargo bench` shows replay cost hurts the 100K soak test, add periodic snapshot checkpoints. Don't pre-optimize.
- **Public-view shape** — `PublicView<G>` is defined as an associated type but its concrete content (what info each player sees) will be shaped by the needs of a real `ScriptedAgent`, not by speculation.
- **Rayon vs. per-process parallelism** — if 10K games in <60s needs multiple processes (unlikely with rayon), defer. Measure first.

## Output Structure

    playtester/
    ├── Cargo.toml                        # workspace root
    ├── rust-toolchain.toml               # pin stable channel
    ├── .github/workflows/ci.yml          # fmt + clippy -D warnings + test
    ├── docs/
    │   └── plans/
    │       └── 2026-04-21-001-...-plan.md
    ├── playtest-roadmap.md               # existing
    └── crates/
        ├── playtest-core/                # Game trait, GameLoop, GameResult, PlayerId
        ├── playtest-ports/               # Clock, Rng, FileSystem, EventSink, LlmClient traits
        ├── playtest-adapters/            # stub/production/record/playback impls per port
        ├── playtest-agents/              # Agent trait + RandomAgent + ScriptedAgent
        ├── playtest-log/                 # EventLog writer/reader, JSONL format, replay
        ├── playtest-metrics/             # SQLite schema, ingestion, MetricRegistry, reporter
        ├── playtest-cli/                 # binary: play | replay | report subcommands
        └── games/
            └── cribbage/                 # playtest-cribbage: State/Action/Event + rules + metrics

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

**Game loop (harness, game-agnostic):**

```text
loop {
    if let Some(result) = Game::game_over(&state) { break result; }
    match Game::next_to_act(&state) {
        Actor::Chance => {
            // engine rolls through Rng port, produces an Event
            let event = Game::resolve_chance(&state, &mut rng_port)?;
            events.push(event.clone());
            state = Game::apply_event(state, event);
        }
        Actor::Player(p) => {
            let view = Game::public_view(&state, p);
            let legal = Game::legal_actions(&state, p);
            let choice = agents[p].choose(view, legal).await?;
            let new_events = Game::apply_action(&state, p, choice)?;
            for e in new_events { events.push(e.clone()); state = Game::apply_event(state, e); }
        }
    }
}
```

**Port/adapter layering:**

```text
   engine / game  --->  trait Port  <---  Stub | Production | Record(wraps P) | Playback
                              ^
                              |
                     injected at GameLoop construction
```

`Record<Rng>` wraps `Production<Rng>` and appends every `(call_id, output)` to a sidecar file. `Playback<Rng>` reads that sidecar and returns the stored outputs in order, panicking if the call pattern diverges — which *is* the test signal for non-determinism.

**Event log shape (JSONL, one file per game):**

```text
{"kind":"header","schema":1,"game":"cribbage","version":"0.1.0","seed":12345,"agents":["random","scripted:v1"]}
{"tick":0,"kind":"event","payload":{"DealCard":{"player":0,"card":"AH"}}}
{"tick":1,"kind":"event","payload":{"DealCard":{"player":1,"card":"7C"}}}
...
{"tick":N,"kind":"event","payload":{"EndGame":{"winner":0,"reason":"score121"}}}
```

**Phase 1 dataflow:**

```text
JSONL/*.jsonl  --[ingest]-->  games table (one row/game)
                              agent_stats table (one row/agent/game)
                              game_metrics table (long-format: game_id, metric_name, value)
         ^
         |
         MetricRegistry (game-provided: which metrics exist + how to extract from event stream)
         |
         v
SQL queries  --[format]-->  markdown report
```

## Implementation Units

### Phase 0 — Engine foundations

- [ ] **Unit 1: Workspace scaffolding and CI baseline**

**Goal:** Cargo workspace exists with all crates registered, compiles, and CI runs fmt + clippy + test.

**Requirements:** R0.13 (enforces the clean-architecture crate boundaries)

**Dependencies:** none

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `.github/workflows/ci.yml`
- Create: `crates/playtest-core/Cargo.toml`, `crates/playtest-core/src/lib.rs`
- Create: `crates/playtest-ports/Cargo.toml`, `crates/playtest-ports/src/lib.rs`
- Create: `crates/playtest-adapters/Cargo.toml`, `crates/playtest-adapters/src/lib.rs`
- Create: `crates/playtest-agents/Cargo.toml`, `crates/playtest-agents/src/lib.rs`
- Create: `crates/playtest-log/Cargo.toml`, `crates/playtest-log/src/lib.rs`
- Create: `crates/playtest-metrics/Cargo.toml`, `crates/playtest-metrics/src/lib.rs`
- Create: `crates/playtest-cli/Cargo.toml`, `crates/playtest-cli/src/main.rs`
- Create: `crates/games/cribbage/Cargo.toml`, `crates/games/cribbage/src/lib.rs`
- Create: `.gitignore`, `README.md` (minimal)

**Approach:**
- Pin stable Rust toolchain in `rust-toolchain.toml`
- Workspace `resolver = "2"`, shared `[workspace.dependencies]` for `serde`, `serde_json`, `thiserror`, `anyhow`, `rand`, `rand_chacha`, `async-trait`, `tokio` (rt+macros only)
- Each crate has an empty `lib.rs` stub that compiles
- CLI binary crate stubs `main()` with a `clap` parser skeleton
- CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

**Test scenarios:**
- Happy path: `cargo build --workspace` succeeds from clean checkout
- Happy path: `cargo test --workspace` runs (no tests yet, but harness executes)
- Happy path: `cargo clippy --workspace -- -D warnings` is clean
- Test expectation: none for crate contents in this unit — scaffolding only

**Verification:**
- All crates listed in the workspace build with no warnings
- CI workflow passes on a push
- `cargo tree -p playtest-core` shows no transitive dependency on any game crate

---

- [ ] **Unit 2: Port trait definitions**

**Goal:** Define the five port traits that carry every external-system interaction.

**Requirements:** R0.5, R0.13

**Dependencies:** Unit 1

**Files:**
- Create: `crates/playtest-ports/src/clock.rs` — `trait Clock { fn now(&mut self) -> UnixMillis; }`
- Create: `crates/playtest-ports/src/rng.rs` — `trait Rng { fn next_u64(&mut self) -> u64; fn gen_range(&mut self, range: Range<u64>) -> u64; fn shuffle<T>(&mut self, slice: &mut [T]); }`
- Create: `crates/playtest-ports/src/filesystem.rs` — `trait FileSystem { fn read(&self, path: &Path) -> Result<Vec<u8>>; fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<()>; fn append_line(&mut self, path: &Path, line: &str) -> Result<()>; }`
- Create: `crates/playtest-ports/src/event_sink.rs` — `trait EventSink { fn emit(&mut self, record: &EventRecord) -> Result<()>; }`
- Create: `crates/playtest-ports/src/llm_client.rs` — `trait LlmClient { async fn complete(&self, req: LlmRequest) -> Result<LlmResponse>; }` *(defined for symmetry; unused in Phase 0–1)*
- Create: `crates/playtest-ports/src/lib.rs` — re-exports
- Test: `crates/playtest-ports/tests/traits_object_safe.rs`

**Approach:**
- All traits object-safe (except `LlmClient` which uses `async_trait` — acceptable, it's behind a feature if needed)
- Error types per port via `thiserror`
- No default impls; each adapter must implement the trait fully

**Patterns to follow:** Hexagonal architecture — ports live in their own crate, know nothing about adapters.

**Test scenarios:**
- Happy path: each port trait is object-safe where applicable (compile-time check via `fn _assert(_: &dyn Clock) {}` etc.)
- Edge case: `Rng::gen_range` with an empty range returns an error, not a panic
- Edge case: `FileSystem::append_line` to a non-existent path creates the file

**Verification:**
- `crates/playtest-ports` has zero dependencies on any other workspace crate
- All trait methods have doc comments stating which adapter variants must honor which invariants

---

- [ ] **Unit 3: Adapter quartet (stub, production, record, playback)**

**Goal:** Every port has four adapter implementations. The record/playback pair is the critical piece — it is what gives us deterministic end-to-end tests.

**Requirements:** R0.5, R0.10, R0.13

**Dependencies:** Unit 2

**Files:**
- Create: `crates/playtest-adapters/src/clock/` — `stub.rs` (fixed time), `production.rs` (`SystemTime::now`), `record.rs`, `playback.rs`
- Create: `crates/playtest-adapters/src/rng/` — `stub.rs` (returns sequence), `production.rs` (`ChaCha20Rng::seed_from_u64`), `record.rs`, `playback.rs`
- Create: `crates/playtest-adapters/src/filesystem/` — `stub.rs` (in-memory HashMap), `production.rs` (std::fs), `record.rs`, `playback.rs`
- Create: `crates/playtest-adapters/src/event_sink/` — `stub.rs` (Vec), `production.rs` (appends JSONL), `record.rs` (same as production in this case), `playback.rs` (N/A for sink; provide no-op that asserts on use)
- Create: `crates/playtest-adapters/src/llm_client/` — `stub.rs` (returns canned response), `production.rs` (stub returning error with "Phase 3" message), `record.rs`, `playback.rs`
- Create: `crates/playtest-adapters/src/recording.rs` — shared `RecordingTape` helper (append-only file of `(call_id, args_hash, output)` entries)
- Test: `crates/playtest-adapters/tests/record_playback_roundtrip.rs`

**Approach:**
- `Record<P: Port>` is a generic wrapper: constructs with an inner adapter plus a tape path; every call delegates to inner, then appends an entry to the tape.
- `Playback<P: Port>` reads a tape at construction; every call pops the next entry, verifies the call matches, returns the stored output. Mismatches are a panic — that's the whole value proposition.
- Tape format is JSONL with a schema version header, same conventions as event log.
- `StubRng` is deterministic (seeded internal ChaCha20) so even stub tests reproduce.

**Patterns to follow:** Decorator pattern; shared recording helper keeps the record/playback logic DRY across ports.

**Test scenarios:**
- Happy path (Rng): record a sequence of `gen_range` calls against `Production<Rng>`, then playback returns identical outputs bit-for-bit
- Happy path (FileSystem): record reads/writes, playback reproduces
- Edge case: playback called with one more `next_u64()` than the tape contains → panic with a clear "tape exhausted at call N" message
- Error path: playback tape from game version `0.1.0` loaded against schema version `0.2.0` → explicit error, not silent divergence
- Integration: a tiny fake-game loop run under `Record<Rng>` and `Record<Clock>` produces tapes; same loop under `Playback<Rng>` and `Playback<Clock>` produces identical events

**Verification:**
- A single round-trip integration test proves record→playback fidelity for at least one Rng call sequence + one FileSystem sequence
- Tape files are human-readable JSONL (eyeball one by hand)

---

- [ ] **Unit 4: Core `Game` trait and game loop**

**Goal:** The game-agnostic harness: `Game` trait, `GameLoop`, `GameResult`, `PlayerId`.

**Requirements:** R0.1, R0.4, R0.14

**Dependencies:** Units 2, 3

**Files:**
- Create: `crates/playtest-core/src/game.rs` — `trait Game { type State; type Action; type Event; type PublicView; type Config; ... }`
- Create: `crates/playtest-core/src/actor.rs` — `enum Actor { Chance, Player(PlayerId) }`
- Create: `crates/playtest-core/src/result.rs` — `struct GameResult { winner: Option<PlayerId>, reason: EndReason, scores: Vec<i32> }`
- Create: `crates/playtest-core/src/game_loop.rs` — orchestrator that takes a `Game` impl, agents, ports
- Create: `crates/playtest-core/src/error.rs` — `enum GameError { IllegalAction, AgentError, ... }`
- Test: `crates/playtest-core/tests/game_loop_shape.rs` — uses a minimal test `Game` impl (rock-paper-scissors) to prove the loop works

**Approach:**
- `Game` trait surface (sketch — finalize during impl):
  - `fn initial_state(&self, seed: u64, cfg: &Self::Config) -> Self::State`
  - `fn next_actor(&self, state: &Self::State) -> Actor`
  - `fn legal_actions(&self, state: &Self::State, player: PlayerId) -> Vec<Self::Action>`
  - `fn apply_action(&self, state: &Self::State, player: PlayerId, action: &Self::Action) -> Result<Vec<Self::Event>>`
  - `fn resolve_chance(&self, state: &Self::State, rng: &mut dyn Rng) -> Result<Self::Event>`
  - `fn apply_event(&self, state: Self::State, event: &Self::Event) -> Self::State`
  - `fn public_view(&self, state: &Self::State, player: PlayerId) -> Self::PublicView`
  - `fn game_over(&self, state: &Self::State) -> Option<GameResult>`
- `GameLoop` owns the state and pumps the loop; it does not own the event log (that is injected via `EventSink` port)

**Patterns to follow:** Pure functions where possible; `apply_event` is deliberately non-failing (events are already validated by `apply_action`).

**Test scenarios:**
- Happy path: a trivial 2-player rock-paper-scissors `Game` impl plays to completion through `GameLoop`
- Happy path: `GameLoop` rejects an illegal action returned by an agent (engine is authoritative)
- Edge case: game that terminates on the very first state (before any action) short-circuits cleanly
- Edge case: `next_actor` returns `Chance` → engine consumes `Rng` port, not the agent
- Error path: agent panics or returns an error → `GameLoop` surfaces it with game context attached

**Verification:**
- `crates/playtest-core` has no dependency on `crates/games/cribbage` (enforced by `cargo tree`)
- The test `Game` impl fits in under 100 lines — confirms the trait is not over-parameterized

---

- [ ] **Unit 5: `Agent` trait + `RandomAgent` + `ScriptedAgent`**

**Goal:** The narrow agent interface, plus two baseline agents.

**Requirements:** R0.1, R0.2, R0.3

**Dependencies:** Unit 4

**Files:**
- Create: `crates/playtest-agents/src/agent.rs` — `#[async_trait] trait Agent<G: Game> { async fn choose(&mut self, view: &G::PublicView, legal: &[G::Action]) -> Result<usize>; }`
- Create: `crates/playtest-agents/src/random.rs` — `RandomAgent<R: Rng>` picks uniformly over legal actions
- Create: `crates/playtest-agents/src/scripted.rs` — `ScriptedAgent` takes a priority function `fn(&PublicView, &Action) -> i32` and picks the highest-scoring legal action
- Test: `crates/playtest-agents/tests/random_agent.rs`
- Test: `crates/playtest-agents/tests/scripted_agent.rs`

**Approach:**
- `Agent::choose` returns a `usize` **index into the legal-actions slice**, not an `Action`. This matches the roadmap's Phase 3 stdio protocol (`action index 0..N`), and prevents an agent from fabricating an illegal action.
- `RandomAgent` holds its own `Rng` adapter — separate seed from the engine's game RNG so agent stochasticity is independent and replayable.
- `ScriptedAgent` priority is just a function; games can export their own factory functions (e.g., `playtest_cribbage::scripted::pair_up_preference`).
- `async_trait` used throughout even though Phase 0 agents are sync — future TerminalAgent/LLMAgent need it.

**Patterns to follow:** Generic over `G: Game`; no Cribbage-specific code in this crate.

**Test scenarios:**
- Happy path (Random): given legal actions of length N, returns an index in `0..N`; distribution over 10K draws is roughly uniform (chi-square sanity check)
- Happy path (Random): two `RandomAgent`s seeded identically produce identical choices for identical inputs
- Edge case: legal actions slice is empty → error (engine bug, but agent must not panic)
- Happy path (Scripted): priority function that always prefers the first legal action returns index 0
- Happy path (Scripted): priority function with ties breaks deterministically (lowest index wins)
- Integration: `GameLoop` driving a `RandomAgent` vs `RandomAgent` on the test rock-paper-scissors game completes and produces a valid `GameResult`

**Verification:**
- `crates/playtest-agents` has zero dependency on `crates/games/cribbage`
- `RandomAgent` + `Playback<Rng>` produces deterministic choice sequences

---

- [ ] **Unit 6: Event log + replay infrastructure**

**Goal:** JSONL event log writer, reader, and replay function. Snapshots are derived, not stored separately.

**Requirements:** R0.6, R0.8, R0.10

**Dependencies:** Units 2, 3, 4

**Files:**
- Create: `crates/playtest-log/src/header.rs` — `LogHeader { schema: u32, game: String, version: String, seed: u64, agents: Vec<String>, started_at: UnixMillis, config_hash: String }`
- Create: `crates/playtest-log/src/record.rs` — `enum LogRecord<E> { Header(LogHeader), Event { tick: u64, payload: E }, Final(GameResult) }`
- Create: `crates/playtest-log/src/writer.rs` — `EventLogWriter` (implements the `EventSink` port, serializes to JSONL)
- Create: `crates/playtest-log/src/reader.rs` — streaming reader that yields `LogRecord<E>` values
- Create: `crates/playtest-log/src/replay.rs` — `fn replay<G: Game>(game: &G, log_path: &Path) -> Result<Vec<G::State>>` — reconstructs every tick's state
- Test: `crates/playtest-log/tests/roundtrip.rs`

**Approach:**
- JSONL: one header line + N event lines + one final line
- Generic over `G::Event: Serialize + DeserializeOwned` — log crate knows nothing about Cribbage
- `config_hash` is a SHA-256 of the serialized `G::Config` to detect replay against a changed config
- Replay works in two modes: full (return every tick's state) and final-only (just confirm log is valid)

**Patterns to follow:** Streaming / iterator-based reader — must handle multi-MB logs without loading everything at once.

**Test scenarios:**
- Happy path: write 100 events, read them back, count matches
- Happy path: replay rock-paper-scissors log reproduces the final state that the original loop ended in
- Edge case: log with just header + final record (0 events) is valid
- Edge case: malformed JSON on line 7 → error points at line 7, not silent truncation
- Error path: log version mismatch (`schema: 1` when code expects `2`) → explicit error
- Error path: `config_hash` mismatch during replay → explicit error ("replay against wrong config")
- Integration: `GameLoop` with `EventLogWriter` as its sink produces a log; `replay()` on that log reproduces the same tick-by-tick states

**Verification:**
- A log file is eyeball-readable and grep-friendly (`grep DealCard game.jsonl` works)
- 10K-event replay completes in well under a second

---

- [ ] **Unit 7: Cribbage primitives (cards, deck, board, hand)**

**Goal:** The game-specific building blocks for Cribbage, with exhaustive tests.

**Requirements:** R0.12

**Dependencies:** Units 2, 4

**Files:**
- Create: `crates/games/cribbage/src/card.rs` — `Card { rank: Rank, suit: Suit }`, `Rank` (Ace=1..King=13 with value cap at 10 for pegging), `Suit`
- Create: `crates/games/cribbage/src/deck.rs` — `Deck::fresh() -> [Card; 52]`, `Deck::shuffle(rng: &mut dyn Rng)`
- Create: `crates/games/cribbage/src/board.rs` — `Board { pins: [PlayerPins; 2] }` — tracks front pin + back pin per player, advance-by-N, detect 121
- Create: `crates/games/cribbage/src/hand.rs` — `Hand(Vec<Card>)`, helpers (sort, contains, remove)
- Test: `crates/games/cribbage/tests/primitives.rs`

**Approach:**
- `Card::value()` returns pegging value (1..10 with faces = 10); `Card::rank_ord()` returns run-ordering (A=1..K=13) so you don't accidentally conflate them. This is a classic Cribbage bug source.
- `Board` uses two pins per player (standard Cribbage; front pin shows current score, back pin shows previous). `Board::advance(player, points)` moves the back pin to the front, then moves the front pin forward.
- `Deck::shuffle` delegates to the `Rng` port.
- No references to `playtest-core` yet — these are standalone types.

**Patterns to follow:** Small, pure, exhaustively tested primitives — everything else in Cribbage depends on these.

**Test scenarios:**
- Happy path: fresh deck has exactly 52 unique cards (13 × 4)
- Happy path: shuffle with same seed produces same order
- Happy path: `Card::value()` returns 1 for Ace, 10 for J/Q/K, 2..10 for 2..10
- Edge case: `Card::value()` for J/Q/K is 10 but `rank_ord()` is 11/12/13 (explicit test that these do not collide)
- Happy path: `Board::advance(0, 5)` moves player 0's pins correctly
- Happy path: `Board::advance(0, 121)` reports game-won
- Edge case: `Board::advance` past 121 → clamp or report win (standard rule: must peg exactly to win or overshoot ends the game — test both interpretations, pick one, document it)
- Edge case: `Hand::remove(card_not_in_hand)` returns `Err`, does not silently no-op

**Verification:**
- `cargo test -p playtest-cribbage --test primitives` all green
- The module exposes no `pub` mutable globals, no `thread_rng`, no `SystemTime::now`

---

- [ ] **Unit 8: Cribbage game logic — deal, discard, pegging**

**Goal:** First half of the Cribbage game flow: deal 6 cards to each player, discard 2 to crib, cut the starter, play the pegging phase with full peg scoring.

**Requirements:** R0.12

**Dependencies:** Unit 7

**Files:**
- Create: `crates/games/cribbage/src/state.rs` — `GameState { phase, dealer, non_dealer, hands, crib, played, starter, pegging_stack, running_total, last_to_play, board }`
- Create: `crates/games/cribbage/src/phase.rs` — `enum Phase { Deal, Discard, Cut, Pegging, Show, ScoreCrib, Finished }`
- Create: `crates/games/cribbage/src/action.rs` — `enum Action { DiscardToCrib(Card, Card), PlayCard(Card), SayGo }`
- Create: `crates/games/cribbage/src/event.rs` — `enum Event { DealCard{player,card}, DiscardToCrib{player,cards}, CutStarter{card}, NibsScored{points}, PegPlayed{player,card,running_total}, PegScored{player,reason,points}, PeggingRoundEnd, ... }`
- Create: `crates/games/cribbage/src/pegging.rs` — pure scoring function `fn score_peg_play(stack: &[Card], running_total: u8) -> Vec<PegReason>`
- Test: `crates/games/cribbage/tests/pegging_scoring.rs`
- Test: `crates/games/cribbage/tests/discard_flow.rs`

**Approach:**
- Scoring during pegging: 15 (2 pts), 31 (2 pts), pair/triple/quadruple (2/6/12), run (3+, must be the last N cards in the played stack in any order), last card (1 pt — except at exactly 31 which already gives 2 for the 31)
- Nibs (his heels): if the cut starter is a Jack, dealer gets 2 points immediately. This can win the game on the cut.
- "Say Go" semantics: if a player cannot play without exceeding 31, they say "Go"; the other player plays as many as they can; last-card/31 is scored; reset stack to 0, continue until both hands are empty
- Game might end mid-pegging (pin hits 121) — check after every `PegScored` event

**Patterns to follow:** Scoring functions are pure — take a stack, return a list of reasons + points. No mutation; the event-emission layer wraps them.

**Test scenarios:**
- Happy path: pair-pegging ("opponent plays 7, I play 7") scores 2
- Happy path: triple-pegging (7-7-7) scores 6
- Happy path: run-pegging (3-5-4 on the stack) scores 3 — order in stack need not be sorted
- Happy path: hitting 15 scores 2
- Happy path: hitting 31 scores 2 (and round ends, stack resets)
- Happy path: last card under 31 scores 1
- Edge case: nibs — jack on the cut awards dealer 2 points pre-pegging
- Edge case: game-winning peg — pin at 119, scores 2, game ends mid-pegging with an `EndGame` event, no subsequent events emitted
- Edge case: double-pair-royal on the stack (four of a kind) scores 12
- Edge case: run must be contiguous cards at the end of the stack — "4, 9, 5, 6" is not a run; "9, 5, 6, 4" ending with 4 is a run of 3
- Error path: `apply_action(PlayCard(C))` where C is not in the player's hand → `GameError::IllegalAction`
- Error path: `apply_action(PlayCard(C))` where C would push total over 31 → illegal
- Error path: `apply_action(SayGo)` when the player *could* legally play → illegal (anti-cheat)
- Integration: full deal→discard→cut→pegging flow with two `RandomAgent`s reaches a terminal pegging state in every run of 1000 games

**Verification:**
- Pegging scorer passes a reference set of at least 30 hand-scored scenarios
- No test uses `thread_rng`; all determinism flows through the `Rng` port

---

- [ ] **Unit 9: Cribbage game logic — the show, crib counting, and game termination**

**Goal:** Second half of the Cribbage game flow: show phase (non-dealer counts hand, dealer counts hand, dealer counts crib), full combinatorial scoring, and the winner check.

**Requirements:** R0.12

**Dependencies:** Unit 8

**Files:**
- Create: `crates/games/cribbage/src/scoring/show.rs` — pure function `fn score_hand(hand: [Card; 4], starter: Card, is_crib: bool) -> ShowScore` returning `{ fifteens, pairs, runs, flush, nobs, total }`
- Create: `crates/games/cribbage/src/scoring/fifteens.rs`, `pairs.rs`, `runs.rs`, `flush.rs`, `nobs.rs` — one module per rule, each pure and testable
- Modify: `crates/games/cribbage/src/state.rs`, `phase.rs`, `event.rs` — wire the show phase
- Create: `crates/games/cribbage/src/rules.rs` — top-level `CribbageGame` implementing `playtest_core::Game`
- Test: `crates/games/cribbage/tests/show_scoring.rs` — minimum 40 hand-scored scenarios including classic edge cases
- Test: `crates/games/cribbage/tests/full_game.rs` — end-to-end Random-vs-Random, 100 runs

**Approach:**
- Fifteens: enumerate all 2–5 card subsets summing to 15; each counts 2 points (note: this is combinatoric, not set-based — 5+5+5 with a 5-of-hearts flush gives multiple fifteens)
- Pairs: all C(n,2) pairs among same-rank cards; 2 pts each (so three-of-a-kind = 3 pairs = 6 pts)
- Runs: longest run of 3+ consecutive ranks, times multiplicity (pair in a run doubles it; triple triples it)
- Flush: 4+ same-suit in hand only (not counting starter unless all 5 match); 4 or 5 pts. **Crib flush requires all 5 match** (stricter rule — classic bug source)
- Nobs: jack in hand (not crib jacks specifically — any jack) matching starter's suit = 1 pt
- Counting order at showdown: non-dealer hand first, then dealer hand, then dealer crib — **order matters** because the game can end mid-show
- `CribbageGame::game_over` returns `Some(GameResult)` as soon as either pin hits 121

**Patterns to follow:** One rule per file, each with its own exhaustive test module. Top-level `score_hand` composes them.

**Test scenarios:**
- Happy path: classic "29-hand" (five-of-a-kind setup) scores 29 — the maximum possible
- Happy path: empty-ish hand (e.g., 2♠ 4♥ 6♦ K♣ + starter Q♥) scores 0
- Happy path: six-card straight with a pair (4-5-6-7-7 with starter 8) scores double-run + pair
- Edge case: flush of 4 in hand, starter different suit → 4 pts (regular hand) or 0 pts (crib)
- Edge case: flush of 5 (all 4 hand cards + starter same suit) → 5 pts in both hand and crib
- Edge case: nobs — hand contains J♥ and starter is ♥ → 1 pt; hand contains J♥ but starter is ♣ → 0
- Edge case: nibs scored at cut (unit 8) already added 2 to dealer — ensure show doesn't double-count
- Edge case: double-double-run (1-2-3 with two 2s and two 3s) — 4 runs × 3 + 2 pairs × 2 = 16 points
- Happy path: showdown order — game ends when non-dealer pegs to 121 during show; dealer never gets to count their hand (classic rule)
- Integration: 100 full Random-vs-Random games all terminate, all produce a winner with score ≥ 121, none panic
- Integration: log every Random-vs-Random game → replay the log → final state matches

**Verification:**
- `cargo test -p playtest-cribbage` green, 40+ scoring test cases
- Hand-scored "29 hand" test exists and passes
- Random-vs-Random produces a winner in < 0.5s per game

---

- [ ] **Unit 10: CLI `play` and `replay` subcommands**

**Goal:** The user-facing binary that drives everything shipped in Phase 0.

**Requirements:** R0.1, R0.8, R0.9, R0.10

**Dependencies:** Units 5, 6, 9

**Files:**
- Create: `crates/playtest-cli/src/commands/play.rs` — `playtest play --game cribbage --agents random,random --games N --seed S --out dir/ [--parallel]`
- Create: `crates/playtest-cli/src/commands/replay.rs` — `playtest replay path/to/game.jsonl [--tick N]`
- Modify: `crates/playtest-cli/src/main.rs` — `clap` subcommand dispatch
- Create: `crates/playtest-cli/src/game_registry.rs` — static registry mapping `"cribbage"` → `CribbageGame`
- Create: `crates/playtest-cli/src/agent_registry.rs` — string → agent factory
- Test: `crates/playtest-cli/tests/cli_smoke.rs` — uses `assert_cmd`

**Approach:**
- `play` fans game instances across rayon threads when `--parallel` is set; each thread has its own RNG seeded from a master seed
- Output directory gets one JSONL file per game
- `replay` prints states tick-by-tick to stdout; with `--tick N`, dumps just state at tick N
- CLI uses production adapters by default; tests can inject record/playback via environment or CLI flag (e.g., `--tape-in`, `--tape-out`)

**Patterns to follow:** `clap` derive, `anyhow::Result` at the binary boundary, `thiserror` inside libraries.

**Test scenarios:**
- Happy path: `playtest play --game cribbage --agents random,random --games 10 --seed 42` exits 0 and produces 10 JSONL files
- Happy path: `playtest replay <file>` from the above pass exits 0 and prints a nonempty state dump
- Happy path: same `--seed` produces bit-for-bit identical JSONL across two runs (determinism end-to-end)
- Edge case: `--games 0` exits 0 with no files written
- Error path: `--game unknown` exits nonzero with a clear "unknown game" error listing the registry
- Integration: a game played with `--tape-out tape.jsonl` followed by `playtest replay tape.jsonl` reproduces all events
- Performance: `--games 10000 --parallel` on 1 core completes in under 60s (R0.9)

**Verification:**
- Determinism end-to-end confirmed by byte-comparing two full output directories from the same seed
- CLI help text is readable (`playtest --help`, `playtest play --help`)

---

- [ ] **Unit 11: Soak test, determinism audit, and Phase 0 exit-criteria validation**

**Goal:** Prove the Phase 0 exit criteria: 10K games <60s/core (R0.9), zero panics in 100K games (R0.11), complete replayable log for every game (R0.10).

**Requirements:** R0.9, R0.10, R0.11

**Dependencies:** Unit 10

**Files:**
- Create: `crates/playtest-cli/tests/soak_10k.rs` — `#[ignore]` by default, runs 10K games single-core, asserts <60s wall clock
- Create: `crates/playtest-cli/tests/soak_100k.rs` — `#[ignore]`, runs 100K games, asserts zero panics, every log file parses, every replay succeeds
- Create: `crates/playtest-core/tests/determinism_audit.rs` — compiles a list of disallowed calls (`SystemTime::now`, `thread_rng`, `Instant::now` outside the `Clock` port) and fails if they appear in `crates/playtest-core` or `crates/games/*` source
- Create: `docs/BENCHMARKS.md` — log the numbers
- Create: `.github/workflows/soak.yml` — nightly soak job (doesn't block PRs)

**Approach:**
- Determinism audit uses a simple grep over source files — not perfect but catches 95% of regressions
- Soak tests are `#[ignore]` in CI to keep PR feedback fast; nightly runs them
- Any panic is captured with its game seed so it can be reproduced via `playtest replay`

**Test scenarios:**
- Happy path: 10K games completes in <60s on a reference machine (document machine in BENCHMARKS.md)
- Happy path: 100K games completes with zero panics, zero replay failures
- Happy path: determinism audit is green — no `SystemTime::now()` or `thread_rng()` in engine/game code
- Edge case: if a panic occurs, the failing seed is printed so it can be reproduced
- Integration: every one of 100K logs is replayable end-to-end

**Verification:**
- Exit criteria R0.9, R0.10, R0.11 all satisfied and numbered in `docs/BENCHMARKS.md`

---

### Phase 1 — Metrics and analytics spine

- [ ] **Unit 12: Metric registry + game-agnostic metrics**

**Goal:** Define the metric-registry abstraction: games declare which metrics they emit and how to extract them from an event stream. The harness knows how to persist any metric.

**Requirements:** R1.2, R1.3, R1.6

**Dependencies:** Unit 6

**Files:**
- Create: `crates/playtest-metrics/src/registry.rs` — `trait MetricRegistry<G: Game> { fn metric_definitions(&self) -> Vec<MetricDef>; fn extract(&self, log: &GameLog<G>) -> Vec<MetricValue>; }`
- Create: `crates/playtest-metrics/src/definition.rs` — `MetricDef { name: String, kind: MetricKind, description: String }`, `MetricKind::{Scalar(f64), Count(i64), Tag(String)}`
- Create: `crates/playtest-metrics/src/value.rs` — `MetricValue { game_id: Uuid, metric_name: String, player: Option<PlayerId>, value: f64 | i64 | String }`
- Create: `crates/playtest-metrics/src/builtin.rs` — always-on game-agnostic metrics: `game_length_ticks`, `winner`, `end_reason`, `wall_clock_ms`, `agent_names`, `decisions_per_player` (count of agent `choose` invocations)
- Test: `crates/playtest-metrics/tests/registry.rs`

**Approach:**
- Game crates export a `CribbageMetrics` impl of `MetricRegistry<CribbageGame>`; CLI discovers them via the same registry Unit 10 uses
- Keep the trait deliberately simple — a pure function from log → list of metric values. No streaming, no incremental updates yet.
- Metric *extraction* runs at ingestion time, not at query time, so the SQLite table can be queried with plain SQL

**Patterns to follow:** Separate metric *definition* from metric *extraction* so reports can be generated for metrics even if zero games have them yet.

**Test scenarios:**
- Happy path: extracting built-in metrics from a known log produces the expected `game_length_ticks`, `winner`, `end_reason`
- Happy path: a registry with duplicate metric names fails fast at registration (not at query time)
- Edge case: a game with zero events (header + final only) produces sensible built-in metric values
- Integration: round-trip — `MetricDef` serializes to JSON and deserializes identically (for schema persistence)

**Verification:**
- `crates/playtest-metrics` has no dependency on `crates/games/cribbage`
- Built-in metrics tested against at least three different log fixtures

---

- [ ] **Unit 13: Cribbage-specific metrics**

**Goal:** Define and implement the metrics that only make sense for Cribbage, including the reframed per-card design-insight metrics (R1.5) that replace the roadmap's CCG-shaped "per-card" metrics for a fixed-deck game.

**Requirements:** R1.4, R1.5

**Dependencies:** Unit 12

**Files:**
- Create: `crates/games/cribbage/src/metrics.rs` — `CribbageMetrics` impl of `MetricRegistry<CribbageGame>`
- Create: `crates/games/cribbage/src/metrics/game_shape.rs` — game-shape metrics
- Create: `crates/games/cribbage/src/metrics/scoring.rs` — hand/crib/pegging score metrics
- Create: `crates/games/cribbage/src/metrics/per_card.rs` — per-rank design-insight metrics
- Create: `crates/games/cribbage/tests/metrics.rs` — fixture-driven tests
- Create: `crates/games/cribbage/tests/fixtures/` — committed JSONL logs used as test inputs

**Metrics to implement:**

*Game-shape (what did this game look like?):*
- `game_ended_in_phase` — tag: `pegging` / `show` / `crib_count`
- `game_winner_was_dealer` — boolean per game
- `cuts_producing_nibs` — 0/1 per game (starter was a jack)
- `lead_changes` — count of times the scoring leader flipped during the game. Computed from Board state after every scoring event.
- `final_score_margin` — winner_score − loser_score (always ≥ 1 since game ends at 121)

*Scoring breakdown (where did points come from?):*
- `avg_hand_score_player_N` — player N's average show-phase hand score
- `avg_crib_score` — dealer's average crib score
- `avg_pegging_score_player_N` — player N's average total pegging points per game
- `pegging_share_of_total_player_N` — pegging points / final score
- `nibs_contribution_to_dealer` — average dealer points from the cut (0 or 2)

*Per-card design insight (R1.5 — the reframed CCG-style metrics):*
- `card_kept_rate[rank]` — of deals where a player held rank R in their 6 dealt cards, what fraction kept it in their 4-card hand
- `card_discarded_to_own_crib_rate[rank]` — as dealer, rate of discarding rank R to your own crib when held
- `card_discarded_to_opp_crib_rate[rank]` — as non-dealer, rate of discarding rank R to opponent's crib when held
- `win_rate_when_card_in_hand[rank]` — win rate conditional on rank R being in the player's 4-card hand
- `win_rate_when_card_in_crib[rank]` — win rate conditional on rank R being in the crib (credited to whoever owns the crib)

**Approach:**
- Each metric extractor is a single pass over the event stream maintaining a small accumulator
- Per-card metrics aggregate across many games; they're stored in `game_metrics` with `metric_name = "card_kept_rate"` and a `tag` column for the rank. The reporter (Unit 15) produces a per-rank table in the output.
- `lead_changes` is computed by tracking which player was ahead on the Board after every `PegScored`/`ShowScored` event
- Fixtures include: (a) a crib-win game, (b) a pegging-win game, (c) a mid-show win by non-dealer, (d) a nibs-on-cut game, (e) a game with multiple lead changes

**Patterns to follow:** Each metric has a single-line description used in the markdown report — write it at definition time. Per-card metrics share a common aggregation helper to avoid duplication.

**Test scenarios:**
- Happy path: fixture (a) — dealer wins by crib-count → `game_ended_in_phase == "crib_count"`, `game_winner_was_dealer == true`, `final_score_margin ≥ 1`
- Happy path: fixture (c) — non-dealer pegs out mid-show → dealer never counted, `avg_hand_score_dealer` is absent for this game, `avg_pegging_score_non_dealer` is populated
- Happy path: fixture (d) — starter is a jack → `cuts_producing_nibs == 1`, dealer's `nibs_contribution == 2`
- Happy path: fixture (e) — multiple lead changes → `lead_changes >= 2`
- Happy path (per-card): over 1000 games, `card_kept_rate[5]` is much higher than `card_kept_rate[2]` — 5s are famously high-value in Cribbage (easy 15s and runs) and scripted agents should reflect that
- Edge case: rank never held by any player in the sample → per-card metric is absent, not zero (avoids dividing by zero)
- Edge case: game ends before anyone counts their hand → per-player hand-score metrics absent, not 0
- Error path: malformed event sequence (e.g., `DiscardToCrib` for a card not in the player's hand) → extraction reports the game as invalid rather than producing garbage metrics
- Integration: metric extraction over 1000 random-vs-random logs produces no errors, populates all expected fields, and per-rank metrics sum to 100% of deals where that rank was held

**Verification:**
- All 5 fixtures committed and documented
- Per-rank metrics visible in the Unit 15 report as a ranked table
- At least 15 Cribbage-specific metric definitions registered (8 game-shape + scoring + 5 per-card × arities)

---

- [ ] **Unit 14: SQLite ingestion + schema**

**Goal:** Take a directory of JSONL logs and produce a queryable SQLite database.

**Requirements:** R1.1, R1.6

**Dependencies:** Units 12, 13

**Files:**
- Create: `crates/playtest-metrics/src/schema.sql` — `games`, `agent_stats`, `game_metrics` tables
- Create: `crates/playtest-metrics/src/ingest.rs` — `fn ingest_directory(db: &mut Connection, dir: &Path) -> Result<IngestReport>`
- Create: `crates/playtest-metrics/src/query.rs` — canned queries for the reporter (avg / distribution / counts)
- Test: `crates/playtest-metrics/tests/ingest.rs`

**Approach:**
- Schema (minimal):
  - `games(id TEXT PRIMARY KEY, game TEXT, version TEXT, seed INTEGER, started_at INTEGER, ended_at INTEGER, winner INTEGER, end_reason TEXT, config_hash TEXT)`
  - `agent_stats(game_id TEXT, player INTEGER, agent_name TEXT, won BOOLEAN, score INTEGER, PRIMARY KEY(game_id, player))`
  - `game_metrics(game_id TEXT, metric_name TEXT, player INTEGER NULL, tag TEXT NULL, value_numeric REAL NULL, value_text TEXT NULL, PRIMARY KEY(game_id, metric_name, player, tag))` — `tag` column supports per-card metrics keyed by rank, and generalizes to any other category (persona, archetype, etc.) in later phases
- Ingestion is idempotent — re-ingesting the same log overwrites rather than duplicating
- An `ingest_report` returns counts (games, metrics, parse errors) for the CLI to print

**Patterns to follow:** `rusqlite` with `bundled` feature; use prepared statements + transactions for throughput.

**Test scenarios:**
- Happy path: ingest 10 JSONL files → `games` table has 10 rows
- Happy path: ingest the same directory twice → still 10 rows (idempotent)
- Happy path: ingest with a custom `MetricRegistry` produces the registered metrics
- Edge case: malformed JSONL file → ingestion reports the file, continues, does not abort the batch
- Edge case: log with `schema: 99` (future version) → skipped with a clear warning
- Performance: ingesting 10K small logs completes in well under the 30s report budget (leaving room for queries)

**Verification:**
- An ingested DB can be inspected manually with `sqlite3` and the schema is self-explanatory
- `ingest_report` prints a useful one-line summary to stdout

---

- [ ] **Unit 15: `playtest report` subcommand + markdown formatter**

**Goal:** The user-facing reporter. Takes a games directory, ingests it, runs canned queries, writes a markdown report in <30s for 10K games.

**Requirements:** R1.5, R1.6, R1.7

**Dependencies:** Unit 14

**Files:**
- Create: `crates/playtest-cli/src/commands/report.rs` — `playtest report --games <dir> --out report.md [--db path.sqlite]`
- Create: `crates/playtest-metrics/src/reporter.rs` — composes canned queries into report sections
- Create: `crates/playtest-metrics/src/markdown.rs` — simple builder (no templating engine; YAGNI)
- Test: `crates/playtest-cli/tests/report_smoke.rs`

**Approach:**
- Report sections (game-agnostic first, then game-specific):
  - **Summary**: total games, avg length, decisions per player, winner distribution, end-reason breakdown
  - **Per-agent**: win rate, avg final score, avg game length, avg decisions per game
  - **Cribbage: game shape**: phase-of-game-end breakdown, avg lead changes per game, avg final-score margin, nibs rate, dealer-wins-vs-non-dealer-wins
  - **Cribbage: scoring breakdown**: avg hand / crib / pegging scores per player, pegging-share-of-total
  - **Cribbage: per-card design insight** (R1.5) — a ranked table with one row per rank, columns: kept-rate, discarded-to-own-crib, discarded-to-opp-crib, win-rate-when-in-hand, win-rate-when-in-crib. This is the table that surfaces real design questions ("why does holding an 8 correlate with losing?")
- Defer fancy rendering (tables via pretty-printed column alignment only; no charts, no HTML)
- If `--db` is given, re-use the DB instead of re-ingesting

**Patterns to follow:** Report is strictly append-only text generation — no stateful renderer.

**Test scenarios:**
- Happy path: report on 1000 cribbage games produces valid markdown with all expected sections
- Happy path: report includes per-agent win rates summing to 100% (± rounding)
- Edge case: report on 0 games produces a minimal "no games found" report, not a panic
- Edge case: report on a directory with mixed cribbage + future-game logs handles unknown games gracefully (skips with warning)
- Happy path: per-card table in the output report shows distinct kept-rates across ranks (5s kept more than 2s, confirming R1.5 produces meaningful signal even with RandomAgents — the *deal* distribution drives the visible asymmetry)
- Performance: `playtest report --games 10000` completes in <30s on the reference machine (R1.6)
- Integration: full pipeline — `playtest play --games 10000 --out games/` → `playtest report --games games/ --out report.md` produces a readable file

**Verification:**
- Exit criterion R1.6 validated and numbered in `docs/BENCHMARKS.md`
- The generated report is copy-pasteable into GitHub without visual glitches
- Per-card design-insight table is present and non-trivially populated, demonstrating that Phase 1 produces real insight on a fixed-deck game even with Random agents (R1.5)

## System-Wide Impact

- **Interaction graph.** `GameLoop` is the single integration point; it composes `Game` + `[Agent]` + ports. Every other crate is either a direct collaborator of one of those (game, agent, adapter) or a downstream consumer of the event log (metrics).
- **Error propagation.** Agent errors surface through `GameLoop` with game context attached (seed, tick, player) so a failing 100K-game soak run always produces a minimal reproducer. Adapter errors propagate through the port `Result` type; `GameLoop` wraps them with tick context.
- **State lifecycle risks.** (1) `Record` adapters writing tapes mid-game — if the process crashes, a partial tape may be left; ingestion must tolerate this. (2) JSONL writer must flush on game end; otherwise a crashed run leaves a half-written log. (3) SQLite ingestion is idempotent, so partial batches can be re-run safely.
- **API surface parity.** The `Agent` trait is the single seam for Phase 3 (stdio) and Phase 4 (personas). Anything that assumes synchronous agent behavior in Phase 0 is a future bug. This plan deliberately uses `async_trait` throughout to forestall that.
- **Integration coverage.** Per-unit tests cover the pieces; Units 5, 6, 9, 10, 14, and 15 have explicit integration tests that span multiple crates. The full-chain test is in Unit 15 (play → log → ingest → report).
- **Unchanged invariants.** *Nothing exists yet to invalidate.* But the invariants this plan *introduces* are the ones Phases 2–8 must honor: (a) determinism-per-seed, (b) no direct RNG/clock/filesystem access outside the port crate, (c) agents never mutate game state, (d) games never depend on the harness's choice of storage backend.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Effect DSL / `Game` trait is over- or under-abstracted, causing pain when the second game lands | Unit 4's test `Game` impl (rock-paper-scissors) + Unit 7–9's Cribbage impl together exercise two very different games. If both fit in <120 lines each of trait-implementing code, the abstraction is right-sized. If either is straining, revise before Phase 2. |
| Cribbage scoring is a classic bug farm (double-count nobs, wrong flush-in-crib rules, run detection on stack vs. hand) | Unit 8/9 require ≥ 30 pegging + 40 show scenarios, including hand-scored edge cases like the 29-hand and order-sensitive game-termination-during-show. Runs and flushes each have their own test file. |
| Non-determinism sneaking in via `HashMap` iteration order, `SystemTime::now()`, or implicit `thread_rng` | Unit 11's determinism audit grep; explicit `Rng` port; `#[deny(clippy::disallowed_methods)]` with a project list that includes the common offenders. |
| 10K-games-in-60s performance target misses (R0.9) | Benchmark in Unit 10 before Unit 11 so we know the delta early. If close to the line, the fix is almost always serialization cost — switch JSON writer to a faster one (e.g. `simd-json`) rather than rewrite the game logic. |
| `async_trait` overhead on sync agents is measurable | `RandomAgent` returns `std::future::ready(...)`; zero heap allocation in steady state. Measure in Unit 5; if non-negligible, switch to an `impl Future` GAT approach. |
| JSONL log grows too fast for 100K soak (disk, not time) | 100K cribbage games × ~200 events/game × ~100 bytes/event = ~2 GB. Mitigation: soak test writes to `/tmp` and cleans as it goes; real users won't soak at this scale. Document this in `BENCHMARKS.md`. |
| SQLite ingestion slow enough to eat the 30s report budget | `rusqlite` with `bundled`, use a single transaction for the whole batch, prepared statements, `PRAGMA synchronous=OFF` for ingestion. Measure in Unit 14. |
| Record/playback tape schema drift between Phase 0 and Phase 2 as new ports appear | Tape files carry a schema version in their header; playback refuses mismatched versions with a clear error. Accept that old tapes may need regeneration when ports change — they're test fixtures, not production data. |

## Documentation / Operational Notes

- `README.md` at the workspace root is minimal (one-paragraph description, build instructions, `playtest --help` pointer).
- `docs/BENCHMARKS.md` records the reference-machine numbers for exit criteria R0.9, R0.11, R1.5.
- `docs/ARCHITECTURE.md` is deferred to the end of Phase 1 — write it after the abstractions have survived contact with Cribbage.
- No operational rollout concerns: everything here is a local CLI.

## Sources & References

- **Origin document:** [playtest-roadmap.md](../playtest-roadmap.md)
- Cribbage rules reference: standard 2-player / 6-card / 121-point / full show + crib + nibs + nobs
- Rust determinism: `rand_chacha::ChaCha20Rng` (portable-across-platforms deterministic RNG)
- CommunicationMod pattern (Slay the Spire) — referenced for future Phase 3 stdio protocol
- ISMCTS paper (Cowling/Powley/Whitehouse 2012) — referenced for future Phase 2 agent
