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
  "api_version": "1.0.0",
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
`1.0.0`.

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

There is no compatibility layer. The server speaks exactly one major
version at a time, and that version is `api_version`.

## Authentication

None. The server binds `127.0.0.1:7878` by default and is intended
for a single local user. Binding on a non-loopback address logs a
warning but is allowed; do not do this on a multi-tenant host.

## Endpoints

| Method | Path                                            | Purpose                         |
| ------ | ----------------------------------------------- | ------------------------------- |
| GET    | `/api/health`                                   | Liveness + version probe        |
| GET    | `/api/games-registry`                           | List registered games           |
| GET    | `/api/agents-registry`                          | List registered agent kinds    |
| POST   | `/api/runs`                                     | Create a run                    |
| GET    | `/api/runs`                                     | List runs                       |
| GET    | `/api/runs/{run_id}`                            | Fetch one run                   |
| GET    | `/api/runs/{run_id}/stream`                     | SSE: run-level events           |
| GET    | `/api/runs/{run_id}/games`                      | List games in a run             |
| GET    | `/api/runs/{run_id}/games/{game_id}`            | Fetch one game's metadata       |
| GET    | `/api/runs/{run_id}/games/{game_id}/events`     | Paginate a game's log records   |
| GET    | `/api/runs/{run_id}/games/{game_id}/stream`     | SSE: live per-game event stream |
| POST   | `/api/reports`                                  | Create a report (stub, 501)     |
| GET    | `/api/reports/{report_id}`                      | Fetch report metadata (stub)    |
| GET    | `/api/reports/{report_id}/markdown`             | Fetch report markdown (stub)    |

### `GET /api/health`

Liveness probe. Also returns the server's wire-contract version so
clients can decide whether their schema still matches.

Example response (HTTP 200):

```json
{
  "api_version": "1.0.0",
  "data": {
    "status": "ok",
    "api_version": "1.0.0"
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
  "api_version": "1.0.0",
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
  "api_version": "1.0.0",
  "data": [
    {
      "id": "random",
      "display_name": "random",
      "supported_games": []
    }
  ],
  "errors": []
}
```

`supported_games` empty means the agent is game-agnostic. A
non-empty array is the whitelist of game ids the agent works with.

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
  "api_version": "1.0.0",
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
  "api_version": "1.0.0",
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
  "api_version": "1.0.0",
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

| `event:` | `data:` shape                    | Meaning                           |
| -------- | -------------------------------- | --------------------------------- |
| `header` | The log's full header JSON       | First frame; sent on connect.     |
| `event`  | One log `event` JSON record      | Game tick. Carries a numeric SSE `id:` equal to the tick id. |
| `final`  | The log's full final-record JSON | Last frame. The stream ends here. |
| `heartbeat` | `null`                        | Keep-alive; emitted every ~15s.   |

The machine-readable variant tag in `SseFrame` (per
`components.schemas.SseFrame` in the OpenAPI dump) uses
`snake_case`: `header`, `event`, `final`, `heartbeat`.

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
