---
title: "Bridging a blocking game loop to the main runtime via a transport-port trait"
date: 2026-04-22
category: architecture-patterns
module: playtest-agents
problem_type: architecture_pattern
component: service_object
severity: medium
applies_when:
  - "A long-running, synchronous/blocking workload must receive inputs from an async multi-threaded server runtime"
  - "A workload runs inside `tokio::task::spawn_blocking` with its own current-thread runtime built inside the blocking task"
  - "An external actor (browser, remote agent) participates non-deterministically and cannot be meaningfully recorded/replayed at the transport layer"
  - "A hexagonal-architecture codebase is tempted to add a new port purely to inject a non-deterministic collaborator"
  - "The same agent/service factory must serve both CLI (no transport) and server (with transport) callers"
tags:
  - hexagonal-architecture
  - ports-and-adapters
  - tokio
  - runtime
  - spawn-blocking
  - async-bridge
related_components:
  - tooling
  - documentation
---

# Bridging a blocking game loop to the main runtime via a transport-port trait

> Note on `component`: the schema enum is Rails-flavored and has no perfect
> value for a Rust axum HTTP+SSE server. `service_object` is the closest
> semantic analogue (server-side orchestration that mediates between the
> engine and a runtime boundary). Filter by `module: playtest-agents`,
> `tags: tokio`, or `tags: hexagonal-architecture` for precision.

## Context

Plan 003 (`docs/plans/2026-04-22-001-feat-http-remote-agent-plan.md`, shipped 2026-04-22) needed a browser to feed action indices into a running Cribbage game. Two design questions collided:

1. **Runtime topology.** The server is a multi-thread axum runtime. Each running game lives on `tokio::task::spawn_blocking` with its own current-thread runtime built inside the blocking task. A POST arriving on the main runtime has to reach an agent awaiting inside the blocking task. (This `spawn_blocking` shape predates plan 003: it was *not* in plan 002's Unit 17 spec — plan 002 said "all async; no `block_on` anywhere" — but emerged during implementation once the reality of a synchronous CPU-bound engine loop met the multi-thread runtime. The `runner.rs` doc comment now explains: the engine loop "is CPU-bound Rust and would starve the tokio runtime if run on an async worker." *(session history)*)

2. **Port placement.** The repo's load-bearing hexagonal invariant (CLAUDE.md #1) says every external-system input port ships four adapters — stub, production, record, playback — because record/playback underwrites reproducible end-to-end tests. But a browser player is non-deterministic, so "where does the browser transport live?" has two different answers depending on whether you classify it as a deterministic input. An analogous "external system vs. core abstraction" debate surfaced in Phase 0 Unit 4 when deciding where the `Agent` trait itself should live; the taxonomy that emerged then directly shaped this decision. *(session history)*

Both questions were flagged as one ce-compound candidate in plan 003's "Institutional Learnings" section.

## Guidance

1. **Bridge the two runtimes with `tokio::sync` primitives; don't try to merge them.** `tokio::sync::{mpsc, oneshot, broadcast}` handles are runtime-agnostic — a `Sender` created on the main axum runtime can be used from a different runtime inside a `spawn_blocking` task, because only the *futures* the channel produces are tied to the runtime polling them. This preserves the Phase 2 "one current-thread runtime per game inside `spawn_blocking`" shape instead of redesigning it.

2. **Non-deterministic transport lives in `playtest-agents`, not `playtest-ports`.** The 4-adapter discipline is scoped to deterministic engine inputs (Clock, Rng, FileSystem, LlmClient) because record/playback is what makes replay reproducible. Transport for a browser player is not a deterministic input — record/playback already happens at the *event* level (the chosen action becomes a durable `Event` in the JSONL log). A two-variant trait (stub for unit tests + production impl provided by the server) is sufficient.

### Crate layout

```
crates/playtest-ports/            <- deterministic inputs only:
  src/clock.rs                       Clock, Rng, FileSystem, LlmClient
  src/rng.rs                         (each has 4 adapters in playtest-adapters)
  src/filesystem.rs
  src/llm_client.rs
  src/game_event_sink.rs

crates/playtest-agents/
  src/remote/
    mod.rs                         <- re-exports
    transport.rs                   <- RemoteAgentTransport trait (2 variants)
    http_remote.rs                 <- HttpRemoteAgent<G>, generic over Game
```

### Runtime boundary

```
[ main axum runtime (multi-thread) ]          [ spawn_blocking task ]

  POST /api/runs/{}/games/{}/actions
      |
      v
  TurnCoordinator::submit(seat, pid, idx) ──>   tokio::runtime::Builder
      |                                           ::new_current_thread()
      |  mpsc::Sender<(u64, usize)>               .build()
      |  (runtime-agnostic handle)            ──> rt.block_on(game_loop)
      |                                              |
      ========================================>    HttpRemoteAgent::choose
                                                      transport.await_action()
                                                        rx.recv().await
```

Handles cross the boundary. Futures stay on the runtime that polls them.

## Why This Matters

1. **Preserves the port discipline's meaning.** Adding a fourth trait to `playtest-ports` with stub/production only (or record/playback stubs that don't really replay anything) would dilute the "every port has four meaningful adapters" invariant. Keeping the discipline scoped to deterministic inputs keeps that promise real.

2. **Keeps replay semantics correct.** Replay reconstructs a game by folding `Event`s onto `initial_state(seed)`. The agent's chosen action becomes a normal `Event` via `apply_action`. Recording transport bytes would capture user clicks — noise, not signal, and not what replay consumes.

3. **Avoids runtime juggling.** The `spawn_blocking` + current-thread inner runtime shape in `crates/playtest-registry/src/play.rs` (see `rt.block_on(loop_.run(...))` around L133 and L208) is load-bearing. `tokio::sync` primitives let the server cross that boundary without redesigning it.

4. **One build function for two callers.** `AgentBuildCtx` with `Option<Arc<dyn RemoteAgentTransport>>` lets the CLI reject `http-remote` fast with a clean error while the server admits it with a live transport — no forked build paths, no per-caller agent registry. The ripple was deliberately bounded: plan 003 flagged "Touch points: `play.rs`, `agent_registry.rs`, `playtest-cli/src/commands/`. Limited surface; verified by `cargo check --release --workspace`." *(session history)*

## When to Apply

- Adding an agent/service that takes input from a non-deterministic external actor (browser, terminal user, human-in-the-loop LLM).
- Deciding whether a new trait belongs in `playtest-ports` (and needs four adapters) or in `playtest-agents`/elsewhere (and needs only as many as its role requires). Question to ask: does record/playback of this input make a replay reproducible, or is the replay already covered by the event log?
- Bridging any blocking, isolated runtime to a main async runtime — reach for `tokio::sync` primitives before considering merging runtimes or rebuilding one side around the other.
- Introducing a cross-cutting construction context (a `Ctx` struct) where some fields are meaningful only to some callers — prefer `Option<T>` over forking the function.

## Examples

### Example 1: `RemoteAgentTransport` in `playtest-agents` (this repo, 2026-04-22)

- **Rejected layout:** put `RemoteAgentTransport` in `playtest-ports` with four adapter variants. Cost: "playback" has no meaningful semantics for a browser click stream; "record" would capture clicks that have no replay value once the `Event` is logged. Also inflates the port crate's public surface with a trait that isn't really a deterministic input port. No implementation detour was attempted — the decision was made at plan time. *(session history: no stub/record/playback adapter for transport was ever written and backed out.)*

- **Chosen layout:** `RemoteAgentTransport` defined in `crates/playtest-agents/src/remote/transport.rs` (trait at ~L32, doc comment L5-14 justifies the placement) with two async methods — `issue_prompt(seat, legal_json) -> u64` (~L40) and `await_action(seat, prompt_id) -> usize` (~L51) — and one error enum `RemoteTransportError { Cancelled, Other(String) }` (~L60). Two variants: a stub in `crates/playtest-agents/tests/http_remote_agent.rs` (`StubTransport` at ~L80, in-memory `Mutex<VecDeque<…>>`) and the production `TurnCoordinator` impl (`crates/playtest-server/src/turn_coordinator.rs` — `impl RemoteAgentTransport for TurnCoordinator` at ~L227). `HttpRemoteAgent<G>` in `crates/playtest-agents/src/remote/http_remote.rs` is generic over `G: Game` (struct at ~L22, `choose` at ~L66-105) and holds `seat: PlayerId` + `Arc<dyn RemoteAgentTransport>`. Agent registration flows through the generic `build_agent` path in `crates/playtest-registry/src/agent_registry.rs` (`AgentBuildCtx` at ~L78, `AgentBuildCtx::cli(seed, player)` at ~L87 hard-codes `remote_transport: None` so CLI callers can't instantiate `http-remote`).

### Example 2: Runtime boundary with `tokio::sync`

- Server is a multi-thread tokio runtime (axum). See `crates/playtest-server/src/runner.rs` (doc comment L1-16 explains why `spawn_blocking`; spawn site ~L272-296).
- Each game runs inside `tokio::task::spawn_blocking`. Inside, `crates/playtest-registry/src/play.rs` builds a current-thread runtime: `tokio::runtime::Builder::new_current_thread().enable_all().build()?` then `rt.block_on(loop_.run(...))` (~L133 for Cribbage, ~L208 for ShipWreck).
- `TurnCoordinator` (in `crates/playtest-server/src/turn_coordinator.rs`, struct at ~L80) is created on the main runtime before `spawn_blocking` fires. It holds per-seat `mpsc::Sender<(u64, usize)>` internally. The `Arc<TurnCoordinator>` is cloned as `Arc<dyn RemoteAgentTransport>` into the per-seat transport vector handed to `run_single_game_into_sink`. Inside the blocking task, `HttpRemoteAgent::choose` calls `transport.await_action(seat, prompt_id).await`, which does `rx.recv().await` — the `Receiver` is polled by the inner current-thread runtime while the `Sender` is written to by the main-runtime HTTP handler.
- `POST .../actions` → `TurnCoordinator::submit` (~L157) → `seat_chan.tx.try_send((prompt_id, action_index))`. The send crosses runtimes with no ceremony.

Short rule: **Handles cross runtimes. Futures don't.**

### A real bug to remember

The first `actions_endpoint` integration test hung — a real `spawn_blocking` deadlock caused by the test's own structure, not the production code. The fix was restructuring the test, not changing the bridge. Lesson: when writing tests that exercise this boundary, think carefully about which runtime the test future is polled on. See `crates/playtest-server/tests/actions_endpoint.rs` module doc (L7-14): "Coverage of rejection paths that require a `spawn_blocking` thread…" *(session history)*

## Generalized rule

When classifying a new piece of plumbing, ask "is this a deterministic engine input, or an orchestration layer?"
- **Deterministic**: it's a port. Put it in `playtest-ports`. Give it all four adapters.
- **Orchestration**: leave it in the crate whose job it actually is. Pick the variants the role needs. Use `tokio::sync` primitives to span any runtime boundary.

## Related

- **Primary source:** `docs/plans/2026-04-22-001-feat-http-remote-agent-plan.md` — "Context & Research" (~L70-78) and "Key Technical Decisions" (~L93-96) capture both sub-patterns; "Risks & Dependencies" (~L489) records runtime isolation as an explicit risk + mitigation; "Institutional Learnings" (~L82) pre-declared this ce-compound candidate.
- **Sibling pattern:** `docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md` — both docs came out of plan 003. That doc answers "what does a coordination message go through?"; this one answers "where does the trait holding those messages live and how does it cross runtimes?"
- **Prior-art anchor for the runtime shape:** `docs/plans/2026-04-21-002-feat-web-spine-shipwreck-phase-2-plan.md` — Unit 17 shipped the `spawn_blocking` + current-thread runtime pattern, even though the plan text had specced a fully-async design.
- **Invariant this pattern clarifies:** `CLAUDE.md` #1 (four adapters per port) — the new pattern does not contradict it; it records the scope: four adapters apply to deterministic inputs, not to non-deterministic orchestration traits.
- **Tests exercising the pattern:**
  - `crates/playtest-agents/tests/http_remote_agent.rs` — stub-transport unit coverage (nine `#[tokio::test]` functions starting ~L136).
  - `crates/playtest-server/tests/actions_endpoint.rs` — five `#[tokio::test]` functions (~L43) exercising rejection paths across the HTTP-to-coordinator boundary.
  - `crates/playtest-server/tests/http_remote_e2e.rs` — end-to-end Cribbage game via HTTP (~L35).
  - Inline coordinator tests at `crates/playtest-server/src/turn_coordinator.rs:303`+.
