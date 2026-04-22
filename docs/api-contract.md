# Playtester wire contract

This document is the source-of-truth prose for the Playtester HTTP +
Server-Sent Events API. The companion `docs/openapi.json` is the
machine-readable version, generated from the same Rust types this
document describes; both stay in sync because they are regenerated
from one shared source (`crates/playtest-api`).

If something is documented here but not in `openapi.json`, the OpenAPI
dump is stale — regenerate with:

```sh
cargo run --release -p playtest-cli -- api-schema --out docs/openapi.json
```

Audience: someone building the SvelteKit frontend who would rather
not read the Rust server source.

## Overview

The server is a localhost-only axum HTTP service. Every JSON response
is wrapped in a uniform envelope:

```json
{
  "api_version": "1.1.0",
  "data": { "...endpoint-specific payload..." },
  "errors": []
}
```

- `api_version` is always present. It matches the server's
  `API_VERSION` constant at send time.
- `data` is populated on success; `null` (or omitted) on failure.
- `errors` is always an array. Empty on success; one or more
  structured errors on failure. Partial-success responses carry both
  a populated `data` and a non-empty `errors`.

Streaming endpoints (paths ending in `/stream`) use
`text/event-stream` and are described in detail below.

## Versioning

`API_VERSION` is semver-major.minor.patch. The current value is
`1.1.0`.

- **Major bump** — any breaking change to request or response JSON
  shapes: fields removed, types changed, meanings changed. Clients
  MUST refuse to proceed if the server's advertised major differs
  from the one they compiled against. `GET /api/health` is the
  cheapest probe for this check.
- **Minor bump** — additive changes only: new optional fields, new
  endpoints, new error variants. Clients tolerant of unknown fields
  can ignore these.
- **Patch bump** — wording changes to error messages, perf tweaks,
  and other non-contract adjustments.

`1.1.0` (Phase 2.5) — additive introduction of the `http-remote` agent
kind, the `turn_prompt` SSE frame, `POST /api/runs/{run_id}/games/
{game_id}/actions`, and the four rejection error codes (`StaleTick`,
`IllegalActionIndex`, `NotYourTurn`, `NoRemoteAgentAtSeat`). Clients
built against `1.0.0` and tolerant of unknown fields continue to work.

There is no compatibility layer. The server speaks exactly one major
version at a time, and that version is `api_version`.

## Authentication

None. The server binds `127.0.0.1:7878` by default and is intended
for a single local user. Binding on a non-loopback address logs a
warning but is allowed; do not do this on a multi-tenant host.

## Endpoints

| Method | Path                                            | Purpose                                  |
| ------ | ----------------------------------------------- | ---------------------------------------- |
| GET    | `/api/health`                                   | Liveness + version probe                 |
| GET    | `/api/games-registry`                           | List registered games                    |
| GET    | `/api/agents-registry`                          | List registered agent kinds              |
| POST   | `/api/runs`                                     | Create a run                             |
| GET    | `/api/runs`                                     | List runs                                |
| GET    | `/api/runs/{run_id}`                            | Fetch one run                            |
| GET    | `/api/runs/{run_id}/stream`                     | SSE: run-level events                    |
| GET    | `/api/runs/{run_id}/games`                      | List games in a run                      |
| GET    | `/api/runs/{run_id}/games/{game_id}`            | Fetch one game's metadata                |
| GET    | `/api/runs/{run_id}/games/{game_id}/events`     | Paginate a game's log records            |
| GET    | `/api/runs/{run_id}/games/{game_id}/stream`     | SSE: live per-game event stream          |
| POST   | `/api/runs/{run_id}/games/{game_id}/actions`    | Submit an action for a pending prompt (1.1.0) |
| POST   | `/api/reports`                                  | Create a report (stub, 501)              |
| GET    | `/api/reports/{report_id}`                      | Fetch report metadata (stub)             |
| GET    | `/api/reports/{report_id}/markdown`             | Fetch report markdown (stub)             |

### `GET /api/health`

Liveness probe. Also returns the server's wire-contract version so
clients can decide whether their schema still matches.

Example response (HTTP 200):

```json
{
  "api_version": "1.1.0",
  "data": {
    "status": "ok",
    "api_version": "1.1.0"
  },
  "errors": []
}
```

### `GET /api/games-registry`

Returns every game the server can dispatch. Use the `id` field as
the `game` parameter in `POST /api/runs`.

Example response:

```json
{
  "api_version": "1.1.0",
  "data": [
    {
      "id": "cribbage",
      "display_name": "cribbage",
      "config_schema": {}
    }
  ],
  "errors": []
}
```

`config_schema` is JSON Schema describing this game's config blob.
Currently empty (`{}`) for every game — treat empty as "no schema
available; pass `null`/omit `config`".

### `GET /api/agents-registry`

Returns every agent kind the server knows about. Use the `id` field
in the `agents` array of `POST /api/runs`.

Example response:

```json
{
  "api_version": "1.1.0",
  "data": [
    { "id": "random",              "display_name": "random",              "supported_games": [] },
    { "id": "http-remote",         "display_name": "http-remote",         "supported_games": [] },
    { "id": "greedy-cribbage",     "display_name": "greedy-cribbage",     "supported_games": [] },
    { "id": "heuristic-cribbage",  "display_name": "heuristic-cribbage",  "supported_games": [] },
    { "id": "ismcts-cribbage",     "display_name": "ismcts-cribbage",     "supported_games": [] }
  ],
  "errors": []
}
```

**Agent catalog (Cribbage-relevant):**

| `id`                 | What it does                                                                 |
| -------------------- | ---------------------------------------------------------------------------- |
| `random`             | Uniform-random legal move. Game-agnostic; baseline for soak tests.          |
| `http-remote`        | **Phase 2.5.** Defers every `choose` to an HTTP client. The server waits for `POST .../actions` with `action_index` before advancing. Game-agnostic. See *Interactive play* below. |
| `greedy-cribbage`    | One-ply lookahead against the Cribbage eval function.                        |
| `heuristic-cribbage` | Softmax over per-action eval scores (temperature-weighted).                  |
| `ismcts-cribbage`    | Information-Set Monte-Carlo Tree Search. Accepts a parameter suffix — see below. |

**`ismcts-*` parameter suffix.** ISMCTS agents accept optional tuning
via a `:`-delimited suffix on the agent id. The base id is what
`/api/agents-registry` returns; the parameterized form is what a
client may pass in `POST /api/runs`'s `agents` array.

Shape: `"<base>:key1=value1,key2=value2"`.

Accepted keys: `iter` (iteration budget, u32), `c` (exploration
constant, f64), `rollout` (rollout depth, usize), `seed` (u64).
Any key may be omitted to keep its default.

Examples:
- `"ismcts-cribbage"` — default budget (1000 iterations, `c = √2`).
- `"ismcts-cribbage:iter=2000,c=1.4"` — stronger play, custom exploration.

**Caveat about `supported_games`.** Today the server returns an empty
array for every agent — including agents whose id suffix (e.g.
`-cribbage`) implies they are game-specific. Treat the field as
unreliable for filtering. Instead, select Cribbage-compatible agents
by name: `random` (game-agnostic) and any id ending in `-cribbage`.
Sending a per-game agent against the wrong game is rejected by the
server with `InvalidConfig`.

### `POST /api/runs`

Creates a new run and starts playing games in the background.

Example request:

```json
{
  "game": "cribbage",
  "agents": ["random", "random"],
  "games_count": 100,
  "seed": 42
}
```

- `game` — must be an `id` from `/api/games-registry`.
- `agents` — length must match the game's player count (2 for
  Cribbage in the current phase).
- `games_count` — positive integer.
- `seed` — optional. Omit to let the server pick one.
- `config` — optional, game-specific blob.

Example response (HTTP 200):

```json
{
  "api_version": "1.1.0",
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "game": "cribbage",
    "agents": ["random", "random"],
    "games_count": 100,
    "games_completed": 0,
    "seed": 42,
    "status": "Pending",
    "created_at": 1713657600000,
    "finished_at": null
  },
  "errors": []
}
```

Poll `GET /api/runs/{id}` or subscribe to
`/api/runs/{id}/stream` for progress.

Failure modes: HTTP 400 for validation (`UnknownGame`,
`UnknownAgent`, `InvalidConfig`); HTTP 500 for internal errors.

### `GET /api/runs`

Lists every run the server has registered, sorted newest-first.
Shape of each row matches `POST /api/runs`' response payload.

### `GET /api/runs/{run_id}`

Fetches one run's summary. HTTP 404 with `ApiErrorCode::RunNotFound`
if the id is unknown (or not a UUID).

### `GET /api/runs/{run_id}/games`

Lists games in a run, in start order.

Example response:

```json
{
  "api_version": "1.1.0",
  "data": [
    {
      "id": "game-0000",
      "run_id": "550e8400-e29b-41d4-a716-446655440000",
      "game": "cribbage",
      "started_at": 1713657600050,
      "finished_at": 1713657600400,
      "winner": 1
    }
  ],
  "errors": []
}
```

### `GET /api/runs/{run_id}/games/{game_id}`

Full metadata for one game. Includes the log header fields (schema
version, engine version, seed, config hash, agents) plus scores when
the game ended cleanly.

### `GET /api/runs/{run_id}/games/{game_id}/events`

Paginated read of a game's JSONL log.

Query parameters:

- `offset` — zero-based starting record index. Default `0`.
- `limit` — records to return (1..=10000). Default `100`.

Pagination is row-count based — records are counted as the line
count of the on-disk JSONL file (header + events + final record).
`total` in the response is the current line count. Walking from
`offset=0` in fixed `limit` increments reaches `total` records for a
finished game; for a running game the `total` grows between calls.

Example request: `GET /api/runs/{id}/games/game-0000/events?offset=0&limit=5`

Example response:

```json
{
  "api_version": "1.1.0",
  "data": {
    "offset": 0,
    "limit": 5,
    "total": 43,
    "events": [
      {
        "kind": "header",
        "line": {
          "kind": "header",
          "game": "cribbage",
          "schema": 2,
          "version": "0.0.0",
          "seed": 42,
          "config_hash": "...",
          "started_at": 1713657600050,
          "agents": ["random", "random"]
        }
      }
    ]
  },
  "errors": []
}
```

Each record's `line` is the full original JSONL object, untouched.
Parse `line` against your game's event type to recover typed values.

Failure modes: HTTP 400 (`InvalidPaginationParams`) if `limit == 0`
or `limit > 10000`; HTTP 404 (`GameNotFound`) if the log file is
missing.

## Server-Sent Events (SSE) streams

Two endpoints emit `text/event-stream`:

- `GET /api/runs/{run_id}/stream` — run-level lifecycle frames.
- `GET /api/runs/{run_id}/games/{game_id}/stream` — per-game log
  frames.

Clients MUST handle the infinite-stream nature of these endpoints:
they do not end with a single JSON body. Use the browser
`EventSource` API (or `@microsoft/fetch-event-source` for POST-like
patterns; the playtester server only uses GET).

### Per-game frames

Each SSE frame's `data:` payload is a JSON object. The frame's
`event:` type tag identifies the variant:

| `event:`       | `data:` shape                        | Meaning                           |
| -------------- | ------------------------------------ | --------------------------------- |
| `header`       | The log's full header JSON           | First frame; sent on connect.     |
| `event`        | One log `event` JSON record          | Game tick. Carries a numeric SSE `id:` equal to the tick id. |
| `final`        | The log's full final-record JSON     | Last frame. The stream ends here. |
| `turn_prompt`  | `{seat, prompt_id, legal_actions[]}` | **Phase 2.5.** An `http-remote` agent is waiting for a `POST .../actions` submission. Ephemeral: not resumable via `Last-Event-ID`; server re-emits on reconnect from its pending state. |
| `heartbeat`    | `null`                               | Keep-alive; emitted every ~15s.   |

The machine-readable variant tag in `SseFrame` (per
`components.schemas.SseFrame` in the OpenAPI dump) uses
`snake_case`: `header`, `event`, `final`, `turn_prompt`, `heartbeat`.

**`turn_prompt` is ephemeral.** It is generated by the server's
per-game `TurnCoordinator` when an `http-remote` agent's turn arrives;
it is **not** written to the JSONL log. `GET .../events` pagination
will never return a `turn_prompt` record. Clients that want "what
happened" history read the log; clients that want "what do I need to
do now" state subscribe to the live stream.

### Per-run frames

| `event:`         | `data:` shape                         |
| ---------------- | ------------------------------------- |
| `game_started`   | `{"game_id": "..."}`                  |
| `game_finished`  | `{"game_id": "...", "winner": ..., "scores": [...]}` |
| `run_complete`   | `null`                                |

The stream terminates after the first `run_complete` frame.

### Heartbeat

Every ~15 seconds the server emits a `heartbeat` frame with no
payload. This prevents intermediate proxies from timing out an idle
stream. Clients SHOULD ignore heartbeats for business logic and only
use them as a "server is still alive" signal.

### Reconnection and `Last-Event-ID`

For `/api/runs/{run_id}/games/{game_id}/stream` the server replays
missed events when a disconnected client reconnects.

1. The server stamps every `event:`-typed frame with an SSE `id:`
   equal to the game's tick number.
2. `header` and `final` frames are stamped too, using the adjacent
   tick — but a client reconnecting with `Last-Event-ID: N` only
   actually re-receives frames whose tick is **greater than** `N`.
3. Reconnect by including the HTTP header `Last-Event-ID: <tick>`.
   Browser `EventSource` does this automatically. On reconnect the
   server reads the current on-disk JSONL log, replays any frames
   with tick `> Last-Event-ID`, then switches back to the live
   broadcaster.
4. If the frontend disconnects without a `Last-Event-ID` (fresh
   connect), the full log is replayed from the header onward.

The run-level `/api/runs/{run_id}/stream` does not currently
replay past `game_started`/`game_finished` frames if a client
reconnects mid-run; plan for a full page reload + re-subscribe if
the run-level socket drops.

**`turn_prompt` on reconnect.** If the game is mid-decision when a
client reconnects, the server reads the coordinator's pending-prompt
state after JSONL catch-up and re-emits the current `turn_prompt`
frame once. Submitting clears the pending state; reconnecting after
a submission but before the next event fires yields no `turn_prompt`
re-emit.

## Interactive play (Phase 2.5)

Interactive play is the inbound path that lets a browser client drive
one or more seats in a live game. It layers on top of the existing
SSE stream with two additions:

1. An `http-remote` agent kind, listed in `/api/agents-registry` and
   accepted in `POST /api/runs`'s `agents` array.
2. A `turn_prompt` SSE frame (described above) + a
   `POST /api/runs/{run_id}/games/{game_id}/actions` endpoint.

The pattern is request → choose → submit:

- The engine calls the `http-remote` agent's `choose` at seat `s`.
- The server emits a `turn_prompt` frame with `{seat, prompt_id, legal_actions}`.
- The client renders the legal actions and posts the chosen index.
- The server validates and delivers; the next `event` frame on the
  stream is the game's response.

### `POST /api/runs/{run_id}/games/{game_id}/actions`

Request body:

```json
{
  "seat": 0,
  "prompt_id": 7,
  "action_index": 2
}
```

- `seat` must be a seat backed by `http-remote` in this run's agents
  array.
- `prompt_id` must match the currently-pending prompt for that seat.
- `action_index` must be `< legal_actions.len()` from that prompt.

Example success response (HTTP 200):

```json
{
  "api_version": "1.1.0",
  "data": { "accepted": true },
  "errors": []
}
```

Failure modes (HTTP 400):

| `code`                | Meaning                                                                      |
| --------------------- | ---------------------------------------------------------------------------- |
| `NoRemoteAgentAtSeat` | That seat is AI-only (not `http-remote`).                                    |
| `NotYourTurn`         | No prompt is pending for this seat, or the pending prompt is for another seat. |
| `StaleTick`           | The submitted `prompt_id` does not match the pending one; the game advanced. Fetch the latest `turn_prompt` and retry with the new id. |
| `IllegalActionIndex`  | `action_index >= legal_actions.len()`.                                       |

HTTP 404 maps to `RunNotFound` / `GameNotFound` as for the read
endpoints. Malformed JSON produces 400 via axum's default body-
parsing rejection.

### Worked Cribbage example

Create a run with seat 0 human-driven, seat 1 AI:

```sh
curl -s -X POST localhost:7878/api/runs \
  -H 'content-type: application/json' \
  -d '{"game":"cribbage","agents":["http-remote","ismcts-cribbage"],"games_count":1,"seed":7}'
```

Subscribe to the per-game stream (new game is `game-0000`):

```
GET /api/runs/{run_id}/games/game-0000/stream
```

A couple of `event` frames arrive (the engine deals cards to both
players), then a `turn_prompt` for the discard decision:

```
event: turn_prompt
data: {"seat":0,"prompt_id":0,"legal_actions":[
  {"Discard":[{"rank":"Ace","suit":"Clubs"},{"rank":"Two","suit":"Hearts"}]},
  {"Discard":[{"rank":"Ace","suit":"Clubs"},{"rank":"Three","suit":"Diamonds"}]},
  ...
]}
```

Submit the first option:

```sh
curl -s -X POST localhost:7878/api/runs/{run_id}/games/game-0000/actions \
  -H 'content-type: application/json' \
  -d '{"seat":0,"prompt_id":0,"action_index":0}'
```

The next frame on the stream is an `event` carrying
`payload.kind == "discard_to_crib"` — the game advanced. The cycle
repeats for each pegging play and for the show-phase confirmation;
see *Cribbage event payloads* below for the full taxonomy.

### What is NOT in this phase

- **No stdio / subprocess protocol.** That is Phase 3.
- **No scratch buffer, no rationale field.** Phase 3 adds the LLM-
  shaped surface; `http-remote` stays minimal.
- **No submission timeout / abandoned-game GC.** If the client never
  posts, the game hangs until the run is shut down. A timeout policy
  is Phase 3+ once real usage shapes what sensible defaults look like.
- **No submission authentication.** Consistent with the rest of the
  localhost-only trust model in this phase.

## Game event payloads

The `events` endpoint and the per-game SSE stream both carry
JSONL log records. The envelope fields (`kind`, `tick`, `payload`)
are typed in the OpenAPI dump; the **inside** of `payload` is
game-defined and opaque to the wire contract. Each game crate
documents its own event taxonomy here.

### Envelope recap

Every record is one of three `kind`s:

```json
// header — first record per game. Fixed shape across all games.
{
  "kind": "header",
  "schema": 2,
  "game": "cribbage",
  "version": "0.0.0",
  "seed": 7,
  "agents": ["random", "random"],
  "started_at": 1713657600050,
  "config_hash": "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b"
}

// event — one per engine tick. `tick` is zero-based, monotonic,
// and equals the SSE `id:` on the per-game stream.
{ "kind": "event", "tick": 0, "payload": { /* game-specific */ } }

// final — last record per game. `scores` is game-defined but always
// one entry per seat in the same order as header.agents.
{ "kind": "final", "winner": 1, "reason": "Victory", "scores": [118, 125], "finished_at": 1713657600400 }
```

**Seat-index convention.** Players are `u8` seat indices starting at
`0`, matching the order of `agents` in the create-run request (and
`header.agents` in the log). `winner` in the `final` record is a
seat index, or `null` on a draw. `scores[i]` is seat `i`'s final
score.

**`reason` values.** The `EndReason` taxonomy is shared across games:

| JSON                        | Meaning                                       |
| --------------------------- | --------------------------------------------- |
| `"Victory"`                 | A player reached the normal win condition.    |
| `"Draw"`                    | All players conceded or no legal continuation.|
| `"Stalemate"`               | Game ended because a player could not act.    |
| `{"Other": "<string>"}`     | Game-specific reason. Do not pattern-match on contents. |

### Cribbage event payloads

`payload.kind` is `snake_case`. Every variant below is what the
server puts on the wire for a Cribbage game; parse `payload`
against this taxonomy to recover typed Cribbage state.

| `payload.kind`       | Fires when                                                     |
| -------------------- | -------------------------------------------------------------- |
| `deal_card`          | One card dealt. Twelve per hand: alternating non-dealer/dealer. |
| `discard_to_crib`    | Player commits two cards to the crib. Twice per hand.          |
| `cut_starter`        | Starter card is cut from the remaining deck after discards.    |
| `nibs_scored`        | Starter was a Jack — dealer scores 2 ("his heels").           |
| `peg_played`         | Player played a card during pegging. `running_total` is the new stack sum. |
| `peg_scored`         | Pegging scored — 15, 31, pair/triple/quad, run, or last-card.  |
| `go`                 | Player said "Go" (has cards but no legal play).                |
| `pegging_round_end`  | Pegging stack reset (31 reached, or both players said Go).     |
| `pegging_complete`   | All eight cards played; pegging phase over.                    |
| `show_scored`        | One of three show steps: non-dealer hand, dealer hand, crib.   |
| `hand_complete`      | Hand ended without a winner; dealer rotates for next hand.     |
| `end_game`           | A player crossed 121. No further events follow.                |

**Worked examples** (captured from `playtest play --game cribbage
--agents random,random --seed 7`):

```json
{"kind":"deal_card","player":1,"card":{"rank":"Ace","suit":"Diamonds"}}
{"kind":"discard_to_crib","player":1,"cards":[{"rank":"Two","suit":"Hearts"},{"rank":"Eight","suit":"Clubs"}]}
{"kind":"cut_starter","card":{"rank":"Four","suit":"Clubs"}}
{"kind":"nibs_scored","player":1,"points":2}
{"kind":"peg_played","player":1,"card":{"rank":"Ace","suit":"Diamonds"},"running_total":1}
{"kind":"peg_scored","player":0,"reason":"ThirtyOne","points":2}
{"kind":"go","player":0}
{"kind":"pegging_round_end"}
{"kind":"pegging_complete"}
{"kind":"show_scored","player":1,"is_crib":false,"score":{"fifteens":4,"pairs":0,"runs":0,"flush":0,"nobs":0,"total":4}}
{"kind":"hand_complete","next_dealer":1}
{"kind":"end_game","winner":1,"reason":"Victory"}
```

**Sub-shapes:**

```jsonc
// Card — unchanged everywhere Cribbage uses cards.
{ "rank": "Ace", "suit": "Diamonds" }

// Rank: one of
//   "Ace" "Two" "Three" "Four" "Five" "Six" "Seven" "Eight" "Nine" "Ten" "Jack" "Queen" "King"
// Suit: one of
//   "Clubs" "Diamonds" "Hearts" "Spades"

// PegReason on peg_scored — one of:
//   "Fifteen"       // +2; running total landed on 15
//   "ThirtyOne"     // +2; running total landed on 31; round ends
//   "Pair"          // +2; last two cards same rank
//   "Triple"        // +6
//   "Quadruple"     // +12
//   { "Run": 3 }    // +N; last N cards form a run of N (tagged object)
//   "LastCard"      // +1; round ended under 31
// Points are already computed in `points` — you do not need to
// recompute them from the reason.

// ShowScore on show_scored — breakdown of a 4-card hand (or crib)
// scored against the starter. `total` is the sum of the other fields.
{ "fifteens": 4, "pairs": 0, "runs": 0, "flush": 0, "nobs": 0, "total": 4 }
```

**Notes for UI work.**
- `scores` on the `final` record can exceed 121 — e.g. `[118, 125]`
  — if the winning counting unit overshoots. Clamp to 121 for
  display if the rulebook wording matters to users.
- `discard_to_crib` carries both hidden cards for both players. A
  spectator UI that wants to reveal crib cards only at show time
  must suppress these payloads client-side until
  `show_scored{is_crib: true}` fires.
- `deal_card` is fully public in the log — a "fog of war" view for
  the opposing seat must also filter these client-side.
- `tick` on the `event` frame is both the JSONL event index and the
  SSE `id:` for reconnection. Paginated reads via
  `/events?offset=N&limit=M` are in line-count order, not
  tick order: offset 0 is the header, offset 1 is tick 0, offset
  `total - 1` is the `final` record.

## Error codes

Every error body has shape:

```json
{
  "code": "UnknownGame",
  "message": "unknown game: notreal",
  "details": {"game": "notreal", "known": ["cribbage"]}
}
```

`code` is stable and machine-parseable. `message` is for operator
and end-user display. `details` is optional and error-specific.

| `code`                    | HTTP | When it fires                                  |
| ------------------------- | ---- | ---------------------------------------------- |
| `UnknownGame`             | 400  | Request references a game id that isn't registered. |
| `UnknownAgent`            | 400  | Request references an agent id that isn't registered. |
| `InvalidConfig`           | 400  | Wrong `agents.len()`, bad `games_count`, etc.  |
| `InvalidPaginationParams` | 400  | `limit` out of `1..=10000` range.              |
| `StaleTick`               | 400  | `POST .../actions` sent with a `prompt_id` that no longer matches the pending prompt. (1.1.0) |
| `IllegalActionIndex`      | 400  | `POST .../actions` sent an `action_index` >= `legal_actions.len()`. (1.1.0) |
| `NotYourTurn`             | 400  | `POST .../actions` sent with no pending prompt for that seat. (1.1.0) |
| `NoRemoteAgentAtSeat`     | 400  | `POST .../actions` sent for a seat that isn't backed by `http-remote`. (1.1.0) |
| `RunNotFound`             | 404  | `run_id` is unknown or malformed.              |
| `GameNotFound`            | 404  | `game_id` has no log file on disk.             |
| `Internal`                | 500  | Unexpected server-side failure.                |
| `NotImplemented`          | 501  | Stub endpoint (currently: all `/api/reports*`). |

The authoritative mapping lives in
`playtest_api::error::http_status` — if the table above disagrees
with the source, the source wins and this table is stale.

## Pagination rules

Only the event-page endpoint paginates today.

- `offset` is zero-based and inclusive.
- `limit` is a hard cap on the returned page size.
- The server never returns more than `limit` records even when more
  are available; call again with the next `offset`.
- `offset >= total` returns an empty `events` array (not an error).
- `limit` outside `[1, 10_000]` is rejected with
  `InvalidPaginationParams`.
- `total` is a point-in-time snapshot; for a running game the count
  grows between requests. Use the SSE stream for live tails rather
  than polling `events`.

Runs and games registries are not paginated. They are bounded by the
server's in-memory run list and return every row.

## Reports (stubbed)

`POST /api/reports`, `GET /api/reports/{id}`, and
`GET /api/reports/{id}/markdown` are scaffolded so the OpenAPI
dump is complete, but the handlers return HTTP 501 with
`ApiErrorCode::NotImplemented`. Wait for a later plan unit to depend
on these. Design against the stub shape at your own risk — the final
shape may change.

## Using `openapi.json` from SvelteKit

The committed `docs/openapi.json` is safe to pipe through any
OpenAPI-3.1-aware tool. For SvelteKit we recommend
[`openapi-typescript`](https://github.com/openapi-ts/openapi-typescript):

```sh
npx openapi-typescript@latest \
    https://raw.githubusercontent.com/ozten/playtester/main/docs/openapi.json \
    -o src/lib/api-types.ts
```

or, equivalently, run it against a local checkout:

```sh
npx openapi-typescript@latest path/to/playtester/docs/openapi.json \
    -o src/lib/api-types.ts
```

The generated `.d.ts` gives you typed request/response shapes for
every endpoint; wrap it with your preferred fetch client. The
playtester CI does not run `openapi-typescript` — it is the
frontend's responsibility to re-run codegen on each contract change.

## Regenerating the dump

```sh
cargo run --release -p playtest-cli -- api-schema --out docs/openapi.json
```

Commit the result alongside whatever Rust change drove it. There is
no CI drift-check in the current phase; a human reviewer is expected
to verify the dump matches the source.
