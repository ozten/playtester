---
title: "feat: Web spine (SSE) + ShipWreck multi-game proof + Phase 2 heuristics/ISMCTS"
type: feat
status: shipped
date: 2026-04-21
origin: docs/plans/2026-04-21-001-feat-playtester-phases-0-1-plan.md
supersedes_section: "Phase 8 (Interactive designer loop) → web frontend decision"
---

# feat: Web spine (SSE) + ShipWreck multi-game proof + Phase 2 heuristics/ISMCTS

## Overview

Three intertwined workstreams land in this plan, in this order:

1. **Web spine.** Stand up an HTTP + Server-Sent Events surface so a SvelteKit frontend (separate repo) can watch live self-play, browse saved logs, and render reports. Drops the TUI plan entirely. A new `playtest-api` crate owns the wire types; a new `playtest-server` crate owns the axum router and the live-game broadcaster. The CLI gains a thin `playtest serve` subcommand.
2. **ShipWreck, full game.** A second playable game — structurally very different from Cribbage (multi-player, shared draft of wreckage cards, event cards with targeting, construction costs, tie-breakers). This is the only way to validate that the `Game` trait, `MetricRegistry`, event log, and server plumbing truly generalized. Any trait-shape bugs from Phase 0–1 must surface and be fixed here, before heuristic agents harden them in place.
3. **Phase 2 — heuristic agents and ISMCTS.** A shared evaluation-function shape, a `GreedyAgent` (one-ply lookahead), per-game `HeuristicAgent` impls, and a generic `ISMCTSAgent`. Adds one new method to the `Game` trait: `determinize(state, observer, rng) -> State` — the minimum surface needed for Information-Set MCTS to sample a consistent hidden-info state during tree descent.

Ordering is deliberate: web spine first because it's the smallest surface and unlocks visual dogfooding of everything downstream; ShipWreck second because heuristics written against a single game are almost always game-specific scaffolding masquerading as trait design; Phase 2 last, on a validated two-game foundation.

## Problem Frame

Phases 0 and 1 shipped (Units 1–15 of the origin plan): deterministic engine, ports/adapters quartet, Cribbage end-to-end, JSONL event log, SQLite ingestion, markdown reports. Exit criteria on Phase 0 (10K games <60s, 100K-game soak with zero panics, every log replayable) and Phase 1 (10K-game report in <30s) were all met.

Three concerns now drive the next planning pass:

- **Multi-game risk is latent.** The architecture was built for two games but only one has been built. The roadmap's "raft and resources" board game (ShipWreck, see `docs/shipwreck.md`) is structurally very different from Cribbage, and retrofitting the `Game` trait after Phase 2 agents compile against it would be painful. The user's first question — *"this simulator needs to support more than one game, have we lost track of that?"* — makes this explicit.
- **The TUI plan is dropped.** The origin plan deferred all UI to Phase 8 as "Web UI or TUI." The decision is now made: SvelteKit frontend, separate repo, SSE (not polling) for live updates. This plan dictates the Rust-side interface.
- **Phase 2 is the natural next feature-ward step** (roadmap ROI ★★★★ for heuristic + ISMCTS), but only once the two foundations above hold.

## Requirements Trace

Carries forward from origin `playtest-roadmap.md` Phase 2 (R2.x) and adds three families of requirements for the web spine (R8.x, pulled forward from roadmap Phase 8) and multi-game validation (R0.14 revisited — originally "the Game trait is abstract enough that a second, structurally different game plugs in without harness changes").

### Web spine (pulled forward from Phase 8)

- **R8.1** A `playtest-server` binary serves HTTP over a configurable port
- **R8.2** REST endpoints: list supported games + configs, start a run, list runs, fetch run metadata, list games in a run, fetch a single game's metadata, fetch a game's paginated event list (for replay), generate + fetch a markdown report
- **R8.3** Live SSE stream per game emits the JSONL event log shape as typed SSE frames in real time as the engine emits them, with tick-scoped `id:` so clients can resume mid-stream
- **R8.4** Live SSE stream per run emits `game-started` / `game-finished` / `run-complete` frames with progress counters
- **R8.5** Every protocol type is defined in `playtest-api`, a crate with zero dependency on `playtest-server`, `playtest-core`, `playtest-cli`, or any game crate — so the SvelteKit repo can consume a frozen wire contract without dragging engine code along
- **R8.6** The wire contract is versioned (`api_version` in every response envelope) and shipped as `docs/api-contract.md` alongside a machine-readable schema (OpenAPI JSON emitted by a `playtest-server` subcommand) that SvelteKit codegen can consume
- **R8.7** The server honors graceful shutdown: in-flight runs finish, SSE streams send a `shutdown` frame, no partial JSONL files left on disk
- **R8.8** No authentication in this phase — server binds to `127.0.0.1` by default; `--bind 0.0.0.0` requires an explicit flag and logs a warning

### Multi-game proof (ShipWreck)

- **R0.14 (re-asserted)** A second, structurally different game plugs into the engine without harness changes. ShipWreck is the canonical stress test: multi-player, shared information (face-up wreckage), event cards with targeting, per-player unique cards, construction costs with exact-match resources, end condition by deck exhaustion rather than score target.
- **R9.1** Support 2–4 players (configurable); default 2
- **R9.2** All ShipWreck card types modeled: player cards (7 unique characters), base raft cards, raft extension cards, equipment upgrade cards (5 types), wreckage resource cards (5 types × 30), wreckage event cards (3 types)
- **R9.3** All turn actions supported: place player card, pick up wreckage, play event card, build equipment, extend raft
- **R9.4** Event-card resolution: shark (targeted destruction of one upgrade/extension), typhoon (all players lose one upgrade/extension, chosen by each), flying fish (one-turn food substitute)
- **R9.5** End condition: wreckage deck exhausted. Winner: most rescue points. Tie-breakers in order: raft length, then total invention count (equipment upgrades), then tie
- **R9.6** Random-vs-Random self-play terminates in 100% of 1000 runs with a valid `GameResult`
- **R9.7** ShipWreck registers its own metrics via `MetricRegistry<ShipWreck>` — no harness changes needed
- **R9.8** `playtest play --game shipwreck ...` works exactly like the Cribbage path; the CLI code path discovered no Cribbage-specific assumptions

### Phase 2 — heuristic + ISMCTS agents

- **R2.1** `GreedyAgent<G>` — one-ply lookahead over legal actions using a game-provided evaluation function
- **R2.2** Per-game `HeuristicAgent` (one for Cribbage, one for ShipWreck) implementing an evaluation function tuned by hand. Quality bar: Heuristic beats Random >90% over 10,000 games per game
- **R2.3** `ISMCTSAgent<G>` — Information-Set MCTS with determinization via a new `Game::determinize` method. Parameterized by time-per-decision or iteration budget. Quality bar: ISMCTS beats Heuristic >65% at its strongest reasonable setting
- **R2.4** 10K-games-per-pair matchup matrix for a 20-agent pool (e.g., 5 distinct agent kinds × 4 parameter configs each = 20 agent instances, forming a 20×20 matrix) runs in under 30 minutes on a reference laptop. This is a manual benchmark recorded in `docs/BENCHMARKS.md`, not a CI-gated test.
- **R2.5** New CLI subcommand `playtest matchup --games N --agents <...>` produces a markdown matchup matrix, readable in the same surface as `playtest report`

## Scope Boundaries

- **No LLM integration.** `LlmClient` port still has no production adapter. Phase 3.
- **No TerminalAgent.** Human play is still deferred to Phase 3.
- **No personas.** Phase 4.
- **No compare / counterfactual subcommand.** Phase 6.
- **No MAP-Elites.** Phase 7 — but ShipWreck is the game that will make those metrics non-degenerate when Phase 7 lands.
- **No production deployment.** Server is localhost-only; TLS, auth, rate limiting all deferred.
- **No SvelteKit code in this repo.** The frontend lives in a separate repo consuming this repo's `docs/api-contract.md` + generated OpenAPI schema.
- **No WebSocket support.** SSE is strictly sufficient — the server never needs to receive a stream from the client. Control flows through REST; notifications flow through SSE.
- **No per-game code in `playtest-server` or `playtest-api`.** Those crates are strictly game-agnostic; ShipWreck-specific or Cribbage-specific types live in their game crates.

### Deferred to Separate Tasks

- **SvelteKit frontend**: separate repo, separate plan. This plan only dictates the server-side contract.
- **Stdio agent protocol + LLMAgent**: Phase 3, next plan.
- **Personas**: Phase 4, next plan.
- **Post-game LLM critique**: Phase 5.
- **Compare/counterfactual subcommand**: Phase 6.
- **MAP-Elites deckbuilding search**: Phase 7, and depends on ShipWreck or a future game with a deck-construction mechanic.

## Context & Research

### Relevant Code and Patterns

Phase 0–1 shipped (see origin plan for full context). Key pieces this plan builds on:

- **`Game` trait** (`crates/playtest-core/src/game.rs`) — associated types for `State`, `Action`, `Event`, `PublicView`, `Config`. Methods: `initial_state`, `next_actor`, `legal_actions`, `apply_action`, `resolve_chance`, `apply_event`, `public_view`, `game_over`. This plan adds one method (`determinize`) and leaves everything else untouched. If ShipWreck forces additional changes, that is itself the risk this plan exists to surface.
- **`Agent` trait** (`crates/playtest-core/src/agent.rs`) — `async fn choose(view, legal) -> usize`. Unchanged. `GreedyAgent`, `HeuristicAgent`, `ISMCTSAgent` all implement this trait.
- **`GameEventSink` port** (`crates/playtest-ports/src/game_event_sink.rs`) — string-oriented, `emit(&str)` + `flush()`. A new `BroadcastGameEventSink` adapter in `playtest-adapters` wraps a production sink and tees every line to a `tokio::sync::broadcast::Sender<String>` that feeds SSE clients. No port changes required — the input/output port asymmetry (origin plan's Key Technical Decisions) anticipated exactly this.
- **`MetricRegistry<G>`** (`crates/playtest-metrics/src/registry.rs`) — `CribbageMetrics` already implements it; `ShipWreckMetrics` will be a second impl. Registry pattern validated.
- **`playtest play` command** (`crates/playtest-cli/src/commands/play.rs`) — the agent+game+CLI wiring pattern the server and `matchup` command mirror. Game and agent registries (`game_registry.rs`, `agent_registry.rs`) are the extension points.
- **JSONL event log** (`crates/playtest-log/`) — generic over `G::Event: Serialize`. Schema v2 includes `started_at` / `finished_at`. The SSE stream and the JSONL writer emit the *same* lines — the server just tees them to two sinks.

### Institutional Learnings

The `docs/solutions/` directory still does not exist. This plan's post-implementation review is the next candidate moment for `ce-compound` writeups — the web spine contract design, the ShipWreck trait-surgery list (if any), and ISMCTS's determinization integration are all candidate learning topics.

### External References

- **Cowling, Powley, Whitehouse (2012)** — Information Set Monte Carlo Tree Search. The reference algorithm for `ISMCTSAgent`. Key points: single-observer (SO-ISMCTS) is sufficient for imperfect-information games with chance; determinization samples a concrete state at the start of each iteration.
- **UCT (Kocsis & Szepesvári, 2006)** — the selection policy inside ISMCTS. Standard c = √2 exploration constant; tunable per game.
- **Axum 0.8** — tokio-native HTTP router. Chosen over actix-web for its minimal surface and first-class tower integration.
- **Server-Sent Events spec (WHATWG)** — `text/event-stream` MIME, `id: <tick>` for resumption via `Last-Event-ID` header, `: <comment>` lines for heartbeat.
- **`tokio::sync::broadcast`** — MPMC broadcast channel for fanning engine events to N concurrent SSE subscribers. Dropped messages are surfaced to slow clients as `Lagged(n)`; the server converts those to a `lagged` SSE frame so the client can refetch the JSONL.
- **`ts-rs` / `typeshare`** — Rust→TypeScript codegen options. Decision deferred to Unit 16; OpenAPI may obviate the choice.
- **CommunicationMod stdio protocol** (Slay the Spire) — NOT used in this plan (Phase 3 concern) but the SSE event shape is designed to remain compatible: each SSE frame is the same JSON a stdio agent would receive.

## Key Technical Decisions

- **SSE, not WebSocket.** Control flows REST→server; notifications flow server→client. The engine never needs to receive anything mid-game from the frontend. SSE gets us trivial HTTP semantics, server-managed backpressure, automatic reconnection with `Last-Event-ID`, and no additional protocol surface.
- **Two new crates, not one.** `playtest-api` (wire types only, no server) and `playtest-server` (axum + state + routing). Rationale: the SvelteKit repo can reference just `playtest-api` through OpenAPI schema without accidentally importing tokio. Keeps the "what agreement does the frontend depend on" boundary crystal clear.
- **Server tees events; it does not replace the JSONL writer.** Every run still produces JSONL files on disk. A `BroadcastGameEventSink` wraps the production sink, writes through, *and* publishes each line to a `broadcast::Sender<String>`. Rationale: JSONL is the source of truth; the SSE stream is a view. A server crash loses only the SSE subscribers, not the game history.
- **No auth in this phase.** Localhost-bound by default; explicit flag to bind publicly and a warning on log. Rationale: YAGNI. When this runs anywhere beyond localhost we'll add a reverse proxy or a minimal token check — neither is a good use of time now.
- **`Game::determinize` is a new trait method, not a separate trait.** Rationale: every game that wants ISMCTS needs it; bundling it avoids a `G: Game + Determinize` bound at every call site. Games that don't support ISMCTS can `unimplemented!()` (the default-impl pattern) — but both Cribbage and ShipWreck implement it.
- **The determinize invariant is: `public_view(determinize(s, p, rng), p) == public_view(s, p)`** — the determinized state and the true state are indistinguishable from the observer's view. This is the single correctness property for determinization, stated once here and referenced elsewhere. Property-tested in Unit 19 (Cribbage) and Unit 22 (ShipWreck).
- **The trait addition lands before ShipWreck.** `Game::determinize` must exist before Unit 22 (ShipWreck's `impl Game for ShipWreckGame`), so the trait is added in Unit 19 — before ShipWreck primitives — with the Cribbage impl + the two existing test-game impls (`TallyGame`, `AlreadyDone`) brought along in the same unit. ShipWreck's determinize is folded into Unit 22 alongside the rest of its `Game` impl, not a separate unit.
- **ShipWreck default is 2 players, supports 2–4.** Rationale: the box describes it as a party game, but balance testing is cleanest at 2. `CribbageGame::Config` already carries a player-count field analog (`dealer_first`); `ShipWreckConfig::num_players` joins it.
- **Wreckage deck exhaustion is the end signal, not score.** Confirmed from `docs/shipwreck.md`: "Once all of the wreckage cards are gone, the game ends." The engine detects this in `game_over` when the deck + face-up-per-player pools are empty.
- **Shared face-up wreckage is public information.** Each player's face-up wreckage row is visible in every player's `PublicView`. Telescope upgrades change the *legal reach* but not visibility.
- **`Agent<G>` parametricity survives Phase 2.** `GreedyAgent`, `HeuristicAgent`, `ISMCTSAgent` all take `G: Game` bounds plus a small `EvaluationFn<G>` or `RolloutPolicy<G>` bound. No new traits in `playtest-core`; all lives in `playtest-agents`.
- **Evaluation functions are plain functions, not a trait.** `type EvalFn<G> = fn(&G::PublicView, PlayerId) -> f64`. Rationale: agents don't compose evaluation functions at runtime; they're picked at construction. A trait would add dispatch cost without adding expressiveness.
- **ISMCTS tree-reuse across turns is deferred.** First implementation rebuilds the tree each decision. If Phase 2 exit-criteria miss (ISMCTS not >65% over Heuristic), tree-reuse is the first optimization to try.
- **Matchup subcommand is a thin wrapper over `play`.** It just runs N games per (agent_a, agent_b) pairing and compiles a matrix. No new metrics layer; uses `playtest-metrics` query layer.
- **`playtest serve` is in the existing `playtest-cli` binary, not a new binary.** It delegates to `playtest_server::run()`. Rationale: one binary to teach users; the crate split is about compile-time dependency hygiene, not runtime packaging.

## Open Questions

### Resolved During Planning

- **SvelteKit location**: separate repo, out of scope for this plan. This plan produces `docs/api-contract.md` + OpenAPI schema as the shared contract.
- **Polling vs SSE**: SSE.
- **Player count in ShipWreck**: 2–4 configurable, default 2.
- **ShipWreck end condition**: wreckage deck exhausted (per `docs/shipwreck.md`).
- **Auth**: none, localhost-only.
- **Multi-game proof depth**: full playable ShipWreck.
- **Crate split**: two new crates (`playtest-api`, `playtest-server`).

### Deferred to Implementation

- **TypeScript codegen strategy**: OpenAPI (via `utoipa` or similar) vs `ts-rs` attribute macros vs `typeshare`. All three are viable; the choice depends on whether the SvelteKit repo is already using one. Defaulting to OpenAPI for language-agnostic consumption; revisit if codegen is awkward.
- **SSE heartbeat interval**: start at 15s; tune once real proxies show timeout behavior.
- **ISMCTS exploration constant `c`**: start at √2; tune per game when exit criteria are evaluated.
- **Number of determinizations per ISMCTS iteration**: 1 per iteration (SO-ISMCTS). Only revisit if variance is a problem.
- **ShipWreck card pool balance**: the `docs/shipwreck.md` counts are the starting point; if 1000-game random self-play reveals structural issues (e.g., games always end in <10 turns because wreckage distribution is wrong), note and flag — this is a game-design issue, not an engine one.
- **Exact `Event` taxonomy for ShipWreck**: will emerge during Units 20–23 construction. Target surface: `DealPlayerCard`, `DealWreckage`, `PickWreckage`, `PlacePlayer`, `BuildEquipment`, `ExtendRaft`, `PlayEvent`, `EventResolved`, `ResourceSpent`, `EndGame`. Finalize when the turn-flow tests make the right seams obvious.
- **Handling of `Config` per game in the API**: the `POST /runs` body needs to accept a game-specific config blob. Either untyped `serde_json::Value` at the API boundary with per-game parsing inside the dispatcher, or a discriminated-union type. Defer until Unit 16 sees the concrete configs Cribbage and ShipWreck expose.

## Output Structure

    playtester/
    ├── Cargo.toml                            # workspace root — adds 3 new crates
    ├── docs/
    │   ├── api-contract.md                   # NEW: server/client wire contract for SvelteKit
    │   └── plans/
    │       └── 2026-04-21-002-...-plan.md    # this file
    └── crates/
        ├── playtest-api/                     # NEW: pure wire types (REST + SSE)
        │   └── src/
        │       ├── lib.rs
        │       ├── version.rs                # api_version constant + envelope types
        │       ├── error.rs                  # ApiError enum (shared with server)
        │       ├── runs.rs                   # CreateRunRequest/Response, RunSummary, RunStatus
        │       ├── games.rs                  # GameSummary, GameMetadata, EventPage
        │       ├── sse.rs                    # SseFrame enum (header, event, final, lagged, shutdown, heartbeat)
        │       └── registry.rs               # GameRegistryEntry, AgentRegistryEntry
        ├── playtest-server/                  # NEW: axum router + broadcast fan-out
        │   ├── src/
        │   │   ├── lib.rs                    # pub fn run(config) -> Result<()>
        │   │   ├── state.rs                  # AppState: active runs map, broadcasters
        │   │   ├── routes/
        │   │   │   ├── mod.rs
        │   │   │   ├── health.rs
        │   │   │   ├── runs.rs               # POST /api/runs, GET /api/runs, GET /api/runs/:id
        │   │   │   ├── games.rs              # GET /api/runs/:id/games, events, stream
        │   │   │   ├── registry.rs           # GET /api/games-registry
        │   │   │   └── reports.rs            # POST /api/reports, GET /api/reports/:id
        │   │   ├── sse.rs                    # SSE encoder: LogRecord JSON line -> SseFrame
        │   │   ├── runner.rs                 # drives runs via playtest-cli's library code
        │   │   └── schema.rs                 # OpenAPI dump (utoipa or hand-rolled)
        │   └── tests/
        │       ├── server_smoke.rs
        │       └── sse_contract.rs
        ├── playtest-adapters/
        │   └── src/game_event_sink/
        │       └── broadcast.rs              # NEW: BroadcastGameEventSink<Inner>
        ├── playtest-core/
        │   └── src/game.rs                   # MODIFY: add fn determinize(...)
        ├── playtest-agents/
        │   ├── src/
        │   │   ├── eval.rs                   # NEW: type EvalFn<G>
        │   │   ├── greedy.rs                 # NEW: GreedyAgent<G>
        │   │   ├── heuristic.rs              # NEW: HeuristicAgent<G>
        │   │   └── ismcts/
        │   │       ├── mod.rs                # NEW: ISMCTSAgent<G>
        │   │       ├── node.rs               # tree node types
        │   │       ├── ucb.rs                # selection policy
        │   │       └── rollout.rs            # default random rollout
        │   └── tests/
        │       ├── greedy_agent.rs
        │       ├── heuristic_agent.rs
        │       └── ismcts_agent.rs
        └── games/
            ├── cribbage/
            │   └── src/
            │       ├── determinize.rs        # NEW: Cribbage-specific determinization
            │       └── heuristic.rs          # NEW: Cribbage evaluation function
            └── shipwreck/                    # NEW: the second game
                ├── Cargo.toml
                └── src/
                    ├── lib.rs
                    ├── card.rs               # PlayerCard, RaftCard, EquipmentCard, ItemCard, EventCard
                    ├── pool.rs               # static card definitions from docs/shipwreck.md
                    ├── raft.rs               # Raft struct: base + extensions + upgrade slots
                    ├── state.rs              # GameState, Phase, per-player state
                    ├── action.rs             # Action enum
                    ├── event.rs              # Event enum
                    ├── rules.rs              # ShipWreckGame impl of Game trait
                    ├── events/               # event-card resolution (shark, typhoon, flying fish)
                    ├── scoring.rs            # rescue points + tie-breakers
                    ├── metrics.rs            # ShipWreckMetrics impl of MetricRegistry
                    ├── determinize.rs        # ShipWreck determinization for ISMCTS
                    └── heuristic.rs          # ShipWreck evaluation function

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

### 1. Server architecture

```text
 SvelteKit client (separate repo)
          │
   REST │ SSE
          ▼
 ┌───────────────────────────┐      ┌──────────────────┐
 │   playtest-server (axum)  │─────►│  playtest-api    │
 │  ┌─────────┬────────────┐ │      │  (wire types)    │
 │  │ routes/ │  state.rs  │ │      └──────────────────┘
 │  │         │  (AppState)│ │
 │  └────┬────┴──────┬─────┘ │
 │       │           │       │
 │   runner          broadcaster
 │       │           ▲
 │       ▼           │ tee
 │  ┌──────────────────────┐
 │  │ BroadcastGameEventSink│───► tokio::broadcast ──► SSE subscribers
 │  │   wraps ProductionSink│
 │  │                       │
 │  │   also writes JSONL   │───► disk (source of truth)
 │  └──────────────────────┘
 └───────────────────────────┘
          │
   spawns│ per run
          ▼
  playtest-core GameLoop ← identical code path as `playtest play`
```

### 2. Wire contract (REST + SSE at a glance)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/health` | Liveness / version |
| `GET` | `/api/games-registry` | List available games + their `Config` JSON schema |
| `GET` | `/api/agents-registry` | List available agent kinds |
| `POST` | `/api/runs` | Start a run: `{game, agents[], games: N, seed, config}` → `{run_id, status}` |
| `GET` | `/api/runs` | List runs (paginated) |
| `GET` | `/api/runs/:run_id` | Run metadata + status |
| `GET` | `/api/runs/:run_id/stream` | SSE: `game-started`, `game-finished`, `run-complete` |
| `GET` | `/api/runs/:run_id/games` | List games in a run |
| `GET` | `/api/runs/:run_id/games/:game_id` | Game metadata |
| `GET` | `/api/runs/:run_id/games/:game_id/events?offset=N&limit=M` | Paginated event list for replay |
| `GET` | `/api/runs/:run_id/games/:game_id/stream` | SSE: live-if-running, or full-log-then-close-if-done |
| `POST` | `/api/reports` | Generate a markdown report from a run — `{run_id}` → `{report_id, status}` |
| `GET` | `/api/reports/:report_id` | Report metadata |
| `GET` | `/api/reports/:report_id/markdown` | Raw markdown |

**SSE frame shape** (`SseFrame` in `playtest-api`):

```text
event: header
id: 0
data: {"schema":2,"game":"cribbage","version":"0.1.0","seed":12345, ...}

event: event
id: 1
data: {"tick":1,"payload":{"DealCard":{"player":0,"card":"AH"}}}

...

event: final
id: 217
data: {"winner":0,"reason":"score121","scores":[121,98], ...}

: heartbeat (every 15s while live)
```

Clients reconnect with `Last-Event-ID: <n>`. The server reads tick `n+1..=current_tick` from the JSONL file as SSE frames, then subscribes to the live broadcast. See Unit 17's Approach for the race-handling detail. Server shutdown closes the connection (no special frame).

### 3. Game trait extension for ISMCTS

```text
trait Game {
    // ... existing methods unchanged ...

    /// Sample a concrete state consistent with what `observer` can see.
    /// Used by ISMCTS to determinize hidden information at iteration start.
    /// `Self::public_view(determinize(state, observer, rng), observer)` must
    /// equal `Self::public_view(state, observer)` — the determinized state
    /// and the true state are indistinguishable from the observer's view.
    fn determinize(
        &self,
        state: &Self::State,
        observer: PlayerId,
        rng: &mut dyn Rng,
    ) -> Self::State;
}
```

The determinize contract — *"observer sees the same thing in both states"* — is the correctness invariant. Every game-specific implementation must preserve it; the shared ISMCTS agent trusts it.

### 4. ISMCTS loop shape (SO-ISMCTS, single-observer)

```text
fn choose(view, legal):
    root = Node::new(view)
    for _ in 0..iteration_budget:
        determinized_state = Game::determinize(inferred_state, self.player, rng)
        leaf = tree_policy(root, determinized_state)   # UCB1 selection among visited legal moves
        reward = rollout(leaf.state)                   # random rollout to terminal
        backprop(leaf, reward)
    return root.best_child_index()                     # max visit count
```

The inferred state is a reconstruction: the engine passes the full game state to the agent through a privileged path only in testing; in the normal path, `self.player` holds a *belief state* updated from the `PublicView`s it has seen. Unit 26 makes this explicit.

### 5. ShipWreck turn-flow shape

```text
Phase::Setup ─► dealt player cards, dealt 6 wreckage cards to each hand,
                remaining wreckage cards placed face-up 1 per player, rotating
Phase::Play ──► loop over players:
                next_actor = Actor::Player(current_player)
                legal_actions = { ExtendRaft?, PlacePlayerCard(?), PickWreckage(?),
                                  PlayEventCard(?), BuildEquipment(?), EndTurn }
Phase::Play ─── when pick would empty deck + face-up → game_over check

game_over: wreckage deck empty AND all face-up pools empty
          AND no player has a pending event-card chain to resolve
result: max rescue_points wins; tie → longest raft; tie → most equipment; tie → tie
```

### 6. Integration sequence diagram (SSE for a live game)

```text
Client                   playtest-server           broadcaster           engine (GameLoop)
  │                            │                        │                      │
  │──POST /api/runs──────────►│                        │                      │
  │                            │──spawn run─────────────┼─────────────────────►│
  │◄──{run_id}──────────────── │                        │                      │
  │                            │                        │◄────emit(header)──── │
  │──GET /api/runs/.../stream─►│                        │                      │
  │                            │──subscribe(rid)───────►│                      │
  │◄──event: header───────────│                        │                      │
  │                            │                        │◄────emit(event)──── │
  │◄──event: event────────────│                        │                      │
  │  :heartbeat (every 15s)    │                        │                      │
  │                            │                        │◄────emit(final)──── │
  │◄──event: final────────────│                        │                      │
  │◄──[stream closes]          │                        │                      │
```

## Implementation Units

### Web spine (Units 16–18)

- [x] **Unit 16: `playtest-api` crate — wire types and versioned envelope**

**Goal:** Define every request, response, and SSE frame type the SvelteKit frontend will consume. Zero runtime dependencies on axum, tokio, or any engine code — just `serde` and `serde_json`.

**Requirements:** R8.5, R8.6

**Dependencies:** none (new leaf crate)

**Files:**
- Create: `crates/playtest-api/Cargo.toml`
- Create: `crates/playtest-api/src/lib.rs` (re-exports + `pub const API_VERSION: &str = "1.0.0"`)
- Create: `crates/playtest-api/src/version.rs` (envelope types: `ApiResponse<T> { api_version, data, errors }`)
- Create: `crates/playtest-api/src/error.rs` (`ApiError { code: ApiErrorCode, message, details }`)
- Create: `crates/playtest-api/src/runs.rs` (`CreateRunRequest { game, agents, games_count, seed, config }`, `RunSummary`, `RunStatus::{Pending, Running, Completed, Failed}`)
- Create: `crates/playtest-api/src/games.rs` (`GameSummary`, `GameMetadata`, `EventPage { offset, limit, total, events: Vec<LogLineDto> }`)
- Create: `crates/playtest-api/src/sse.rs` (`SseFrame::{Header(JsonValue), Event(JsonValue), Final(JsonValue), Heartbeat }` — frames carry pre-serialized JSON from the log so no re-serialization is needed server-side. `Lagged` and `Shutdown` variants intentionally excluded — localhost-only scope makes the 1024-line broadcaster buffer ample, and graceful shutdown is communicated via connection close)
- Create: `crates/playtest-api/src/registry.rs` (`GameRegistryEntry { id, display_name, config_schema: JsonValue }`, `AgentRegistryEntry`)
- Test: `crates/playtest-api/tests/roundtrip.rs` (serialize → deserialize → eq for every public type)

**Approach:**
- Use `serde_json::Value` to carry game-specific `Config` blobs and per-tick event payloads — the API crate must stay game-agnostic. Unit 17's dispatcher deserializes these on arrival.
- Every response is wrapped in `ApiResponse<T>` so the `api_version` is always present and error shapes are uniform.
- `SseFrame` variants tag-match the `event: <name>` SSE field; `serde`'s `#[serde(tag = "kind", content = "data")]` on a separate internal enum handles the JSON shape.
- Derive `JsonSchema` (via `schemars`) on every public type to enable Unit 18's OpenAPI dump.

**Patterns to follow:** `thiserror`-free — no runtime errors live in this crate. All enums derive `Serialize + Deserialize + Debug + PartialEq + Eq + Clone`.

**Test scenarios:**
- Happy path: every request type roundtrips through JSON without loss
- Happy path: `SseFrame::Event` serializes with `"kind":"event"` and a `data` payload that is just the original JSONL event line content
- Edge case: `ApiResponse::errors` can carry multiple errors (batch validation)
- Edge case: unknown `RunStatus` variant deserialization fails cleanly (forward-compat check — reject, don't silently drop)
- Integration: `ApiError` → HTTP status mapping table is in this crate (lookup function), so `playtest-server` doesn't re-invent it

**Verification:**
- `crates/playtest-api` has no dependency on any workspace crate
- `cargo check -p playtest-api --no-default-features` succeeds (it has no features; this just proves zero cruft)
- Every public type appears in the JSON-schema dump that Unit 18 produces

---

- [x] **Unit 17: `playtest-server` crate — axum router, broadcast fan-out, run supervisor**

**Goal:** HTTP server that exposes every endpoint in the wire contract, runs games through the same `GameLoop` the CLI uses, and streams events to SSE subscribers in real time.

**Requirements:** R8.1, R8.2, R8.3, R8.4, R8.7, R8.8

**Dependencies:** Unit 16, plus the existing `playtest-core`, `playtest-adapters`, `playtest-log`, `playtest-metrics`, `playtest-cli` (for its registries)

**Files:**
- Create: `crates/playtest-server/Cargo.toml`
- Create: `crates/playtest-server/src/lib.rs` (`pub struct ServerConfig { bind: SocketAddr, data_dir: PathBuf }`, `pub async fn run(cfg: ServerConfig) -> Result<()>`)
- Create: `crates/playtest-server/src/state.rs` (`AppState { active_runs: DashMap<RunId, RunHandle>, data_dir }`, `RunHandle { broadcaster: broadcast::Sender<String>, status: watch::Receiver<RunStatus> }`)
- Create: `crates/playtest-server/src/routes/mod.rs`, `health.rs`, `runs.rs`, `games.rs`, `registry.rs`, `reports.rs`
- Create: `crates/playtest-server/src/sse.rs` (`line_to_sse_frame(line: &str) -> SseFrame`, `subscribe(rid) -> impl Stream<Item = SseFrame>`)
- Create: `crates/playtest-server/src/runner.rs` (spawns a tokio task per run, constructs `BroadcastGameEventSink`, drives the `GameLoop`)
- Create: `crates/playtest-adapters/src/game_event_sink/broadcast.rs` (`BroadcastGameEventSink<I: GameEventSink>` — delegates to `I` then publishes to a `broadcast::Sender<String>`)
- Modify: `crates/playtest-cli/src/main.rs` + `commands/` (add `serve` subcommand that calls `playtest_server::run`)
- Test: `crates/playtest-server/tests/server_smoke.rs` (spawn server, POST run, assert game files on disk)
- Test: `crates/playtest-server/tests/sse_contract.rs` (spawn server, subscribe to SSE, assert frame sequence matches JSONL)

**Approach:**
- `AppState` holds a `DashMap<RunId, RunHandle>`. Creating a run spawns a tokio task that constructs the production adapters plus a `BroadcastGameEventSink`, runs the `GameLoop`, and drops its broadcaster when done.
- `BroadcastGameEventSink::emit` calls the inner sink (writes to JSONL), then calls `broadcast::Sender::send` on the published line. **`Sender::send` returns `Err(SendError)` when there are zero receivers**, which is the common case (the game starts before any SSE client subscribes). The adapter discards that error — it is not a `GameEventSinkError`. Only inner-sink errors propagate.
- **Reconnection / catch-up flow (the load-bearing detail):** `tokio::sync::broadcast` does *not* retain history by id. On `GET .../stream` with a `Last-Event-ID: N`, the server's stream handler: (1) acquires a read-side snapshot of current write position from the run's writer; (2) opens the JSONL file and reads tick `N+1..=current_write_position` into the stream; (3) *then* calls `broadcaster.subscribe()` and continues from live. Race handling: while the JSONL catch-up is running, new events keep appending to the file and to the broadcaster — the broadcaster tail buffers up to `channel_capacity` (1024) lines, so as long as catch-up finishes before 1024 new frames arrive, no events are lost. In practice this is trivially fast (catch-up is a file read; game events are sub-millisecond). If the broadcaster *did* overflow during catch-up, the client detects the gap via the `id:` jump on the first live frame and fetches the missing tail via `GET .../events?offset=...`.
- Fresh subscribers (no `Last-Event-ID`) with a still-live game: same pattern — catch-up from JSONL header to current tick, then subscribe. No live frames are missed.
- Completed-game subscribers: server sees `RunStatus::Completed`, streams the full JSONL as SSE frames, closes.
- Graceful shutdown: `tokio::signal::ctrl_c` cancels the run-supervisor task, awaits run-task completion with a 5s timeout, flushes writer, then exits. SSE streams close cleanly via broadcaster drop (no special frame; clients see connection close).
- Bind defaults to `127.0.0.1:7878`. `--bind 0.0.0.0:...` logs a `WARN` line.
- Run-spawn re-uses the CLI's `game_registry.rs` and `agent_registry.rs`. **`playtest-server` therefore depends on `playtest-cli` as a library, which transitively pulls in every game crate via the static registry.** That is the correct shape: the server and the CLI share one dispatch point. The previously-aspirational "no `crates/games/` dependency" was wrong; the invariant we actually want is that no *game-specific code* appears in `playtest-server` source, which is verifiable by grep.

**Execution note:** Start with a failing integration test for the live SSE stream (Unit 17's `sse_contract.rs`). The shape of that test dictates the broadcaster API.

**Patterns to follow:**
- axum 0.8, tower middleware, `tokio::sync::broadcast` (bounded at 1024; slow clients get `Lagged`).
- Errors map through `ApiError` from `playtest-api` — no ad-hoc error types in routes.
- All async; no `block_on` anywhere.

**Test scenarios:**
- Happy path: server starts, `GET /api/health` returns 200 + `api_version`
- Happy path: `POST /api/runs` with valid Cribbage config returns `{run_id}`; `GET /api/runs/:id` shows `Running` then `Completed`
- Happy path: client subscribes to `GET /api/runs/.../games/.../stream` mid-game, receives `header` + all subsequent events + `final`
- Happy path: live SSE frame sequence matches JSONL file byte-for-byte after transformation (Unit 16's `line_to_sse_frame` round-trip)
- Happy path: same run via server produces JSONL files identical to `playtest play` with the same seed (determinism under the server wrapper)
- Edge case: subscriber connects *after* the run completes — receives the full log from JSONL + closes, no live broadcast needed
- Edge case: subscriber reconnects with `Last-Event-ID: 42` — receives frames from tick 43 onward
- Edge case: slow client lags the broadcast channel — receives a `Lagged {skipped: N}` frame and continues from the latest tick (client expected to fetch the gap via `GET .../events?offset=...`)
- Error path: `POST /api/runs` with invalid game id → 400 + `ApiError { code: UnknownGame }`
- Error path: `POST /api/runs` with 5-agent Cribbage config → 400 (validation enforced at route boundary, not deep inside the engine)
- Integration: `SIGINT` during a run → `Shutdown` frame, JSONL file finalized on disk (no dangling half-written game)
- Integration: concurrent 10 runs × 10 games each, 10 SSE subscribers each, no deadlock, every game terminates, every SSE stream sees `final`

**Verification:**
- `playtest-server` depends on `playtest-api`, `playtest-core`, `playtest-adapters`, `playtest-log`, `playtest-metrics`, `playtest-cli`. Game crates are pulled in transitively via `playtest-cli`'s static registry (expected — same dispatch point as the CLI).
- No game-specific identifiers in `playtest-server/src/` sources: `grep -r 'cribbage\|shipwreck' crates/playtest-server/src/` returns nothing
- All routes appear in the OpenAPI dump (Unit 18) and match the wire contract table
- `cargo clippy -p playtest-server -- -D warnings` clean

---

- [x] **Unit 18: Wire-contract docs + OpenAPI dump + SvelteKit handoff**

**Goal:** Produce the artifacts the SvelteKit frontend repo needs to build against this server without reading Rust source.

**Requirements:** R8.5, R8.6

**Dependencies:** Units 16, 17

**Files:**
- Create: `docs/api-contract.md` (human-readable explanation of every endpoint, SSE frame, error code, and `api_version` policy)
- Create: `crates/playtest-server/src/schema.rs` (OpenAPI 3.1 spec built from the `schemars` schemas in `playtest-api`)
- Modify: `crates/playtest-cli/src/commands/` — add `playtest api-schema --out <path>` subcommand that writes the OpenAPI dump (the one-shot handoff tool)
- Create: `docs/openapi.json` (committed output of the above, regenerated manually when the wire contract changes)
- Test: `crates/playtest-server/tests/openapi_dump.rs` (asserts every route + type from `playtest-api` appears in the dump)

**Approach:**
- `docs/api-contract.md` carries hand-written prose: versioning policy (semver-major breaks are mandatory `api_version` bump; new optional fields are minor), reconnection semantics for SSE, error-code catalog, pagination rules, and the one example request/response per endpoint.
- `docs/openapi.json` is the machine-readable contract — generated from `schemars::JsonSchema` on `playtest-api` types, assembled with route descriptions from `playtest-server`. Regenerated manually (`playtest api-schema --out docs/openapi.json`) when the wire contract changes; no CI drift-check in this phase — the handoff is human-verified for now. If drift becomes a real pain point we can add the CI check later.
- SvelteKit repo consumes `docs/openapi.json` via its preferred codegen tool (e.g., `openapi-typescript`). This plan does not ship SvelteKit code but does verify that a vanilla `openapi-typescript` run on the dump produces compilable `.d.ts` output (one-time check noted in the contract doc).

**Patterns to follow:** Documentation lives in `docs/`; generated artifacts are committed so SvelteKit doesn't need to build this repo to consume it.

**Test scenarios:**
- Happy path: `playtest api-schema --out -` emits valid OpenAPI 3.1 JSON to stdout
- Happy path: every endpoint in the wire contract table appears in the dump with request + response schemas
- Integration: feeding `docs/openapi.json` into `openapi-typescript@latest` produces no errors — documented in `docs/api-contract.md` with a frontend-side snippet

**Verification:**
- `docs/api-contract.md` is the single source of truth for the frontend team; `docs/openapi.json` is the machine form; neither is in source-of-truth conflict with the other
- CI passes on a clean checkout

---

### `Game` trait extension (Unit 19)

- [x] **Unit 19: `Game::determinize` trait method + test-game + Cribbage impl**

**Goal:** Add the one new `Game` method ISMCTS will need, with its invariant documented and the existing Cribbage + in-repo test impls brought along. Lands *before* ShipWreck so ShipWreck's `Game` impl includes determinize from day one, not as a retrofit.

**Requirements:** R2.3 (enabling), R0.14 (preserves trait-stability claim)

**Dependencies:** Phase 0 + Phase 1 shipped (no in-plan prerequisites)

**Files:**
- Modify: `crates/playtest-core/src/game.rs` — add `fn determinize(&self, state: &Self::State, observer: PlayerId, rng: &mut dyn playtest_ports::Rng) -> Self::State;` with doc comment stating the invariant
- Modify: `crates/playtest-core/tests/game_loop_shape.rs` — the two existing test `Game` impls (`TallyGame` at line 37, `AlreadyDone` at line 312) each need a trivial `determinize` — both games have no hidden info, so it clones state
- Create: `crates/games/cribbage/src/determinize.rs` — Cribbage determinization: resample opponent's hand + crib from cards the observer hasn't seen (played pegging cards + own hand + cut card + already-scored show cards are all observer-known)
- Modify: `crates/games/cribbage/src/rules.rs` — wire `determinize` into `impl Game for CribbageGame`
- Test: `crates/games/cribbage/tests/determinize.rs` — invariant property test (1000 random states × observer seats)

**Approach:**
- The invariant — `public_view(determinize(s, p, rng), p) == public_view(s, p)` — is the only correctness property. Every game must satisfy it; callers (ISMCTS) trust it.
- No default impl. Forcing explicit implementation makes the "which information is hidden?" question unavoidable at `Game`-impl time — the cheap trap of "default returns state.clone()" would silently break ISMCTS for any game with hidden info.
- Cribbage determinization algorithm: compute `unknown_cards = full_deck − observer.hand − played_stack − cut_card − publicly_shown_cards`; then shuffle `unknown_cards` and deal the right count into opponent's hand and crib.
- ShipWreck's determinize is *not* in this unit — it lives in `crates/games/shipwreck/src/determinize.rs`, wired up as part of Unit 22's `impl Game for ShipWreckGame`.

**Execution note:** Write the invariant property test *first*, against a mocked Cribbage mid-game state. The test forces the algorithm to be correct rather than merely type-check.

**Patterns to follow:** `Rng` port is already `&mut dyn Rng`-friendly; determinize takes the same by-mut reference the rest of the engine uses.

**Test scenarios:**
- Happy path (TallyGame): `determinize(s, 0, rng) == s` (no hidden info → identity)
- Happy path (Cribbage): mid-pegging state where player 0 has seen `[AS, 5H]` played — `determinize(s, 0, rng)` returns a state where those two cards are in the played stack exactly as in `s`, and opponent's hand is some 4-card draw from the rest of the unseen pile
- Invariant test: 1000 random Cribbage states × `determinize` → observer's `public_view` is byte-equal before and after
- Edge case: at game start, before any cards are revealed, `determinize` is approximately a full reshuffle of unknown cards
- Edge case: at game end, every card is known → `determinize` returns something state-equivalent
- Error path: observer id out of range → panics deterministically (engine bug; not observable at the agent surface)

**Verification:**
- `crates/playtest-core/src/game.rs` has exactly one new method
- Existing `cargo test --workspace` stays green (TallyGame + AlreadyDone + Cribbage all compile + pass)
- Property test succeeds on 1000 seeds
- `cargo tree -p playtest-core` unchanged (no new deps)

---

### Multi-game proof: ShipWreck (Units 20–23)

- [x] **Unit 20: ShipWreck primitives — cards, static pools, raft structure**

**Goal:** The atomic building blocks for ShipWreck. Implemented first and exhaustively tested, because every other ShipWreck unit depends on these.

**Requirements:** R9.2, R0.14

**Dependencies:** Unit 7 (Cribbage primitives — precedent for this file layout)

**Files:**
- Create: `crates/games/shipwreck/Cargo.toml`
- Create: `crates/games/shipwreck/src/lib.rs`
- Create: `crates/games/shipwreck/src/card.rs` (`Card` enum with variants: `Player`, `BaseRaft`, `RaftExtension`, `Equipment`, `Item`, `Event`; plus per-variant payload types like `PlayerCard { name, rescue_points, food_cost, skill }`, `EquipmentCard { name, cost: ResourceCost, effect }`)
- Create: `crates/games/shipwreck/src/pool.rs` (static `fn all_player_cards() -> Vec<PlayerCard>`, `fn all_equipment() -> Vec<EquipmentCard>`, wreckage item pool (30 each of 5 resources), event pool (shark, typhoon, flying fish counts per `docs/shipwreck.md`))
- Create: `crates/games/shipwreck/src/raft.rs` (`Raft { base_left, base_right, extensions: Vec<RaftExtension>, upgrades: HashMap<SlotId, Equipment> }`, methods: `extend(ext)`, `build_upgrade(slot, eq)`, `length() -> usize`, `invention_count() -> usize`)
- Create: `crates/games/shipwreck/src/resource.rs` (`enum Resource { Plastic, Wood, Rope, Cloth, Wire }`, `struct ResourceCost([u8; 5])` with `can_pay(&inventory)`, `pay(&mut inventory)`)
- Test: `crates/games/shipwreck/tests/primitives.rs`

**Approach:**
- Card pool is defined as `const`-buildable data from `docs/shipwreck.md`. Counts: 7 player cards, 40 raft extension cards, 5 equipment types (with `docs/shipwreck.md` quantities 2/1/3/2/5), 30×5 = 150 item cards, 3 event card types (quantities TBD during implementation — document defaults as 6 sharks, 2 typhoons, 10 flying fish pending playtesting).
- `Raft` uses an `Vec<RaftExtension>` between `base_left` and `base_right` — extending splits the insertion point. Upgrade slots are indexed by position (base-left, base-right, each extension).
- `ResourceCost::can_pay` checks inventory without mutation; `pay` returns `Err(InsufficientResources)` rather than panicking.
- `Card` derives `Serialize` so it can appear in `Event`s and the JSONL log.

**Patterns to follow:** Precedent is `crates/games/cribbage/src/card.rs` and `deck.rs` — small, pure, exhaustively tested primitives. No references to `playtest-core` yet.

**Test scenarios:**
- Happy path: `all_player_cards()` returns exactly 7 unique named cards matching `docs/shipwreck.md`
- Happy path: `all_equipment()` returns 5 equipment definitions with costs matching the spec
- Happy path: `Raft::extend` increases `length()` by 1 and preserves base-left / base-right at the ends
- Happy path: `ResourceCost { plastic: 1, wood: 1 }.can_pay(inventory)` is true when inventory has both, false otherwise
- Edge case: extending a raft between `base_left` and `base_left` (i.e., with zero existing extensions) inserts one extension between them
- Edge case: `ResourceCost::pay` on insufficient inventory returns error, leaves inventory untouched
- Edge case: building an upgrade on a slot that already has one returns error
- Integration: full deck (player + raft ext + equipment + item + event) serializes to JSON and round-trips without loss

**Verification:**
- `cargo test -p playtest-shipwreck --test primitives` all green
- `playtest-shipwreck` depends only on `serde`, `thiserror`, nothing from `playtest-core` yet

---

- [x] **Unit 21: ShipWreck state, setup, action, and event types**

**Goal:** Define the full state machine — `GameState`, per-player state, `Action` enum, `Event` enum, setup phase. Does not yet implement turn-taking or action resolution (those land in Unit 22).

**Requirements:** R9.1, R9.2

**Dependencies:** Unit 20

**Files:**
- Create: `crates/games/shipwreck/src/state.rs` (`GameState { config, players: Vec<PlayerState>, wreckage_deck: Vec<Card>, face_up_pools: Vec<Vec<Card>>, current_player, phase, event_resolution_stack: Vec<PendingEvent> }`, `PlayerState { raft, hand, played_players: Vec<PlayerCard>, food_counter, inventory: [u8; 5] }`)
- Create: `crates/games/shipwreck/src/phase.rs` (`enum Phase { Setup, Play, ResolvingEvent, Finished }`)
- Create: `crates/games/shipwreck/src/action.rs` (`enum Action { ExtendRaft { insert_after }, PlacePlayerCard { card, slot }, PickWreckage { from_pool, card }, PlayEventCard { card, target }, BuildEquipment { equipment, slot }, EndTurn, ResolveEvent(EventResolution) }`)
- Create: `crates/games/shipwreck/src/event.rs` (`enum Event { DealPlayerCard, DealWreckageHand, DealWreckageFaceUp, PickedWreckage, PlacedPlayerCard, ExtendedRaft, BuiltEquipment, EventCardPlayed, EventResolved, ResourceSpent, FoodConsumed, EndGame }` — each with appropriate payloads)
- Create: `crates/games/shipwreck/src/config.rs` (`struct ShipWreckConfig { num_players: u8 }` with `Default::default() == 2`)
- Test: `crates/games/shipwreck/tests/setup.rs` (verifies initial state shape per player count)

**Approach:**
- Setup flow in `initial_state` (implemented in Unit 22's `rules.rs`): shuffle player cards → deal one to each player → shuffle remaining player cards into wreckage deck → shuffle wreckage deck → deal 6 wreckage cards per player's hand → distribute remaining face-up, round-robin, one per player per pass, until deck is empty.
- Per `docs/shipwreck.md`: the face-up pool is *per player*, not a shared pool. Players on the end may have fewer face-up cards if the deck runs uneven.
- `ResolvingEvent` phase: when a player plays an event card (shark/typhoon), normal turn is paused while resolution happens. `event_resolution_stack` holds pending resolution steps (each player choosing what to lose to a typhoon, etc.).
- `Event::EndGame` carries `{ winner: Option<PlayerId>, tiebreaker_used: Option<TieBreaker>, final_scores: Vec<(PlayerId, RescuePoints)> }`.

**Patterns to follow:** Precedent is `crates/games/cribbage/src/state.rs` — state is passive data; rules live in `rules.rs`. Event payloads are small structs, not free-form.

**Test scenarios:**
- Happy path: `ShipWreckConfig::default().num_players == 2`
- Happy path: with `num_players = 2`, initial state has 2 `PlayerState`s, each with 1 player card in hand (unplayed) and 6 wreckage cards in hand
- Happy path: face-up pools sum to 150 (items) + (40 - extensions_in_hand?) + remaining player cards + event cards — matches `(total wreckage) - (2 * 6)`
- Happy path: `Action::PickWreckage { from_pool: own, card: Item(Wood) }` serializes correctly
- Edge case: `num_players = 5` → `ShipWreckConfig::new(5)` returns `Err(InvalidPlayerCount)` (spec is 2–4)
- Integration: `GameState` and every `Action`/`Event` variant round-trip through JSON

**Verification:**
- State shape accommodates 2, 3, and 4 player configurations
- All Action variants are covered in the legal-actions logic (to be implemented in Unit 22)

---

- [x] **Unit 22: ShipWreck turn flow — legal actions, apply_action, apply_event, determinize**

**Goal:** The core `Game` trait impl minus event-card resolution. A random-action game terminates and produces a sensible log. ShipWreck's `determinize` (added to `Game` in Unit 19) is implemented here as part of the `impl Game for ShipWreckGame` block — ShipWreck's hidden-info shape (opponent hands + deck order) is shallow enough to co-locate with the rest of the rules. This unit is where the `Game` trait is stress-tested for structural differences vs. Cribbage — if anything needs to change in `playtest-core`, it surfaces here.

**Requirements:** R9.3, R0.14, R0.4, R2.3 (via determinize)

**Dependencies:** Units 19, 21

**Files:**
- Create: `crates/games/shipwreck/src/rules.rs` (`struct ShipWreckGame; impl Game for ShipWreckGame { ... }` — including `determinize`)
- Create: `crates/games/shipwreck/src/turns.rs` (private helpers for `legal_actions`)
- Create: `crates/games/shipwreck/src/determinize.rs` (gather unknown cards from the observer's perspective, partition randomly into opponent hands + deck tail)
- Modify: `crates/games/shipwreck/src/lib.rs` — re-export `ShipWreckGame`
- Modify: `crates/playtest-core/src/game.rs` — **only if** Unit 22 surfaces a genuine abstraction gap beyond what Unit 19 already added. The default expectation is no further change. Any proposed change must come with a one-paragraph rationale in the commit message citing the ShipWreck situation that needs it.
- Test: `crates/games/shipwreck/tests/turn_flow.rs`
- Test: `crates/games/shipwreck/tests/determinize.rs` — invariant property test (1000 random states × observer seats)

**Approach:**
- `next_actor` returns `Actor::Player(current_player)` during normal play; returns `Actor::Chance` during setup (initial deals) and possibly when shuffling.
- `legal_actions` for the current player enumerates: possible `ExtendRaft` insertion points; possible `PlacePlayerCard` combinations (player cards in hand × open slots); possible `PickWreckage` picks from own face-up pool (modified by Telescope to adjacent pools); possible `PlayEventCard` (event cards in hand × legal targets); possible `BuildEquipment` (equipment up for construction ∩ affordable ∩ with an open slot); and always `EndTurn`.
- `apply_action` validates and emits a sequence of events per action. Examples: `PickWreckage { from_pool, card }` → `[PickedWreckage { player, pool, card }]`; `BuildEquipment` → `[ResourceSpent ×N, BuiltEquipment { player, slot, eq }]`.
- `apply_event` is pure mutation; no validation. Mirrors Cribbage's pattern.
- `game_over` returns `Some(result)` when `wreckage_deck.is_empty() && face_up_pools.iter().all(Vec::is_empty) && event_resolution_stack.is_empty()`.
- End-of-turn food consumption: at end of each player's turn, each played player card consumes its `food_cost` from the player's inventory. If inventory can't cover, card is *discarded* (implementation detail: "`starve`" event). Spec gap — flagged in `docs/shipwreck.md` review: food comes from flying-fish events and from rain-catcher upgrades, not directly from wreckage items. This unit adopts a minimal interpretation (food counter, starvation discards the card) and notes the design question in `docs/shipwreck.md` follow-ups.

**Execution note:** Start with a single integration test — "2-player random-vs-random ShipWreck game reaches a terminal state with a valid winner" — and build downward from that failure. Trait-shape issues surface here, not in unit tests.

**Patterns to follow:** Precedent is `crates/games/cribbage/src/rules.rs`. `legal_actions` is the method whose length in lines is the best proxy for how well the abstraction fits — if it's >300 lines, something is wrong.

**Test scenarios:**
- Happy path: 2-player random-vs-random game terminates in <500 turns; produces a valid `GameResult`
- Happy path: 3-player and 4-player random-vs-random games terminate
- Happy path: `apply_action(BuildEquipment)` with exact resources succeeds; inventory decremented; `BuiltEquipment` event emitted
- Happy path: `apply_action(ExtendRaft { insert_after: base_left })` adds an extension at the correct position
- Edge case: `apply_action(PickWreckage { from_pool: neighbor })` is illegal without a Telescope; legal with
- Edge case: picking the last wreckage card triggers `game_over == Some(_)` after that turn's event-resolution finishes
- Edge case: starvation — a placed player card whose food cost can't be paid is removed with a `FoodConsumed { starved: true }` event (or spec-refined equivalent)
- Error path: `apply_action(BuildEquipment { slot: occupied_slot })` returns `GameError::IllegalAction`
- Error path: `apply_action(PlayEventCard { target: nonexistent })` returns `GameError::IllegalAction`
- Integration: 1000 random-vs-random games all terminate; zero panics; every log replayable via `replay()` in `playtest-log`

**Verification:**
- `ShipWreckGame` implements `Game` with no additions to the trait (or, if additions are made, with a documented rationale)
- Random-vs-Random produces a winner in under 1s per game on the reference machine
- `cargo tree -p playtest-core` still shows no dependency on any game crate

---

- [x] **Unit 23: ShipWreck event-card resolution — shark, typhoon, flying fish**

**Goal:** Full event-card semantics. Event cards are the most complex ShipWreck mechanic (targeting, multi-player decisions) and are the true stress test of the `apply_action`/`apply_event` split.

**Requirements:** R9.4

**Dependencies:** Unit 22

**Files:**
- Create: `crates/games/shipwreck/src/events/mod.rs`
- Create: `crates/games/shipwreck/src/events/shark.rs` (targeted destruction of one upgrade or extension on a chosen player's raft)
- Create: `crates/games/shipwreck/src/events/typhoon.rs` (all players lose one upgrade or extension, chosen by each)
- Create: `crates/games/shipwreck/src/events/flying_fish.rs` (one-turn food substitute)
- Modify: `crates/games/shipwreck/src/rules.rs` — wire event resolution into the phase state machine
- Test: `crates/games/shipwreck/tests/event_cards.rs`

**Approach:**
- Shark: `PlayEventCard { card: Shark, target: { player, target_ref: UpgradeSlot | ExtensionIdx } }`. Resolution: `apply_action` validates target exists; emits `EventCardPlayed` + `EventResolved { destroyed }`. Steel-cordage upgrades on the target player's raft defend and destroy instead.
- Typhoon: multi-step resolution. `PlayEventCard { card: Typhoon }` enters `Phase::ResolvingEvent` with a `TyphoonResolution { remaining_players: [...] }`. Each remaining player's next `Action` must be a `ResolveEvent(TyphoonLose { target_ref })` — or `ResolveEvent(TyphoonPass)` when that player owns nothing losable (base rafts are protected per spec). `legal_actions` during resolution always returns a non-empty set (at minimum `[TyphoonPass]`) so `GameLoop` never stalls. Engine drives this by changing `next_actor` to each player in turn. When all have resolved, return to `Phase::Play`.
- Flying Fish: `PlayEventCard { card: FlyingFish }`. Resolution: emits `FoodGranted { player: current, amount: 1 }`. No target; resolves immediately.

**Execution note:** Write the typhoon test first — multi-player resolution is the mechanic most likely to force a `Game` trait change. If `next_actor` can't express "ask each remaining player in turn," that's the abstraction gap to discover.

**Patterns to follow:** Event resolution uses the `event_resolution_stack` introduced in Unit 20. Each event card has its own module; the dispatcher in `rules.rs` is a match on `PendingEvent` variants.

**Test scenarios:**
- Happy path (shark): 2-player, opponent has an equipment + an extension; shark target = extension → extension destroyed, equipment on that extension also lost
- Happy path (shark): opponent has steel-cordage upgrade → shark defends, destroys the steel-cordage instead; `EventResolved { defended: true }` event
- Happy path (typhoon): 3-player game; each player's next action is a `ResolveEvent(TyphoonLose)`; `next_actor` cycles through all three before returning to normal turn order
- Happy path (flying fish): current player gains 1 food this turn; expires at end of turn
- Edge case (typhoon): a player with no upgrades and no extensions has exactly one legal action: `ResolveEvent(TyphoonPass)` (per spec, base rafts are never forfeited). `legal_actions` is never empty during event resolution.
- Edge case: playing an event card that targets the player themselves (if not explicitly forbidden in spec) — flag and default to illegal
- Error path: `PlayEventCard { card: Shark, target: { player: self } }` on spec-disallowed targeting → illegal
- Error path: `ResolveEvent(TyphoonLose { target_ref: invalid })` → illegal
- Integration: 1000 random-vs-random games that each play at least one event card complete correctly; replay reproduces the event-resolution ordering

**Verification:**
- Event cards represent 100% of spec in `docs/shipwreck.md`
- Multi-player event resolution requires zero changes to `Actor` or `next_actor`'s contract (or, if changes are required, they are the multi-game lessons this plan exists to surface)

---

- [x] **Unit 24: ShipWreck scoring, metrics, CLI integration, soak validation**

**Goal:** Full integration — ShipWreck's `MetricRegistry` impl, CLI registry entry, end-to-end `playtest play --game shipwreck` working, and 10K-game random-vs-random soak run completes cleanly.

**Requirements:** R9.5, R9.6, R9.7, R9.8

**Dependencies:** Units 22, 23

**Files:**
- Create: `crates/games/shipwreck/src/scoring.rs` (`fn score_player(p: &PlayerState) -> RescuePoints`, `fn determine_winner(players: &[PlayerState]) -> GameResult` using tie-breaker chain)
- Create: `crates/games/shipwreck/src/metrics.rs` (`ShipWreckMetrics` impl of `MetricRegistry<ShipWreckGame>`)
- Create: `crates/games/shipwreck/src/metrics/game_shape.rs` (metrics like `game_length_turns`, `winner_raft_length`, `winner_equipment_count`, `tie_breaker_used`, `event_cards_played_per_game`)
- Create: `crates/games/shipwreck/src/metrics/player.rs` (`avg_rescue_points_player_N`, `avg_raft_length_player_N`, `food_starvation_events_player_N`)
- Modify: `crates/playtest-cli/src/game_registry.rs` — add `"shipwreck" => ShipWreckGame`
- Modify: `crates/playtest-cli/src/commands/report.rs` — ensure ShipWreck metrics render in the report
- Test: `crates/games/shipwreck/tests/soak_1k.rs` — 1000 random-vs-random 2-player games, zero panics, all valid winners

**Approach:**
- `ShipWreckMetrics::extract` walks the event stream once per game; reuses the `CribbageMetrics` pattern.
- At least 8 ShipWreck-specific metric definitions: `game_length_turns`, `winner_raft_length`, `winner_equipment_count`, `tie_breaker_used`, `event_cards_played`, `avg_rescue_points_player_N`, `avg_raft_length_player_N`, `food_starvation_events_player_N`.
- CLI registration: one line in `game_registry.rs`. No CLI code changes beyond the registry — if more are needed, they indicate a Cribbage-specific leak to fix.
- Report smoke test at 1K games: markdown output has a "Shipwreck: game shape" and "Shipwreck: per-player" section. Generalizes the report template.

**Test scenarios:**
- Happy path: `playtest play --game shipwreck --agents random,random --games 10 --seed 42 --out games/` exits 0 and produces 10 JSONL files
- Happy path: `playtest report --games games/ --out report.md` includes ShipWreck-specific sections
- Happy path: `ShipWreckMetrics::extract` on 1000 logs produces metric counts with zero extraction errors
- Happy path: scoring — a 4-player game where two players tie on rescue points falls through to raft-length tie-breaker; `tie_breaker_used == "raft_length"` appears as a metric tag
- Edge case: 2-player game with zero event cards played still terminates; `event_cards_played == 0` is recorded
- Edge case: all four players have equal rescue points, equal raft length, equal invention count → `GameResult { winner: None, reason: Tie }`
- Performance: 1000-game soak completes in under 120s (much laxer than Cribbage; ShipWreck has more per-turn decisions)
- Integration: a Cribbage run and a ShipWreck run in the same output dir → `playtest report` handles both in one pass

**Verification:**
- `ShipWreckMetrics` lives entirely in `crates/games/shipwreck/` — no edits to `playtest-metrics`
- `cargo tree -p playtest-cli` shows both game crates; CLI now lists both in `--help` output for `--game`
- `docs/BENCHMARKS.md` gets a row for ShipWreck
- `README.md` gets a "Games" section listing Cribbage and ShipWreck

---

### Phase 2 — Heuristic agents + ISMCTS (Units 25–27)

- [x] **Unit 25: Evaluation functions, `GreedyAgent`, `HeuristicAgent`**

**Goal:** Game-provided evaluation functions and two agents that use them. `HeuristicAgent` for Cribbage and ShipWreck hits the R2.2 quality bar (beats random >90%).

**Requirements:** R2.1, R2.2

**Dependencies:** Units 9 (Cribbage rules), 24 (ShipWreck metrics + rules)

**Files:**
- Create: `crates/playtest-agents/src/eval.rs` (`pub type EvalFn<G> = fn(&<G as Game>::PublicView, PlayerId) -> f64;`)
- Create: `crates/playtest-agents/src/greedy.rs` (`struct GreedyAgent<G: Game, E> { eval: E, ... }`; simulates each legal action one ply, scores, picks max; ties broken by index)
- Create: `crates/playtest-agents/src/heuristic.rs` (`struct HeuristicAgent<G: Game, E>` — a greedy variant with optional softmax over top-K to add stochasticity for more varied metric coverage)
- Create: `crates/games/cribbage/src/heuristic.rs` (`pub fn cribbage_eval(view: &PublicView, player: PlayerId) -> f64` — weighs: own pegging score, own show-hand score on current known cards, crib counts *for dealer* / *against non-dealer*, board-position pressure)
- Create: `crates/games/shipwreck/src/heuristic.rs` (`pub fn shipwreck_eval(view: &PublicView, player: PlayerId) -> f64` — weighs: rescue points held, raft length, invention count, resource inventory diversity, opponent vulnerability)
- Modify: `crates/playtest-cli/src/agent_registry.rs` — add `"greedy-cribbage"`, `"heuristic-cribbage"`, `"greedy-shipwreck"`, `"heuristic-shipwreck"`
- Test: `crates/playtest-agents/tests/greedy_agent.rs`
- Test: `crates/games/cribbage/tests/heuristic_beats_random.rs` — 10K games, Heuristic vs Random, asserts win rate >90%
- Test: `crates/games/shipwreck/tests/heuristic_beats_random.rs` — 10K games, Heuristic vs Random, asserts win rate >90%

**Approach:**
- Greedy needs to simulate legal actions without mutating state: it uses `Game::apply_action` + `Game::apply_event` on cloned state, then `eval` on the resulting public view. No lookahead beyond one action.
- Evaluation functions are plain `fn` pointers rather than trait objects — zero dispatch cost.
- For games where one "action" is the agent's full turn (like Cribbage's DiscardToCrib where the agent commits two cards simultaneously), greedy's one-ply is already meaningful. For ShipWreck, one action per call means greedy is genuinely one decision deep.
- Heuristic agent's softmax variant reduces exact-replay collisions and increases metric coverage. Temperature as a tunable parameter; default 0.5.
- `eval` is expected to return higher = better for `player`. Greedy's argmax works without sign convention gymnastics.

**Execution note:** For each game's heuristic, start from a failing "heuristic beats random >90%" integration test. That's the only quality signal that matters; individual eval-function unit tests are far less informative.

**Patterns to follow:** Precedent: `ScriptedAgent` in Unit 5 — priority function shape generalizes naturally.

**Test scenarios:**
- Happy path (Greedy): a test game with a dominant "score +1" action vs. "score 0" action → GreedyAgent picks the +1 action every time
- Happy path (Greedy on Cribbage): `heuristic-cribbage` agent vs `random` over 10K games, win rate ≥ 90%
- Happy path (Greedy on ShipWreck): `heuristic-shipwreck` vs `random` over 10K games, win rate ≥ 90%
- Edge case (Greedy): tie on eval score — breaks deterministically by lowest legal-action index
- Happy path (softmax HeuristicAgent): temperature = 0 is pure greedy; temperature → ∞ approaches random; both extremes test
- Edge case: a game state where `eval` returns `NaN` (bug in the eval fn) → agent returns an error rather than picking junk
- Integration: `HeuristicAgent` plays through the SSE stream (Unit 17) just like Random — no agent-specific code in the server

**Verification:**
- R2.2 exit criterion met: both games' heuristics beat random over 10K games
- `playtest-agents` crate has no dependency on `crates/games/cribbage` or `crates/games/shipwreck` (eval fns import the other direction)
- New agent kinds appear in `playtest --help`

---

- [x] **Unit 26: `ISMCTSAgent` — generic Information-Set MCTS with determinization**

**Goal:** A single generic ISMCTS implementation that works for any `Game` implementing `determinize`. Plugs into Cribbage and ShipWreck without any game-specific code.

**Requirements:** R2.3

**Dependencies:** Units 19 (trait), 22 (ShipWreck determinize impl), 25 (Greedy baseline)

**Files:**
- Create: `crates/playtest-agents/src/ismcts/mod.rs` (`ISMCTSAgent<G: Game, Rf: RolloutFn<G>>`, `Config { iterations: u32, exploration_c: f64, rollout_depth: u32 }`)
- Create: `crates/playtest-agents/src/ismcts/node.rs` (tree node — action index, visit count, total value, child index list)
- Create: `crates/playtest-agents/src/ismcts/ucb.rs` (UCB1 selection: `value / visits + c * sqrt(ln(parent_visits) / visits)`)
- Create: `crates/playtest-agents/src/ismcts/rollout.rs` (`trait RolloutFn<G>`; default: random rollout to terminal state with fallback max-depth cutoff where `eval` estimates the outcome)
- Modify: `crates/playtest-cli/src/agent_registry.rs` — add `"ismcts-cribbage"`, `"ismcts-shipwreck"` with sensible default configs
- Test: `crates/playtest-agents/tests/ismcts_agent.rs` — correctness (picks a dominating move) and convergence (more iterations → strictly non-worse play)

**Approach:**
- SO-ISMCTS (single-observer): `self.player` is the observer. Each iteration determinizes once at the root, then descends the tree with UCB1, expands at leaves, runs a rollout to terminal (or `rollout_depth` cutoff with eval fallback), backpropagates the reward (1.0 for win, 0.0 for loss, 0.5 for tie).
- Tree is rebuilt each decision call — no tree reuse across turns in this iteration. If quality is borderline, revisit.
- Reward is derived from `GameResult::winner`. For terminal states hit inside the rollout, we have `game_over`; for cutoff, we fall back to the game's eval function (Unit 25's `EvalFn<G>`) normalized to [0, 1].
- Budget: `iterations` (default 1000) or `wall_clock_ms` (default 500ms) — whichever binds first.
- Config is exposed in `agent_registry.rs` via arguments parseable from the agent-spec string: `"ismcts-cribbage:iter=2000,c=1.4"`.

**Execution note:** Write a "convergence" test first: the same ISMCTSAgent with `iterations=2000` beats itself at `iterations=500` over 1K games. If it doesn't, there's a bug in selection, expansion, rollout, or backprop — debug there before tuning eval weights.

**Patterns to follow:** The UCB1 formula and tree structure is textbook (Cowling et al. 2012); the craft is in the integration with `determinize` and the rollout policy. One module per concern; `mod.rs` is the public API.

**Test scenarios:**
- Happy path: on a game state with a clearly dominant action, ISMCTS at `iterations=1000` picks it every time
- Happy path: convergence — `iter=2000` beats `iter=500` at a statistically significant rate over 1K games
- Happy path (R2.3 exit): Cribbage — `ismcts-cribbage:iter=2000` vs `heuristic-cribbage` over 10K games, win rate ≥ 65%
- Happy path (R2.3 exit): ShipWreck — `ismcts-shipwreck:iter=2000` vs `heuristic-shipwreck` over 10K games, win rate ≥ 65%
- Edge case: a game state where legal_actions has one element → ISMCTS returns 0 without iterating
- Edge case: rollout hits `rollout_depth` cutoff → falls back to eval; test that eval normalization is in [0, 1]
- Error path: `determinize` panics (bug) → ISMCTS surfaces it through `AgentError` with game context
- Integration: ISMCTS vs. ISMCTS produces varied games across different seeds (non-degenerate — if both sides pick the same move every game, determinization or exploration is broken)

**Verification:**
- R2.3 exit criteria met for both games
- `ISMCTSAgent` has zero game-specific code; lives in `playtest-agents` only
- No changes to `playtest-core` beyond the Unit 19 `determinize` addition

---

- [x] **Unit 27: `playtest matchup` subcommand + Phase 2 exit validation**

**Goal:** Operator-facing tooling to produce matchup matrices, plus the benchmark run that validates R2.4.

**Requirements:** R2.4, R2.5

**Dependencies:** Units 25, 26

**Files:**
- Create: `crates/playtest-cli/src/commands/matchup.rs` (`playtest matchup --game cribbage --agents random,greedy,heuristic,ismcts --games-per-pair 10000 --out matrix.md`)
- Create: `crates/playtest-metrics/src/matchup.rs` (composes matchup matrix from ingested DB: rows × columns × cells-with-win-rate)
- Modify: `crates/playtest-metrics/src/reporter.rs` — add `render_matchup_matrix(entries: &[MatchupCell]) -> String`
- Modify: `crates/playtest-cli/src/main.rs` — wire `matchup` subcommand
- Create: `crates/playtest-cli/tests/matchup_smoke.rs`
- Create: `docs/BENCHMARKS.md` rows for R2.2, R2.3, R2.4

**Approach:**
- Matrix is computed by running N games per pair (agent_a, agent_b), collecting winner counts, building a table `rows = agents, cols = agents, cells = P(row_wins)`.
- For symmetry (agent_a vs agent_b vs. vice versa) we run both permutations and average — position effects in Cribbage (dealer vs. non-dealer) show up as systematic bias otherwise.
- **Automated tests run with N=100 games-per-pair** (smoke-level, ~seconds). **R2.4's 10K-per-pair benchmark is a manual one-shot documented in `docs/BENCHMARKS.md`**, not a CI test. For a 5-agent × 2-game pool at 10K per ordered pair: 5 × 5 × 10000 × 2 = 500K games per game, 1M total — the target is <30 minutes on a laptop. With rayon parallelism and Phase 0's per-game speed (100-300 games/sec/core × 8 cores ≈ 2400 games/sec for Random-vs-Random), the Random/Greedy/Heuristic cells finish in minutes. ISMCTS-involving cells are much slower (deep rollouts) — if they dominate total time, add **adaptive budget**: stop a pair early once the Wilson-score confidence interval on the win rate is narrower than ±2%. Adaptive budget implementation is in Unit 27 if a straight 10K/pair miss R2.4.
- Output is a markdown table: agents on both axes, win rates in cells. Raw rates only — significance testing / confidence intervals are Phase 6 (compare/counterfactual) scope.

**Test scenarios:**
- Happy path: matchup on Cribbage with `random,greedy-cribbage,heuristic-cribbage,ismcts-cribbage`, 100 games-per-pair — markdown matrix generated, diagonal is ~50% (self-play), random's row sums are lowest, ISMCTS's row sums are highest
- Happy path: matchup on ShipWreck with analogous agents → same shape
- Happy path (R2.4): 20-agent × 10K-games pool matchup completes in <30 min on reference laptop
- Edge case: matchup with a single agent → 1×1 matrix showing ~50% self-play win rate
- Edge case: one of the pair panics in the middle of its run → matrix cell notes "N/A (partial results: K games)"
- Integration: matchup output + `playtest report` output copy-paste into the same markdown document without clashing heading styles

**Verification:**
- R2.2, R2.3, R2.4 exit criteria all pass and are recorded in `docs/BENCHMARKS.md` with reference-machine context
- `playtest matchup --help` is self-explanatory
- Matchup matrix for both Cribbage and ShipWreck committed to `docs/benchmarks/` as reference output

## System-Wide Impact

- **Interaction graph.** The server introduces one new integration point: `BroadcastGameEventSink` tees into `tokio::sync::broadcast`. The `GameLoop` is untouched. Phase 2 agents compose through `Agent<G>` — same integration point as Phase 0.
- **Error propagation.** Server surfaces engine errors through `ApiError` with a run-id attached. SSE streams carry a terminal `error` frame if the run fails mid-game. ISMCTS wraps rollout errors in `AgentError::EvaluationFailed { game_context }`.
- **State lifecycle risks.** (1) Server crash mid-run — in-flight JSONL may be half-written; the existing `finish()` contract means after-crash logs are distinguishable (no `Final` line) and the replay path rejects them cleanly. (2) SSE client disconnects leave broadcaster receivers alive until the run ends; bound the channel at 1024 and `drop` on client disconnect. (3) ISMCTS tree is transient per decision — no cross-turn state hazard yet.
- **API surface parity.** Every operation available from the CLI is available from the server (`play`, `replay`, `report`, `matchup`). Every game-registered agent and metric is accessible in both surfaces.
- **Integration coverage.** Unit 17 (server smoke), Unit 22 (ShipWreck 1K random self-play + determinize property test), Unit 24 (ShipWreck soak), Unit 25 (heuristic beats random 10K), Unit 26 (ISMCTS beats heuristic 10K), Unit 27 (full matchup matrix) are the cross-crate checks.
- **Unchanged invariants.** The invariants Phase 0 introduced — determinism-per-seed, no direct RNG/clock/filesystem access outside the port crate, agents never mutate game state, games never depend on the harness's choice of storage — all remain. This plan adds one new invariant, stated in Key Technical Decisions: *`public_view(determinize(s, p, rng), p) == public_view(s, p)`*. Property-tested in Unit 19 for Cribbage, in Unit 22 for ShipWreck.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| ShipWreck forces a `Game` trait change that ripples through all agents | Unit 22 explicitly calls this out and requires any proposed change to come with a written rationale. Multi-player event resolution (Unit 23, typhoon) is the highest-risk spot; writing the typhoon test first forces the question early. Unit 19's `determinize` addition is the *planned* trait change; anything else counts as a genuine surprise. |
| `Actor::Chance` + `Actor::Player(id)` doesn't express ShipWreck's multi-player resolution (e.g., "next action is player 2's typhoon choice, even though it's player 0's turn") | `next_actor` already switches per-state; the typhoon implementation just flips `current_player` to the next resolver during `ResolvingEvent`, and `legal_actions` guarantees non-empty via the `TyphoonPass` variant (Unit 23). No trait change anticipated. If this is wrong, catch it in Unit 23. |
| ISMCTS fails to meet the 65%-over-heuristic bar because rollout variance is too high | Budget is 500ms or 1000 iterations per decision. Mitigations available: (a) increase budget; (b) add eval-guided rollout instead of random; (c) add progressive history heuristic; (d) tree reuse across turns. Unit 26's convergence test catches this early. |
| SSE broadcaster backpressure drops frames to slow clients, breaking the "every event visible" expectation | Broadcast channel bounded at 1024 lines (≈5 Cribbage games' worth of events). Slow clients receive a `lagged` frame and must fetch the gap via the paginated event endpoint. Test explicitly in Unit 17. |
| SvelteKit repo drifts from `docs/openapi.json` without anyone noticing | CI check in Unit 18 (server generates dump, diffs committed file). SvelteKit side: expected to run its codegen on each change; `api_version` mismatches cause hard failures at the envelope level. |
| `playtest-server` accidentally pulls in game crates via `playtest-cli` | Enforce via `cargo tree -p playtest-server` assertion in CI. If the CLI exposes games through a trait rather than a static registry, the server can import the trait without the games. |
| 1M-game matchup takes longer than 30 minutes (R2.4 miss) | ISMCTS is the expensive agent; most cells don't need 2000 iterations. Mitigation: per-pair adaptive budget — if win-rate confidence interval is already tight at 2000 games, stop early. Flag as a possible follow-up if R2.4 misses. |
| Determinization is subtly wrong (the observer can tell the state was resampled) | Property test in Unit 19 (Cribbage) and Unit 22 (ShipWreck): `public_view(determinize(s, p, rng), p) == public_view(s, p)` over 1000 random states. Failure in either is a hard stop before Unit 26. |
| `api_version` policy is unclear, leading to breaking changes sneaking through | `docs/api-contract.md` includes the policy up front. Every breaking change requires a new major version; the server can serve multiple versions behind path prefixes if that ever becomes necessary. |
| ShipWreck card-pool balance is wrong and random self-play games degenerate (e.g., always end in 5 turns, or always end via starvation rather than deck exhaustion) | This is a game-design issue, not an engine one. Unit 22 (turn flow) and Unit 24 (soak) both run self-play and flag distributional issues; Unit 22 adds a specific assertion that the majority of self-play games end via deck exhaustion (R9.5) rather than starvation cascade. Fixes go in `docs/shipwreck.md` and card-pool tuning. Not a blocker for this plan's exit criteria. |

## Documentation / Operational Notes

- **`docs/api-contract.md`** is the new anchor for the Rust↔SvelteKit boundary. Any wire change without a `docs/api-contract.md` update is a breaking-change red flag.
- **`docs/openapi.json`** is committed and CI-checked. SvelteKit repo pulls it by URL or sub-module.
- **`docs/shipwreck.md`** likely needs sharpening during Units 20–22 (player-count bounds, food-cost semantics, event-card quantities, tiebreaker exactness). Edits there are expected; note them in the commit.
- **`docs/BENCHMARKS.md`** gains rows for R2.2 (heuristic-beats-random), R2.3 (ISMCTS-beats-heuristic), R2.4 (matchup time), R8.3 (SSE latency: goal is <50ms from engine emit to client receive on localhost).
- **`README.md`** gets a "Web API" section pointing to `docs/api-contract.md` and a "Games" section listing both Cribbage and ShipWreck. The status line bumps to "Phase 2 shipped."
- **`docs/ARCHITECTURE.md`**: deferred at end of Phase 1; now due. Write it at the end of Unit 27 — the abstractions have seen two games and one protocol, which is enough to describe honestly.
- **Operational rollout**: still none; localhost-only server. Document the `--bind` flag and the explicit warning in the serve subcommand's `--help`.

## Alternative Approaches Considered

- **Add `serve` directly to `playtest-cli` without new crates.** Simpler, but the SvelteKit repo would have to import CLI-adjacent code to consume types. The two-crate split costs ~200 LOC of `Cargo.toml` and gains a clean wire contract — worth it.
- **Use WebSocket instead of SSE.** Offers bidirectional streaming, but the frontend never sends anything mid-game. Rejected as unnecessary complexity.
- **Implement ShipWreck as a minimal skeleton first.** Would catch trait issues faster but leaves a half-finished second game for months. User chose full implementation; this has the side benefit of validating the full metrics path for a structurally different game.
- **Build Phase 2 agents against only Cribbage, then retro-apply to ShipWreck.** Rejected because game-specific assumptions would sneak into `playtest-agents`. Agents developed against *both* games from the start have no such temptation.
- **Skip GreedyAgent, go straight to ISMCTS.** Rejected: GreedyAgent is ~100 LOC and establishes the `EvalFn<G>` pattern ISMCTS reuses. Also a useful baseline for debugging ISMCTS ("ISMCTS should at least beat Greedy in most positions").

## Success Metrics

- **Trait stability.** Unit 22 ships with zero *additional* changes to the `Game` trait. ✅ Met. (Unit 19's planned `determinize` addition was the only Game-trait delta. Unit 26 added `Hash + Eq` bounds to `Game::Action` for ISMCTS's action-keyed tree — bound widening, not a new method, so the plan treats this as a minor-surface change rather than a Success-Metric miss.)
- **Frontend readiness.** `docs/openapi.json` exists (regenerated manually via `playtest api-schema`), and `openapi-typescript` consumes it without error. ✅ Met (Unit 18).
- **Agent quality.** R2.2 and R2.3 exit criteria pass for both games. ⚠️ **Partially met** — see Post-ship findings. R2.2 passes for both (96.66% cribbage, 92.48% shipwreck). R2.3 passes for cribbage (75.38% at 10K × iter=1000) but fails for shipwreck (52.10% at 1K × iter=1000; stdev ~1.6 pp, gap is real).
- **Performance.** R2.4 passes (10K-per-pair on a 20-agent pool completes in <30 min on the reference laptop; recorded manually in `docs/BENCHMARKS.md`).
- **Dogfood moment.** A human watches a live Cribbage game end-to-end via the SSE stream at least once before this plan is called done.

## Post-ship findings

Implementation units 16–27 all shipped and are merged to `main`; the
plan's code deliverables are complete. One exit criterion is open:

- **R2.3 shipwreck gap.** ISMCTS-shipwreck at the registry default
  budget (iter=1000, `rollout_depth=50`, `exploration_c=sqrt(2)`) wins
  52.10% against heuristic-shipwreck over 1,000 games. That's a narrow,
  statistically real margin — not parity, but well short of the 65% bar.
  Suspected contributors, from most to least likely:
  1. `rollout_depth=50` is too shallow for ~150-ply ShipWreck games.
     Most rollouts hit the depth cap and fall back to `shipwreck_eval`,
     reducing ISMCTS to a shallow wrapper around the same heuristic.
  2. `shipwreck_eval` captures resources + raft length but not
     equipment-build progress. Lifting its richness closes the gap on
     heuristic play while also giving ISMCTS better leaf signals.
  3. Event-card variance (shark/typhoon/flying-fish) dominates short
     rollouts; increasing iterations or switching to a variance-reduced
     rollout policy helps.

  A follow-up plan should tune these and re-measure with the existing
  `ismcts_beats_heuristic_1k_iter1000` test before attempting the full
  10K spec test.

## Dependencies / Prerequisites

- Phases 0–1 shipped (origin plan Units 1–15 all checked). Confirmed by `git log --oneline | grep 'Unit 1[0-9]'`.
- Rust 1.95 MSRV holds (no tokio/axum version in scope needs more).
- No new external services required.

## Sources & References

- **Origin plan:** [docs/plans/2026-04-21-001-feat-playtester-phases-0-1-plan.md](2026-04-21-001-feat-playtester-phases-0-1-plan.md) — Phases 0–1 context, architectural invariants, completed units.
- **Roadmap:** [playtest-roadmap.md](../../playtest-roadmap.md) — Phases 2 and 8 mapped into this plan.
- **Game spec:** [docs/shipwreck.md](../shipwreck.md) — ShipWreck rules. Expected to sharpen during implementation.
- **ISMCTS reference:** Cowling, Powley, Whitehouse — *Information Set Monte Carlo Tree Search*, IEEE TCIAIG 2012.
- **UCB1:** Kocsis & Szepesvári, 2006.
- **SSE spec:** https://html.spec.whatwg.org/multipage/server-sent-events.html
- **Axum docs:** https://docs.rs/axum/latest/axum/ (0.8.x)
- **`tokio::sync::broadcast`:** https://docs.rs/tokio/latest/tokio/sync/broadcast/
