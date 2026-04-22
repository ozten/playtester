---
to: cribbage-ui team
from: playtester maintainer
date: 2026-04-22
subject: Phase 2.5 — browser-driven play is ready; re-wire the SvelteKit app
---

# Playtester Phase 2.5 — interactive slice shipped

You flagged this gap three days ago: the contract only let the frontend
watch AI-vs-AI runs. That's fixed. The minimum interactive slice is on
`main` in `playtester`; you can re-wire against it now.

## What shipped

- **`http-remote` agent kind** — register it in `POST /api/runs` like
  any other agent. `agents: ["http-remote", "ismcts-cribbage"]` gives
  you human-at-seat-0-vs-CPU. `agents: ["http-remote", "http-remote"]`
  is also allowed if you want two tabs.
- **`turn_prompt` SSE frame** on the per-game stream, carrying
  `{seat, prompt_id, legal_actions}`. This is the request direction
  you asked for.
- **`POST /api/runs/{run_id}/games/{game_id}/actions`** accepts
  `{seat, prompt_id, action_index}`. Success is `{accepted: true}`;
  the next `event` frame on the SSE stream is the game's response.
- **Four rejection codes** at HTTP 400: `StaleTick`, `NotYourTurn`,
  `IllegalActionIndex`, `NoRemoteAgentAtSeat`. Details payload names
  submitted vs. expected values where useful.
- Wire contract is `api_version: 1.1.0` (minor bump). See
  [`docs/api-contract.md`](../api-contract.md) §*Interactive play* for
  the full surface + a worked Cribbage example.
- Regenerated [`docs/openapi.json`](../openapi.json) — rerun
  `openapi-typescript` on it for typed client bindings.

## Minimum worked example

```text
POST /api/runs                              # agents: ["http-remote","ismcts-cribbage"]
GET  /api/runs/{rid}/games/game-0000/stream # subscribe
<-- event: header  ...
<-- event: event   (deal_card x12)
<-- event: turn_prompt data={"seat":0,"prompt_id":0,"legal_actions":[15 discard options]}
POST /api/runs/{rid}/games/game-0000/actions {"seat":0,"prompt_id":0,"action_index":0}
--> 200 {"accepted": true}
<-- event: event   (discard_to_crib)
<-- event: turn_prompt data={"seat":0,"prompt_id":1, ...}   # next decision
```

## Gotchas we discovered in build

- **`prompt_id`, not `tick`.** Your original note used `tick` on the
  submit body; we renamed to `prompt_id` because no event has been
  emitted yet for the pending prompt, so tick alignment would be
  ambiguous. Per-game monotonic counter; echo it verbatim.
- **`turn_prompt` is ephemeral.** It is NOT in the JSONL log.
  `GET .../events` pagination will never return one. If your history
  view needs "what are the current options," read live SSE, not log.
- **Reconnect replays pending once.** Dropping the tab mid-decision
  and reconnecting (with or without `Last-Event-ID`) re-emits the
  pending `turn_prompt` exactly once after catch-up. Submit clears it.
- **One tab per seat.** Two tabs open on the same seat both see the
  prompt; first valid POST wins, second gets `StaleTick`. By design.
- **Abandonment hangs.** If the browser never POSTs, the engine
  blocks forever waiting. Submission timeout is Phase 3+; for now,
  tell users to close the tab to end the game (restart the server to
  clear hung runs).

## Still Phase 3

- Stdio / CommunicationMod subprocess agent protocol.
- `LlmAgent` + Anthropic API + prompt caching.
- Scratch buffer (`plan`/`notes`/`turn_log`) and rationale field.
- Authentication on the actions endpoint.
- Submission timeouts and abandoned-game GC.

## Pointers

- [`docs/api-contract.md`](../api-contract.md) — human-readable wire
  contract, updated with everything above.
- [`docs/openapi.json`](../openapi.json) — machine contract. Run
  `openapi-typescript` against it.
- [`docs/plans/2026-04-22-001-feat-http-remote-agent-plan.md`](../plans/2026-04-22-001-feat-http-remote-agent-plan.md)
  — the plan with design rationale and scope boundaries.
- End-to-end test:
  [`crates/playtest-server/tests/http_remote_e2e.rs`](../../crates/playtest-server/tests/http_remote_e2e.rs)
  — a reference client that drives a full Cribbage game via HTTP in
  ~250 lines. Copy the SSE parsing + submit loop shape.

Ship the re-wire. Ping me if any of this contract isn't rendering the
way you need in the app — small additive tweaks are cheap at 1.1.x.
