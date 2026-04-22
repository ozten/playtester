---
title: Ephemeral coordination frame vs. logged event
date: 2026-04-22
category: architecture-patterns
module: playtest-server
problem_type: architecture_pattern
component: service_object
severity: medium
applies_when:
  - "Designing server-to-client messages for a system that also writes a durable domain event log"
  - "Adding a new SSE/WebSocket frame variant that carries orchestration metadata (prompts, nudges, session hints)"
  - "Deciding whether reconnect/catch-up state belongs in the event log or in server-side session state"
  - "Extending a hexagonal architecture where events are the serialized unit"
tags:
  - sse
  - event-log
  - coordination
  - hexagonal-architecture
  - determinism
  - replay
related_components:
  - tooling
  - documentation
---

# Ephemeral coordination frame vs. logged event

> Note on `component`: the schema enum is Rails-flavored and has no perfect
> value for a Rust axum HTTP+SSE server. `service_object` is the closest
> semantic analogue — a server-side orchestration layer that mediates between
> the engine and SSE clients. Filter by `module: playtest-server` or
> `tags: sse` for precision.

## Context

The playtest server streams each in-flight game to browsers over SSE. Phase 2 shipped spectator-only — every frame was an event that had already been folded into the JSONL log by `BroadcastGameEventSink`: write to disk, tee to the per-game broadcast channel. Phase 2.5 (`docs/plans/2026-04-22-001-feat-http-remote-agent-plan.md`, shipped 2026-04-22) added the inbound direction: a browser seat driven by `HttpRemoteAgent`. That introduced a new kind of frame — `turn_prompt` — carrying `{ seat, prompt_id, legal_actions }` so a client knows it's their turn and what their options are.

The friction: the repo already has a uniform, grep-simple, replay-simple pipeline for "things the engine says" (JSONL + broadcast). Treating `turn_prompt` as an event would reuse that pipeline with zero new plumbing. But the JSONL log is load-bearing. It is a determinism contract: the header plus events fold back into state from `initial_state(seed)` (CLAUDE.md invariant #4). A prompt isn't produced by `apply_action`, carries no state change, and is moot the instant its action is submitted or the client walks away.

The decision was made during planning, before any `turn_prompt` code was written (session history). No aborted "log it everywhere" implementation existed — the ephemeral design was captured in plan 003's "Key Technical Decisions" and the implementation walked straight to it.

## Guidance

Keep server-to-client coordination frames off the domain-event path. Route them through a dedicated channel and a small piece of server state; log only the outcome of the decision they orchestrate.

Corollaries:

- **Domain events** travel `engine → apply_action → Event → BroadcastGameEventSink → (JSONL, broadcast::Sender<String>)`. One code path, uniform, replayable.
- **Coordination frames** travel `coordinator → broadcast::Sender<String>` only. They never touch `GameEventSink`. They are additionally *stored* in a small piece of `AppState` (an `Option<Pending...>`) so a late or reconnecting subscriber can be brought back to the current waiting state without reading the log.
- The submitted decision re-enters the domain path as an action: the agent returns the chosen index, `GameLoop` calls `apply_action`, an `Event` lands in JSONL and broadcasts. The log stays a pure record of what happened, not what was asked.
- Replay / playback adapters see only the log. Coordination frames are invisible to `playback`, which is correct — replay plays back *outcomes* of past decisions, not prompts to make new ones.

Two-path sketch:

```
  turn_prompt (ephemeral)              action submit (durable)
  ---------------------------          -----------------------
  HttpRemoteAgent::choose              POST .../actions
    └─> TurnCoordinator                  └─> TurnCoordinator::submit
          ├─ pending = Some(p)                 └─ tx.send((pid, idx))
          └─ broadcast::Sender<String>   HttpRemoteAgent returns idx
               (SSE only — NOT via      GameLoop::apply_action
                GameEventSink)            └─> Event
                                              └─> BroadcastGameEventSink
                                                    ├─ JSONL file (source of truth)
                                                    └─ broadcast::Sender<String> (SSE)
```

Reconnect replay reads `TurnCoordinator::pending_snapshot()` after the normal JSONL catch-up and, if `Some`, emits one final `turn_prompt` frame (`crates/playtest-server/src/routes/games.rs` — `~L401-408`). No log entry is involved.

## Why This Matters

1. **Preserves invariant #4 (events are the serialized unit).** A `turn_prompt` is not produced by `apply_action` and carries no state transition. Logging it would force replay code to filter non-events, and a game would no longer be reconstructable byte-for-byte from `initial_state(seed) + fold(events)`.
2. **Keeps the JSONL grep-able and replay-simple.** Every downstream consumer scans for specific `"kind":"..."` values; introducing a fifth shape would either bloat the log with records that say nothing about history, or require every reader to skip them.
3. **Decouples coordination lifetime from history lifetime.** A pending prompt lives for at most the duration between "engine asks seat N" and "seat N submits or game ends." History is forever. Conflating the two couples every log-reader to the churn of transient server state and leaks stale prompts into replays.
4. **Reconnect handling lives at the right layer.** A browser reload is a server-state concern — "is this seat currently blocked waiting?" — answered by `Option<PendingPrompt>` in `TurnCoordinator`, not by re-reading a JSONL line. The SSE attach handler replays the pending prompt *after* the log catch-up, so event ordering stays coherent without polluting the log itself.

The enforcement boundary is a test, not a convention. `crates/playtest-server/tests/http_remote_e2e.rs:203` asserts `!log.contains("turn_prompt")` at end-of-game with the message "turn_prompt must not leak into the JSONL log (Phase 2.5 invariant)". Any future PR that routes a coordination frame through `BroadcastGameEventSink` fails this test.

## When to Apply

- Designing a server-to-client side-channel for an in-flight decision (turn prompts, approval requests, input requests, authorization challenges).
- Deciding whether a new frame, message, or record belongs in a domain event log or in an auxiliary coordination channel.
- Building SSE / WebSocket channels that carry *both* domain history and coordination traffic over the same wire.
- Introducing or extending `record` / `playback` adapters where coordination traffic must be invisible to replay because only outcomes are replayed.
- Adding reconnect / resume logic: if the answer to "what does a resuming client need to see?" is a point-in-time server snapshot, not a log-tail replay, the message is coordination.

## Examples

### Example 1: The `turn_prompt` decision (this repo, 2026-04-22)

**Tempting design.** Add `TurnPrompt` as a fifth log-record kind. Route it through the existing `BroadcastGameEventSink` — one fanout, uniform shape, SSE subscribers get it for free, the on-disk JSONL grows a new line per prompt. Costs: replay code now has to skip `turn_prompt` records (they aren't events and have no `apply_action` to re-run); the file includes entries that describe nothing about game state; a resuming client still needs *current*-prompt logic since arbitrarily many logged prompts are stale by the time of reconnect.

**Chosen design.** `turn_prompt` is an `SseFrame::TurnPrompt(TurnPromptPayload)` variant (`crates/playtest-api/src/sse.rs` — `TurnPromptPayload`, `SseFrame::TurnPrompt`) broadcast only on the per-game `broadcast::Sender<String>`. `TurnCoordinator::issue_prompt` stores an `Option<PendingPrompt>` in a `StdMutex` and sends the SSE line directly (`crates/playtest-server/src/turn_coordinator.rs`). `BroadcastGameEventSink` is untouched — its doc comment still reads "the durable write (to JSONL via the inner sink) is the source of truth" (`crates/playtest-adapters/src/game_event_sink/broadcast.rs`). When the browser POSTs the action, `TurnCoordinator::submit` validates and hands the index to the agent's mpsc; the agent returns it; `GameLoop` calls `apply_action`; an `Event` flows through the existing `BroadcastGameEventSink` — JSONL *and* SSE — exactly as it would for any other agent kind. Reconnect is handled in `crates/playtest-server/src/routes/games.rs`, which reads `coord.pending_snapshot()` after the log catch-up.

Two code paths, each doing one thing.

**Related naming vestige** (session history). The submit-body discriminator ended up as `prompt_id` (per-game monotonic `u64` counter dedicated to prompts), not `tick` — no event has been emitted for a pending prompt yet, so tick alignment is weak. The *error code* name `StaleTick` was deliberately preserved for wire-contract continuity with the cribbage team's original message, even though its internal check is `prompt_id` equality. Worth knowing: the name reflects the rejected design, not the chosen one.

### Example 2: Generalized rule of thumb

If a message exists only to orchestrate a pending decision — and is moot the instant that decision is made or abandoned — it is coordination, not history. Log the *outcome* of the decision, never the prompt that asked for it. If a reader of your domain history would be confused or slowed down by the message's presence, that confusion is the signal to pull the message off the log path and onto a dedicated coordination channel backed by a small piece of server state for resume.

## Related

- **Primary source:** `docs/plans/2026-04-22-001-feat-http-remote-agent-plan.md` — "Key Technical Decisions" records the decision; "Post-ship findings" (via plan 003 line 82) flagged this exact pattern as a ce-compound candidate.
- **Sibling pattern:** `docs/solutions/architecture-patterns/blocking-loop-to-main-runtime-via-transport-trait-2026-04-22.md` — the other ce-compound candidate from plan 003. This doc answers "what does `turn_prompt` go through?"; the sibling answers "where does the trait holding it live and how does it cross runtimes?"
- **Load-bearing invariant:** `CLAUDE.md` invariant #4 — "Events, not effects, are the serialized unit."
- **Sibling pattern:** `docs/plans/2026-04-21-002-feat-web-spine-shipwreck-phase-2-plan.md` established the "JSONL is source of truth; SSE is a view" rule that this pattern builds on.
- **Wire contract:** `docs/api-contract.md` and `docs/openapi.json` document the ephemerality as part of the external contract (`turn_prompt` not in `GET .../events`; re-emitted on SSE reconnect from server pending state; `StaleTick` on `prompt_id` mismatch).
- **Consumer-facing restatement:** `docs/handoffs/2026-04-22-cribbage-team-http-remote.md` — "`turn_prompt` is ephemeral. It is NOT in the JSONL log."
- **Invariant test:** `crates/playtest-server/tests/http_remote_e2e.rs:203` — `assert!(!log.contains("turn_prompt"), "turn_prompt must not leak into the JSONL log (Phase 2.5 invariant)")`.
