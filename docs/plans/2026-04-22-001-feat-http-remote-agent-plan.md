---
title: "feat: HTTP remote agent (Phase 2.5) — browser-submits-action path"
type: feat
status: shipped
date: 2026-04-22
---

# feat: HTTP remote agent (Phase 2.5) — browser-submits-action path

## Overview

Phases 0–2 shipped the full AI-vs-AI spine: deterministic engine, ports/adapters, two games, heuristics, ISMCTS, web-spine HTTP + SSE. Every external observer can *watch* a run today; none can *play* in one.

This plan adds the inbound direction. One new agent kind (`http-remote`), one new SSE frame variant (`turn_prompt`), and one new endpoint (`POST .../actions`) give a browser client the minimum it needs to drive a human-vs-CPU game — the exact slice the cribbage SvelteKit frontend needs to survive the re-wire against this server without regressing to a spectator viewer.

Phase 3's stdio agent protocol and `LlmAgent` are out of scope. This plan explicitly does *not* design the CommunicationMod-style subprocess protocol, the scratch buffer, or any LLM plumbing. It carves out the narrowest interactive slice that satisfies the cribbage use case while leaving the door open for stdio to be a sibling transport over the same async `Agent` trait.

## Problem Frame

The cribbage frontend team read `docs/api-contract.md` and correctly flagged that there is no path for a client to submit an action into an in-flight game. Every endpoint is setup (registries, `POST /api/runs`) or observation (SSE streams, paginated event reads). Re-wiring the existing human-vs-CPU SvelteKit app against the current contract would force a product regression — the app becomes a replay viewer over random-vs-random matches.

Phase 3 as drafted in `playtest-roadmap.md` does not unblock this. That plan is about stdio subprocess protocol + `LlmAgent` + prompt caching — a browser tab is not a subprocess and cannot be the other end of a pipe. Even when Phase 3 ships, the frontend would still need an HTTP transport built alongside.

The fix is small. The `Agent` trait is already async (invariant #7) and takes `(view, legal, state)`. An agent that `await`s a channel slots in with no engine changes. The SSE fan-out is built. The only missing pieces are:

1. The agent kind itself.
2. A per-game coordinator that routes between a blocking agent and HTTP handlers on the main runtime.
3. A `turn_prompt` SSE frame carrying `legal_actions`.
4. A `POST .../actions` endpoint that validates and delivers the index.

## Requirements Trace

Informed by the cribbage-team message on `2026-04-22` and the gap analysis against `docs/api-contract.md`. Every requirement here is a pre-condition for the cribbage frontend's human-vs-CPU flow to work against this server.

- **R2.5.1** A new agent kind named `http-remote` is accepted in `POST /api/runs`'s `agents` array. It can be mixed with any existing agent kind (including AI kinds, so "human vs CPU" is `agents: ["http-remote", "ismcts-cribbage"]`).
- **R2.5.2** When the engine asks the `http-remote` agent at seat `s` to choose, the server emits a `turn_prompt` SSE frame on the per-game stream carrying `{ seat, prompt_id, legal_actions }` where `legal_actions` is the JSON-serialised list indexed 0..N.
- **R2.5.3** `POST /api/runs/{run_id}/games/{game_id}/actions` with body `{ seat, prompt_id, action_index }` validates the submission and delivers the index to the waiting agent. The game advances; the next `event` frame fires with `tick = prev_tick + 1`.
- **R2.5.4** Four new error codes cover the rejection taxonomy: `StaleTick`, `IllegalActionIndex`, `NotYourTurn`, `NoRemoteAgentAtSeat`. (The first is named `StaleTick` in the cribbage-team context but implemented as a stale-prompt check; see Key Technical Decisions.)
- **R2.5.5** The JSONL event log is unchanged in shape. The chosen action becomes a normal game event exactly as it would for any other agent, so replay from seed + event log still reproduces the game byte-for-byte.
- **R2.5.6** `docs/api-contract.md` and `docs/openapi.json` document every addition with worked Cribbage examples.
- **R2.5.7** An end-to-end integration test drives a full Cribbage game via HTTP submissions, deterministic under a fixed seed + fixed action sequence.
- **R2.5.8** `playtest play --game cribbage --agents http-remote,random ...` rejects cleanly with a helpful message — the CLI has no coordinator, and silently hanging would be worse than failing fast.
- **R2.5.9** The grep-enforced invariant holds: `grep -rn 'cribbage\|shipwreck' crates/playtest-server/src/` returns nothing.

## Scope Boundaries

Everything below is *explicitly* deferred so Phase 3's design space remains open.

- **No stdio / CommunicationMod subprocess protocol.** Phase 3.
- **No scratch buffer.** The `plan` / `notes` / `turn_log` slots are designed for LLM agents. Humans don't need them.
- **No `LlmAgent` or Anthropic API integration.** Phase 3.
- **No prompt caching.** Phase 3.
- **No rationale field on submissions.** Agents that want to emit rationales can do so via their own log events later.
- **No multi-client coordination.** One browser tab per `http-remote` seat. Two tabs opening the same seat is allowed but not arbitrated — first POST to submit a valid prompt wins; second gets `StaleTick`.
- **No authentication.** Inherited from `R8.8`; localhost-only.
- **No turn-level authorization.** Anyone who can reach the server can submit actions for any seat. Consistent with Phase 2's trust model.
- **No submission timeout / abandoned-game GC.** If the browser never POSTs, the agent hangs until the run is shut down. A timeout policy is a Phase 3+ concern once usage is understood.
- **No WebSocket upgrade.** SSE for outbound, REST for inbound. Same shape as Phase 2.

### Deferred to Separate Tasks

- **SvelteKit frontend re-wire**: separate repo, separate plan. The cribbage team drives it; this plan ships only the server-side contract they need.
- **`StdioRemoteAgent` + LLMAgent**: Phase 3.
- **Action-submission authentication**: Phase 3 or later, once deployment targets beyond localhost are defined.

## Context & Research

### Relevant Code and Patterns

- **`Agent` trait** (`crates/playtest-core/src/agent.rs`) — `async fn choose(&mut self, view, legal, state) -> Result<usize, AgentError>`. Already async, already returns an index. `HttpRemoteAgent` plugs in with zero trait changes.
- **Agent registry** (`crates/playtest-registry/src/agent_registry.rs`) — `KNOWN_AGENTS` array + per-game `build_*_agent(spec, seed, player)` factories. Extension point for registering `http-remote`. The generic `build_agent` path is reserved for game-agnostic agents like `random` and `http-remote`.
- **Dispatch** (`crates/playtest-registry/src/play.rs`) — `run_single_game_into_sink` is the shared dispatch point for CLI and server. Agents are built inline per seat via `build_cribbage_agent` / `build_shipwreck_agent`. Threading an optional transport through here is the cleanest extension point.
- **Run supervisor** (`crates/playtest-server/src/runner.rs`) — each game runs on `tokio::task::spawn_blocking` with a *current-thread* `tokio` runtime built inside the blocking task. `tokio::sync` primitives (`mpsc`, `oneshot`, `broadcast`) are runtime-agnostic, so an agent on the blocking runtime can receive from a sender on the main runtime without ceremony.
- **Per-game broadcaster** (`crates/playtest-server/src/state.rs` + `runner.rs`) — `broadcast::Sender<String>` fans JSONL lines out to SSE subscribers. The `turn_prompt` frame piggybacks on the same fan-out pattern but as a distinct frame type.
- **`BroadcastGameEventSink`** (`crates/playtest-adapters/src/game_event_sink/broadcast.rs`) — the existing pattern for "engine emits line → write to disk AND publish to broadcast". `turn_prompt` is deliberately *not* routed through this path: it is coordination metadata, not a log record. See Key Technical Decisions.
- **SSE frame type** (`crates/playtest-api/src/sse.rs`) — tagged union with four variants today: `header`, `event`, `final`, `heartbeat`. Adding a fifth variant is additive (minor `api_version` bump).
- **Error taxonomy** (`crates/playtest-api/src/error.rs`) — `ApiErrorCode` + `http_status` mapping, both exhaustive-matched. Adding variants requires updating both; the compiler catches misses.
- **Hexagonal architecture — `LlmClient` port pattern** (`crates/playtest-ports/src/llm_client.rs`) — existing port/adapter pattern to mimic for the new `RemoteAgentTransport` trait, though the trait will live in `playtest-agents` alongside `HttpRemoteAgent` rather than `playtest-ports` because the "external system" here is browser-facing transport, not a deterministic input port needing record/playback.

### Institutional Learnings

- Both post-ship ce-compound candidates are now written (2026-04-22):
  - [`docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`](../solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md) — `turn_prompt` is engine→client coordination, not log history; routed through the SSE broadcast channel only, never through `GameEventSink`. Enforced by `crates/playtest-server/tests/http_remote_e2e.rs:203`.
  - [`docs/solutions/architecture-patterns/blocking-loop-to-main-runtime-via-transport-trait-2026-04-22.md`](../solutions/architecture-patterns/blocking-loop-to-main-runtime-via-transport-trait-2026-04-22.md) — two sub-patterns: (a) bridge `spawn_blocking`-hosted game loops to the main axum runtime via runtime-agnostic `tokio::sync` primitives; (b) non-deterministic transport lives in `playtest-agents` with two variants, not in `playtest-ports` with four — the 4-adapter discipline is scoped to deterministic engine inputs.

### External References

- **Server-Sent Events (WHATWG)** — `text/event-stream` MIME, `id:` and `Last-Event-ID` reconnection, `event:` tag on each frame. No spec extension needed; `turn_prompt` is just a new `event:` value.
- **Cribbage team message** (conversation record, 2026-04-22) — the four-point consumer requirements list: register interactive agent, request-action frame with legal actions, submit endpoint, structured errors. This plan's shape matches those points.

## Key Technical Decisions

- **`turn_prompt` is ephemeral, not logged.** It's engine→client coordination, not an engine outcome. Logging it would bloat the JSONL, break the "log event is the serialized unit" invariant (#4), and require replay code to skip it. The chosen action becomes an event via the normal `apply_action` path; that is the log entry. Replay of `turn_prompt` on reconnect is handled by server-side state (see below).
- **`prompt_id`, not `tick`, in the submit body.** The cribbage-team message and the feature-description both use "tick," but no event has been emitted for the pending prompt yet — tick alignment is weak. A per-game monotonic `u64` counter dedicated to prompts (`prompt_id = 0, 1, 2, ...`) is unambiguous. Each `turn_prompt` frame carries its `prompt_id`; each `POST .../actions` echoes it. Stale submission = mismatched `prompt_id`. The `StaleTick` error-code name is preserved for contract continuity but its *check* is `prompt_id` equality.
- **Transport as a port owned by `playtest-agents`, not `playtest-ports`.** The four-variant (stub/production/record/playback) discipline applies to deterministic inputs the engine consumes. A browser player is non-deterministic by definition; record/playback is handled at the *event* level (the chosen action is logged), not at the *transport* level. So the new `RemoteAgentTransport` trait lives in `playtest-agents` next to `HttpRemoteAgent`, with two variants: a stub for unit tests and a production impl provided by the server.
- **Pending-prompt replay on reconnect.** The per-game state stores `Option<PendingTurnPrompt>`. On SSE attach (fresh connect OR reconnect with `Last-Event-ID`), after the normal JSONL catch-up, if a pending prompt exists AND it hasn't been satisfied, emit it as the final catch-up frame. Cleared when the action is submitted. This is the "browser was mid-decision, reloaded the tab" path.
- **One coordinator per game, keyed by `game_id`.** Created in `run_one_game` (`runner.rs`) before the engine spawns, populated with seat→agent-channel map for any `http-remote` seats, registered in `AppState` alongside the per-game broadcaster, dropped when the game finishes. No cross-game coordination.
- **`AgentBuildCtx` replaces the `(seed, player)` tuple.** Threads optional `Arc<dyn RemoteAgentTransport>` through `build_*_agent`. CLI passes `None` and gets a clean error if the user names `http-remote`; server passes `Some(transport)` and the build succeeds. Preserves one build function for both callers.
- **Generic agent, not per-game.** `HttpRemoteAgent<G>` is generic over `G: Game`. The legal-actions list is serialized via the game's existing `Action: Serialize` bound (already required by the log writer). No per-game code enters `HttpRemoteAgent` or the server.
- **Cancel-on-game-end.** When the per-game loop exits (normal win, error, or shutdown), the coordinator is dropped. Dropping closes the action channel; any agent `await`ing `recv()` returns an error that propagates as `AgentError::Unavailable` up through `GameLoop`, which already handles agent failure. Prevents the agent from hanging forever past game end.
- **Server bumps `api_version` 1.0.0 → 1.1.0 (minor).** New frame variant, new endpoint, new error codes — all additive. Existing clients tolerant of unknown fields keep working against the new server; new clients need the new `api_version`.

## Open Questions

### Resolved During Planning

- **Logged or ephemeral `turn_prompt`?** Ephemeral. See Key Technical Decisions.
- **`tick` or `prompt_id` in submit body?** `prompt_id`. See Key Technical Decisions.
- **Where does the transport trait live?** `playtest-agents`. See Key Technical Decisions.
- **What if both seats are `http-remote`?** Allowed. `prompt_id` is global-per-game, so the two seats interleave cleanly. One POST handler dispatches by `seat`. No seat-pair specific code.
- **How does `HttpRemoteAgent` know its seat?** Passed at construction. The existing `build_*_agent(spec, seed, player)` signature already supplies `player: PlayerId`, which becomes `seat: u8`.
- **Does `http-remote` need to appear in `supported_games`?** Today `supported_games` is empty for every agent (see `api-contract.md` caveat). `http-remote` follows the same convention — empty array, the registry accepts it for any game.

### Deferred to Implementation

- **Exact channel primitive for the action path.** `tokio::sync::oneshot<(u64, usize)>` per prompt, or `tokio::sync::mpsc<(u64, usize)>` shared across a session. The shape depends on how prompt-cancellation races with submission — decide when writing the coordinator, not here.
- **Exact axum extractor style.** `Json<ActionBody>` vs. manual `Bytes` + `serde_json::from_slice`. Pick per existing route conventions.
- **Whether to reject POST on terminal game state (`game.game_over(state) == true`).** Obvious yes, but the check sits inside the coordinator's `submit` method — shape it alongside the other rejection paths rather than pre-designing here.
- **Whether `turn_prompt` should include the `view` (redacted state).** The cribbage team may want it for rendering, but it's derivable from the event stream they already have. Start without; add in a minor bump if needed.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```mermaid
sequenceDiagram
    participant Browser as Browser (SvelteKit tab)
    participant Routes as axum routes
    participant Coord as TurnCoordinator (per game)
    participant GameLoop as GameLoop (spawn_blocking)
    participant Agent as HttpRemoteAgent
    participant SSE as per-game SSE broadcaster

    Note over Routes,SSE: run already created; game in progress
    Browser->>Routes: GET .../games/{gid}/stream
    Routes->>SSE: subscribe
    SSE-->>Browser: header, event(0), event(1), ...

    Note over GameLoop,Agent: engine reaches seat 0 (http-remote)
    GameLoop->>Agent: choose(view, legal, state)
    Agent->>Coord: issue_prompt(seat=0, legal_json)
    Coord->>Coord: prompt_id = next(); pending = Some(...)
    Coord->>SSE: broadcast turn_prompt{seat, prompt_id, legal_actions}
    SSE-->>Browser: SSE: turn_prompt

    Note over Agent: awaits action channel

    Browser->>Routes: POST .../actions {seat, prompt_id, action_index}
    Routes->>Coord: submit(seat, prompt_id, action_index)
    alt valid
        Coord->>Agent: send (prompt_id, action_index)
        Coord->>Coord: pending = None
        Routes-->>Browser: 200 {}
    else stale / illegal / wrong seat
        Routes-->>Browser: 400 {code, message, details}
    end

    Agent-->>GameLoop: Ok(action_index)
    GameLoop->>GameLoop: apply_action → events
    GameLoop->>SSE: broadcast event(tick+1, ...)
    SSE-->>Browser: SSE: event
```

**Reconnect path.** A browser tab dropping mid-decision and reconnecting with `Last-Event-ID: N`: the SSE handler catches up from the on-disk JSONL through tick `N+1..current`, then reads `Coord.pending` — if `Some`, emits it as one more `turn_prompt` frame, then subscribes live. `Coord.pending` is cleared only when `submit` succeeds; this is the single source of truth for "is the game waiting on this seat right now?".

## Implementation Units

- [x] **Unit 1: `RemoteAgentTransport` port trait + `HttpRemoteAgent<G>` + unit tests**

**Goal:** Establish the transport abstraction and the game-agnostic agent that uses it. No server wiring yet — the agent is testable against a stub transport.

**Requirements:** R2.5.1, R2.5.2 (partial — agent-side only), R2.5.5

**Dependencies:** None (Phase 2 shipped).

**Files:**
- Create: `crates/playtest-agents/src/remote/transport.rs` — defines `RemoteAgentTransport` trait, `PendingPrompt`, `AgentError` surface for remote failures.
- Create: `crates/playtest-agents/src/remote/http_remote.rs` — `HttpRemoteAgent<G>` impl of `Agent<G>`.
- Create: `crates/playtest-agents/src/remote/mod.rs` + wire into `crates/playtest-agents/src/lib.rs`.
- Test: `crates/playtest-agents/tests/http_remote_agent.rs` — against a hand-rolled stub transport.

**Approach:**
- Trait shape (directional): `async fn issue_prompt(&self, seat: u8, legal_json: Vec<JsonValue>) -> u64` (returns `prompt_id`) and `async fn await_action(&self, seat: u8, prompt_id: u64) -> Result<usize, RemoteTransportError>`.
- `HttpRemoteAgent<G>` holds `seat: u8` + `Arc<dyn RemoteAgentTransport>`. Its `choose` serializes `legal` via the game's existing `Action: Serialize` bound, calls `issue_prompt`, then `await_action`, then returns the index.
- Error translation: `RemoteTransportError` → `AgentError::Unavailable` with message carrying context. The engine wraps that in `GameError::AgentFailed`.
- Stub transport in tests is a `Mutex<VecDeque<usize>>` that pops an action index each time `await_action` is called; `issue_prompt` no-ops. Proves the agent logic without any network or channel plumbing.

**Execution note:** Start with a failing unit test that drives a fake one-action game through `HttpRemoteAgent + StubTransport`; the test shape pins the trait signature.

**Patterns to follow:**
- `LlmClient` port in `playtest-ports/src/llm_client.rs` for the trait-style abstraction.
- `RandomAgent` in `playtest-agents/src/random.rs` for the `Agent<G>` impl layout.

**Test scenarios:**
- Happy path: stub transport returns index 0; `choose` returns 0; call count = 1.
- Happy path: sequence of 3 choose calls against a 3-element queue returns `[0, 1, 2]` in order.
- Edge case: legal slice with exactly one element — agent still emits the prompt (not skipped) and returns 0.
- Error path: transport returns `RemoteTransportError::Cancelled` → `choose` returns `AgentError::Unavailable` with a cancellation message.
- Integration (stub-scope): prompt-and-action over stub transport round-trip 1000 times without leaks.

**Verification:**
- `cargo check --release -p playtest-agents` passes.
- `cargo clippy --release -p playtest-agents -- -D warnings` clean.
- `HttpRemoteAgent` holds no game-specific code (grep `cribbage\|shipwreck` in the new files returns nothing).

---

- [x] **Unit 2: `AgentBuildCtx` threading + `http-remote` registration + CLI rejection**

**Goal:** Thread the optional transport through the shared agent-build and dispatch machinery so both CLI and server use one path. `http-remote` shows up in `KNOWN_AGENTS` and `/api/agents-registry` but cleanly fails fast when no transport is supplied.

**Requirements:** R2.5.1, R2.5.8, R2.5.9

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/playtest-registry/src/agent_registry.rs` — add `AgentBuildCtx { seed, player, remote_transport: Option<Arc<dyn RemoteAgentTransport>> }`; change `build_cribbage_agent` / `build_shipwreck_agent` / `build_agent` signatures to take `&AgentBuildCtx`; add `"http-remote"` to `KNOWN_AGENTS`.
- Modify: `crates/playtest-registry/src/play.rs` — construct `AgentBuildCtx` per seat; pass through `run_single_game_into_sink`'s new optional `remote_transports: Option<&[Option<Arc<dyn RemoteAgentTransport>>]>` parameter indexed by seat.
- Modify: `crates/playtest-cli/src/commands/play.rs` — call new signature with `remote_transports: None`; CLI produces a clear error if `http-remote` appears in the agents list.
- Modify: `crates/playtest-cli/src/commands/matchup.rs` (if present) — same update.
- Test: existing CLI tests pass; add a new test in `crates/playtest-registry/tests/` asserting that building `http-remote` with `remote_transport: None` returns a specific error variant.

**Approach:**
- `build_agent(name, ctx)`: when `name == "http-remote"`, require `ctx.remote_transport.is_some()`; if `None`, bail with `"http-remote agent requires a server-provided transport; use POST /api/runs with agents=[\"http-remote\", ...] instead of the CLI"`.
- `run_single_game_into_sink` grows an optional `remote_transports: Option<&[Option<Arc<dyn RemoteAgentTransport>>]>`. Per-seat: if `Some(Some(t))`, propagate into `AgentBuildCtx.remote_transport`; else `None`.
- Server call sites pass `Some(&transports)` built from the coordinator (Unit 3). CLI passes `None`.
- Preserve `is_known_agent("http-remote") == true` so `POST /api/runs` validation accepts it.

**Patterns to follow:**
- Existing `build_cribbage_agent` match arms for error message shape.
- Existing `KNOWN_AGENTS` ordering (generic first, then per-game).

**Test scenarios:**
- Happy path: CLI play with `random,random` still works (no regression from signature change).
- Happy path: `build_agent("http-remote", ctx_with_transport)` returns an agent successfully.
- Error path: `build_agent("http-remote", ctx_without_transport)` returns an error containing `"requires a server-provided transport"`.
- Error path: `playtest play --game cribbage --agents http-remote,random` exits non-zero with the same helpful message.
- Integration: `GET /api/agents-registry` includes `"http-remote"` in the returned list.

**Verification:**
- Existing CLI integration tests pass.
- No game-specific strings added to `playtest-server/src/` (grep invariant holds).

---

- [x] **Unit 3: Server-side `TurnCoordinator` + `AppState` extension**

**Goal:** Implement the production `RemoteAgentTransport` on the server side, owning the per-game pending-prompt state and the agent channels. Register coordinators in `AppState` alongside the existing per-game broadcasters.

**Requirements:** R2.5.1, R2.5.2, R2.5.3 (coordinator-side), R2.5.4 (taxonomy)

**Dependencies:** Units 1, 2.

**Files:**
- Create: `crates/playtest-server/src/turn_coordinator.rs` — `TurnCoordinator` struct + `impl RemoteAgentTransport for Arc<TurnCoordinator>`.
- Modify: `crates/playtest-server/src/state.rs` — `RunHandle` gains `turn_coordinators: DashMap<String, Arc<TurnCoordinator>>`.
- Modify: `crates/playtest-server/src/runner.rs` — `run_one_game` builds the coordinator *before* spawning the blocking engine, inserts it into the run handle, hands per-seat transports into `run_single_game_into_sink`, and removes the coordinator when the game ends.
- Test: `crates/playtest-server/tests/turn_coordinator.rs` — unit tests against an in-memory coordinator without running a real engine.

**Approach:**
- `TurnCoordinator` holds: `next_prompt_id: AtomicU64`, `pending: Mutex<Option<PendingPrompt>>`, `seats: HashMap<u8, mpsc::Sender<(u64, usize)>>` (or oneshot map keyed by `prompt_id` — decide when implementing), and a reference to the per-game broadcaster for `turn_prompt` emission.
- `issue_prompt(seat, legal_json)`: assigns the next `prompt_id`, stores `PendingPrompt { seat, prompt_id, legal_json }`, broadcasts a `TurnPrompt` SSE frame JSON, returns the `prompt_id`.
- `await_action(seat, prompt_id)`: awaits the matching seat's channel for a tuple whose `prompt_id` matches; drops any stale ones silently.
- `submit(seat, prompt_id, action_index)`: checks coordinator has seat registered (`NoRemoteAgentAtSeat`), pending is `Some` and seat+prompt_id match (`StaleTick` otherwise), `action_index < pending.legal_actions.len()` (`IllegalActionIndex` otherwise), then sends on the channel and clears `pending`.
- Cancellation: `Drop for TurnCoordinator` closes all seat senders, which unblocks any `await_action` with a `Cancelled` error → `AgentError::Unavailable` → `GameError::AgentFailed` → run supervisor marks run failed. Acceptable semantics on game end.

**Patterns to follow:**
- `RunHandle` (`state.rs`) for the per-run registry pattern; mirror it for coordinators.
- `run_one_game`'s pre-spawn setup for broadcasters (`runner.rs:181-196`) — identical lifecycle.

**Test scenarios:**
- Happy path: `issue_prompt` followed by `submit` delivers the index and clears pending.
- Happy path: interleaved two-seat coordination — seat 0's prompt_id=0, then seat 1's prompt_id=1 — both resolve independently.
- Edge case: submit arrives before pending is set → `NotYourTurn` (no pending prompt).
- Edge case: submit with the right seat but wrong `prompt_id` → `StaleTick`.
- Edge case: submit to a seat not backed by `http-remote` → `NoRemoteAgentAtSeat`.
- Edge case: `action_index == legal_actions.len()` → `IllegalActionIndex`.
- Integration: coordinator dropped mid-await → `await_action` returns cancellation error; agent propagates as `AgentError::Unavailable`.
- Integration: coordinator emits `turn_prompt` to the broadcaster exactly once per `issue_prompt`.

**Verification:**
- `cargo check --release -p playtest-server` passes.
- `grep -rn 'cribbage\|shipwreck' crates/playtest-server/src/` returns nothing.
- No `SystemTime::now()` / `thread_rng()` introduced (determinism audit test still passes).

---

- [x] **Unit 4: `SseFrame::TurnPrompt` variant + per-game SSE merge + pending-prompt replay**

**Goal:** Wire the new SSE frame variant end-to-end: API type, server-side broadcast merge, and reconnect-time pending-prompt replay.

**Requirements:** R2.5.2, R2.5.5

**Dependencies:** Unit 3.

**Files:**
- Modify: `crates/playtest-api/src/sse.rs` — add `SseFrame::TurnPrompt(JsonValue)` variant with snake_case tag `turn_prompt`.
- Modify: `crates/playtest-server/src/sse.rs` — helper for building a `turn_prompt` frame line from a seat/prompt_id/legal payload.
- Modify: `crates/playtest-server/src/routes/games.rs` — in the per-game SSE handler, after the JSONL catch-up and before the live subscribe, check the coordinator's `pending` and emit one final `turn_prompt` frame if present; during the live merge, forward coordinator-broadcast `turn_prompt` frames alongside the existing `event` broadcasts.
- Test: extend `crates/playtest-server/tests/sse_contract.rs` with reconnect-mid-decision scenarios.

**Approach:**
- The per-game broadcaster currently carries `String` (JSONL lines). Add a second broadcast or tag lines with a frame-kind prefix — implementation choice deferred, but keep the "broadcaster carries strings that parse to `SseFrame`" invariant.
- SSE route: JSONL catch-up stays as-is. After it, if `coordinator.pending.is_some()`, emit the frame. Then call `broadcaster.subscribe()` and continue live.
- `turn_prompt` frames do NOT carry an SSE `id:` — they are not tick-stamped and are not resumable; the pending-prompt replay covers the reconnect path.
- `api_version` bump to `1.1.0` — the `Header` frame's `api_version` advertises this from the first response after deploy.

**Patterns to follow:**
- Existing four-variant `SseFrame` in `playtest-api/src/sse.rs`.
- Existing `line_to_sse_frame` pattern in `playtest-server/src/sse.rs` — the `TurnPrompt` path is *not* wired through here (it isn't a log line), but mimic the helper style.

**Test scenarios:**
- Happy path: connect before a prompt fires, observe `turn_prompt` arriving on the live stream when the engine reaches the remote seat.
- Happy path: `SseFrame::TurnPrompt` JSON serialization round-trips through `serde`.
- Edge case: client reconnects with `Last-Event-ID: <N>` while a prompt is pending — receives events through N, then the pending `turn_prompt`, then live.
- Edge case: client reconnects *after* submission but before the next event — no `turn_prompt` re-emit; only subsequent events.
- Edge case: fresh connect to a game with a pending prompt — full log replay + `turn_prompt` + live.
- Integration: two-seat `http-remote,http-remote` game emits interleaved `turn_prompt` frames with monotonic `prompt_id`.

**Verification:**
- `docs/openapi.json` regenerated includes the new frame variant.
- `cargo test --release -p playtest-server --test sse_contract` passes.
- No regression in existing SSE tests.

---

- [x] **Unit 5: `POST /api/runs/{run_id}/games/{game_id}/actions` + new error codes**

**Goal:** Add the inbound endpoint and wire it to the coordinator, with the full rejection taxonomy in `ApiErrorCode`.

**Requirements:** R2.5.3, R2.5.4

**Dependencies:** Units 3, 4.

**Files:**
- Modify: `crates/playtest-api/src/error.rs` — add `ApiErrorCode::StaleTick`, `IllegalActionIndex`, `NotYourTurn`, `NoRemoteAgentAtSeat`; extend `http_status` (all 400).
- Modify: `crates/playtest-api/` — add request type `SubmitActionBody { seat: u8, prompt_id: u64, action_index: u32 }` and response type (probably `serde_json::Value::Null` or a trivial `Ok` type).
- Modify: `crates/playtest-server/src/routes/games.rs` — `POST /api/runs/:run_id/games/:game_id/actions` handler looking up the coordinator, calling `submit`, mapping errors.
- Modify: `crates/playtest-server/src/routes/mod.rs` — register the new route.
- Test: `crates/playtest-server/tests/actions_endpoint.rs` — coordinator interactions through HTTP.

**Approach:**
- Lookup path: `active_runs.get(&run_id).ok_or(RunNotFound) → run_handle.turn_coordinators.get(&game_id).ok_or(GameNotFound) → coordinator.submit(...)`.
- Coordinator errors map straightforwardly: `NotYourTurn` → 400 + `NotYourTurn`; `StaleTick` → 400 + `StaleTick`; `IllegalActionIndex` → 400 + `IllegalActionIndex`; `NoRemoteAgentAtSeat` → 400 + `NoRemoteAgentAtSeat`. `details` payload carries the submitted vs. expected values when useful.
- Success response is a minimal `{ "accepted": true }` or similar — shape deferred to implementation, but documented in the contract (Unit 7).
- The post-submit game advance is already driven by the engine loop; the endpoint does not wait on it — it returns 200 as soon as the coordinator acknowledges delivery.

**Patterns to follow:**
- Existing axum route handlers in `routes/runs.rs` and `routes/games.rs` — same `State<AppState>` extractor, same `ApiError` mapping, same envelope (`ApiEnvelope<T>`).
- Existing validation-error style in `POST /api/runs` for request-body parsing errors.

**Test scenarios:**
- Happy path: valid submission for a pending prompt → 200; next event follows on the SSE stream.
- Error path: run id unknown → 404 `RunNotFound`.
- Error path: game id unknown → 404 `GameNotFound`.
- Error path: seat has no `http-remote` agent → 400 `NoRemoteAgentAtSeat`.
- Error path: no prompt pending for that seat → 400 `NotYourTurn`.
- Error path: prompt_id mismatch → 400 `StaleTick`.
- Error path: action_index >= legal_actions.len() → 400 `IllegalActionIndex`.
- Error path: malformed JSON body → 400 (axum-default `InvalidConfig`-style handling; alternatively a new code if it proves confusing in practice).
- Integration: a full Cribbage hand — discard + pegging decisions — driven entirely via POST requests, game terminates, `final` frame fires.

**Verification:**
- `cargo test --release -p playtest-server --test actions_endpoint` passes.
- Exhaustive match on `ApiErrorCode` in `http_status` still compiles (new variants assigned).
- `docs/openapi.json` dump includes the new endpoint + error codes.

---

- [x] **Unit 6: End-to-end integration test — full Cribbage game via HTTP submissions**

**Goal:** Prove the inbound path works at the integration boundary by playing a complete Cribbage game end-to-end through the real server, the real coordinator, and real HTTP. Deterministic under a fixed seed + fixed action-index sequence.

**Requirements:** R2.5.7

**Dependencies:** Units 1–5.

**Files:**
- Create: `crates/playtest-server/tests/http_remote_e2e.rs` — spawns server, POSTs a run with `agents: ["http-remote", "random"]`, subscribes to the SSE stream, responds to every `turn_prompt` with a predetermined action_index, asserts the game ends with a `final` frame and matching JSONL on disk.
- Create or modify: `crates/playtest-server/tests/common/` harness helpers — probably a small SSE-reading helper keyed on frame kind, and a POST helper for action submission.

**Approach:**
- Use a recorded action sequence — playing Cribbage end-to-end through HTTP with truly random input would be hard to assert. Instead, capture a reference game once (agents `random`, seed `42`) at the action-index level, then have the test replay those action indices via HTTP — the resulting JSONL should be byte-identical to the reference run's log.
- The reference action indices are captured by a one-shot helper in the test file (e.g., a `#[test]` that plays one game and dumps indices to a const array).
- Alternative if byte-identity is too brittle: assert only that the game terminates with a `final` frame, that every submitted action was legal, and that at least one of each Cribbage event kind appeared. Decide per what the test proves most cheaply.

**Execution note:** Start with the smallest viable test — a single discard + a single peg play against a `random` opponent, asserting the next event arrives. Expand to a full hand, then a full game.

**Patterns to follow:**
- Existing `sse_contract.rs` test for server-spawn + SSE subscription patterns.
- Existing `server_smoke.rs` for run-creation patterns.

**Test scenarios:**
- Happy path: full Cribbage game driven via HTTP submissions terminates with `final` + `end_game` event + winner in `[0, 1]`.
- Happy path: JSONL log on disk after the test exits has one `header`, one or more `event`s per expected game phase, one `final`.
- Edge case: an intentionally illegal action_index during the test returns 400 and the game continues pending the next submission (prompt is NOT cleared on rejected submits).
- Integration: server SSE stream emits exactly one `turn_prompt` per decision point for the `http-remote` seat.
- Integration: no `turn_prompt` frames appear in the on-disk JSONL log.

**Verification:**
- Test runs green under `cargo test --release -p playtest-server --test http_remote_e2e`.
- Test completes in under 5 seconds (a Cribbage game is short).

---

- [x] **Unit 7: Contract docs refresh — `api-contract.md` + `openapi.json` + worked example**

**Goal:** Everything added in Units 1–5 is discoverable by someone reading the contract without reading Rust source. The cribbage team can consume this plan's output with no further context.

**Requirements:** R2.5.6

**Dependencies:** Units 1–5 (implementation shape must be final).

**Files:**
- Modify: `docs/api-contract.md` — add the `http-remote` row to the agent catalog; document the `POST .../actions` endpoint; add `turn_prompt` to the per-game SSE frame table; document the four new error codes; add a Cribbage worked example showing a full `turn_prompt → POST /actions → event` round-trip; update the `api_version` narrative to `1.1.0` and describe what's new.
- Regenerate: `docs/openapi.json` via `cargo run --release -p playtest-cli -- api-schema --out docs/openapi.json`.
- Modify: `crates/playtest-server/src/schema.rs` (or equivalent) — ensure the new route + frame variant + error codes show up in the OpenAPI dump.

**Approach:**
- Add a `### Interactive play` subsection to `api-contract.md` between the "Endpoints" table and the "SSE streams" section, describing the `http-remote` agent, the `turn_prompt` frame, and the `actions` endpoint as one coherent flow.
- Worked example in Cribbage shows: (a) `POST /api/runs` with `agents: ["http-remote", "random"]`; (b) subscribing to the per-game SSE stream; (c) receiving `turn_prompt` with `legal_actions` showing the four discard choices; (d) `POST .../actions` with `action_index: 2`; (e) next `event` frame firing.
- Call out explicitly that `turn_prompt` is **ephemeral** — not in `GET .../events` pagination — and explain why. Link forward to Phase 3 for the stdio variant.
- Document the `api_version` bump to `1.1.0` and the policy (additive → minor; clients tolerant of unknown fields stay compatible).

**Patterns to follow:**
- Existing structure of `docs/api-contract.md` (endpoint table, frame table, error-code table).
- Existing `Cribbage event payloads` worked-example format at line ~441.

**Test scenarios:**
- `cargo run --release -p playtest-cli -- api-schema --out /tmp/check.json` produces a schema containing the new endpoint and frame variant.
- Committed `docs/openapi.json` matches the freshly generated output (manual diff; no CI drift-check per Phase 2 policy).
- The `openapi-typescript` recipe in `api-contract.md` (line ~580) still works against the new schema (human spot-check, no automated test).

**Verification:**
- A reader of `docs/api-contract.md` can build a working client without reading Rust source.
- No broken internal links; existing sections unchanged in shape.

---

- [x] **Unit 8: Cribbage-team handoff message draft**

**Goal:** A short, copy-pasteable message the maintainer can send to the cribbage UI team confirming what's now supported, giving a minimal worked example, and calling out what's still Phase 3.

**Requirements:** Inbound-team handoff (no numbered R but implied by the problem frame).

**Dependencies:** Units 1–7. Exact wording depends on what implementation reveals.

**Files:**
- Create: `docs/handoffs/2026-04-22-cribbage-team-http-remote.md` — the message as a committed artifact, so it's archived and the team has a stable URL.

**Approach:**
- Keep under 400 words. Four sections:
  1. **What shipped.** One-paragraph summary: `http-remote` agent kind, `turn_prompt` frame, `POST .../actions` endpoint. Link to `docs/api-contract.md` section.
  2. **Minimum worked example.** Five-step flow from `POST /api/runs` with `agents: ["http-remote", "ismcts-cribbage"]` through first `turn_prompt` frame through submitting the discard through the next event. Inline JSON snippets.
  3. **Gotchas we found in build.** Filled in post-implementation — e.g., "prompt_id is not the same as tick," "turn_prompt is ephemeral not logged," "reconnect with Last-Event-ID replays the pending prompt exactly once," and anything else Units 1–6 surface.
  4. **Still Phase 3.** One short paragraph: stdio protocol, `LlmAgent`, scratch buffer, rationale field, submission auth.
- Link the handoff doc back to this plan so the team can see the scope boundary.

**Patterns to follow:**
- Any existing `docs/` wording style (no `docs/handoffs/` precedent in the repo yet — this unit establishes it).

**Test expectation: none — the deliverable is prose. Verification is a read-through by the maintainer before sending.**

**Verification:**
- Message fits in one screen.
- Every claim in the message is grounded in shipped code or shipped docs (nothing speculative).
- The team can read it, read `docs/api-contract.md` once, and begin their re-wire.

---

## System-Wide Impact

- **Interaction graph:** The engine loop (on `spawn_blocking`) now talks out of band to `TurnCoordinator` for `http-remote` seats. This is the first time an agent has coordinated with server state outside the sink; the pattern is contained to `HttpRemoteAgent` and does not leak into `GameLoop` or `Game` trait.
- **Error propagation:** A rejected submit is a user error (400). A dropped coordinator mid-game is a run failure — the agent returns `AgentError::Unavailable`, the engine returns `GameError::AgentFailed`, the run supervisor marks the run `Failed`. That path is already handled in `runner.rs`.
- **State lifecycle risks:** `TurnCoordinator` must be dropped exactly once per game end to avoid leaks. Register in `RunHandle.turn_coordinators` on game start; remove in `run_one_game` after the blocking-task `.await` completes, parallel to the existing `game_broadcasters` removal (`runner.rs:246`).
- **API surface parity:** The `openapi.json` dump and `api-contract.md` are two views of the same surface; both must update in Unit 7. The minor `api_version` bump (1.0.0 → 1.1.0) is the contract signal to clients that a new capability exists.
- **Integration coverage:** Unit 6 is the load-bearing cross-layer test — it proves the coordinator ↔ agent ↔ SSE ↔ HTTP path end-to-end. Unit 3's coordinator tests are narrower; Unit 5's endpoint tests are narrower. Unit 6 is where the plan's correctness is proved.
- **Unchanged invariants:**
  - `Game` trait: unchanged. No new methods, no new bounds.
  - JSONL log schema: unchanged at v2.
  - Replay semantics: unchanged — the chosen action enters the log as a normal event; replay from seed + event log still reproduces the game.
  - `playtest-server` game-agnosticism: preserved; grep invariant holds.
  - Determinism audit: unchanged; no new `SystemTime::now()` or `thread_rng()`.
  - CLI: unchanged for all non-`http-remote` agents; rejects `http-remote` with a clear message.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Agent hangs forever if browser never POSTs (abandoned game) | Out of scope for this plan (Scope Boundary). Documented so operators know; Phase 3+ adds a timeout policy once usage is understood. |
| Runtime isolation bug — agent on current-thread runtime awaits a channel whose sender is on the main runtime | `tokio::sync` primitives are runtime-agnostic. Unit 3's coordinator tests exercise this explicitly; Unit 6's e2e test proves it in the real configuration. |
| `turn_prompt` pollutes the log if a future change routes it through `BroadcastGameEventSink` | Key Technical Decisions explicitly documents ephemerality; `turn_prompt` has its own broadcast path, not routed through the sink. A grep-test could guard this but is overkill for the bounded surface. |
| Reconnect-after-submit race — client reconnects just as submit lands, sees stale `turn_prompt` | Coordinator clears `pending` synchronously inside `submit` before ack; SSE reconnect reads `pending` after the ack has completed. Normal fence; no extra work needed. |
| Two tabs open for the same seat race to submit | First valid POST wins (clears `pending`); second gets `StaleTick`. This is by-design behavior, documented in the contract, not a bug. |
| `AgentBuildCtx` signature ripple breaks an unreviewed call site | The compiler catches it. Touch points: `play.rs`, `agent_registry.rs`, `playtest-cli/src/commands/`. Limited surface; verified by `cargo check --release --workspace`. |
| The cribbage team builds against `prompt_id` semantics that we later want to rename | `prompt_id` is in the wire contract at `api_version 1.1.0`; renaming would be a major bump. Commit to the name. |

## Documentation / Operational Notes

- `docs/api-contract.md` and `docs/openapi.json` are the two contract artifacts; both are committed and versioned. SvelteKit codegen reads `openapi.json`.
- `api_version` moves from `1.0.0` to `1.1.0`. All additions are minor (additive). Clients built against 1.0 and tolerant of unknown fields keep working.
- No deployment changes (still localhost).
- Handoff artifact (`docs/handoffs/2026-04-22-cribbage-team-http-remote.md`) lives in the repo and can be linked from Slack/email rather than retyped per-recipient.

## Sources & References

- **Prior plan:** `docs/plans/2026-04-21-002-feat-web-spine-shipwreck-phase-2-plan.md` — the web spine this plan extends.
- **Architecture invariants:** `CLAUDE.md` at repo root.
- **Roadmap:** `playtest-roadmap.md` — Phase 3 remains scoped to stdio + LLM; this plan does not consume any of Phase 3's scope.
- **Contract:** `docs/api-contract.md` (current wire surface); `docs/openapi.json` (machine-readable form).
- **Cribbage team message:** recorded in conversation log `2026-04-22` — the four-item requirements list.
- Related code: `crates/playtest-core/src/agent.rs` (Agent trait), `crates/playtest-registry/src/play.rs` (dispatch), `crates/playtest-server/src/runner.rs` (per-game lifecycle), `crates/playtest-server/src/state.rs` (AppState shape).
