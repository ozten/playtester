//! OpenAPI 3.1 spec builder.
//!
//! Produces a single `serde_json::Value` that describes the full wire
//! contract exposed by [`crate::routes`]. The schema of every request
//! and response body is pulled from the `schemars::JsonSchema` impls
//! that already live on the [`playtest_api`] types, so there is exactly
//! one source of truth: bumping a Rust type updates both the live
//! handlers and the OpenAPI dump on the next regeneration.
//!
//! # Shape
//!
//! The top-level object follows OpenAPI 3.1:
//!
//! ```json
//! {
//!   "openapi": "3.1.0",
//!   "info": { "title": ..., "version": API_VERSION },
//!   "paths": { ... },
//!   "components": { "schemas": { ... } }
//! }
//! ```
//!
//! `components.schemas` carries one entry per public `playtest-api`
//! type; `paths` entries reference these via
//! `$ref: "#/components/schemas/<Name>"`.
//!
//! # Regeneration
//!
//! The `playtest api-schema --out docs/openapi.json` CLI subcommand
//! pretty-prints this value to the committed artifact. No CI
//! drift-check exists in this phase — the handoff to the SvelteKit
//! repo is human-verified.

use playtest_api::{
    API_VERSION, AgentRegistryEntry, ApiError, ApiErrorCode, ApiResponse, CreateRunRequest,
    EventPage, GameMetadata, GameRegistryEntry, GameSummary, LogLineDto, RunStatus, RunSummary,
    SseFrame, SubmitActionBody, SubmitActionResponse,
};
use schemars::JsonSchema;
use schemars::r#gen::{SchemaGenerator, SchemaSettings};
use serde_json::{Map, Value, json};

/// Build the OpenAPI 3.1 JSON document for the playtester HTTP API.
///
/// Pure: called any number of times, returns the same value.
#[must_use]
pub fn openapi_json() -> Value {
    let mut generator = SchemaSettings::openapi3().into_generator();

    // Force every public API type into the generator's definitions
    // map. Using `subschema_for` returns a `$ref` into
    // `#/components/schemas/...` and side-effects the definition in.
    register::<ApiResponse<Value>>(&mut generator);
    register::<ApiError>(&mut generator);
    register::<ApiErrorCode>(&mut generator);
    register::<CreateRunRequest>(&mut generator);
    register::<RunSummary>(&mut generator);
    register::<RunStatus>(&mut generator);
    register::<GameSummary>(&mut generator);
    register::<GameMetadata>(&mut generator);
    register::<EventPage>(&mut generator);
    register::<LogLineDto>(&mut generator);
    register::<SseFrame>(&mut generator);
    register::<GameRegistryEntry>(&mut generator);
    register::<AgentRegistryEntry>(&mut generator);
    register::<SubmitActionBody>(&mut generator);
    register::<SubmitActionResponse>(&mut generator);

    // Convert the generator's definitions into a JSON object suitable
    // for `components.schemas`.
    let mut schemas = Map::new();
    for (name, schema) in generator.take_definitions() {
        let v = serde_json::to_value(schema).unwrap_or(Value::Null);
        schemas.insert(name, v);
    }

    // Inject a hand-written `HealthBody` schema — the `/api/health`
    // payload is defined locally in the health route handler (not in
    // `playtest-api`) because it is the one piece of data the server
    // itself owns. Keeping it in the dump avoids a `$ref` dangling at
    // a schema the frontend cannot resolve.
    schemas.insert(
        "HealthBody".to_owned(),
        json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["ok"]},
                "api_version": {"type": "string"}
            },
            "required": ["status", "api_version"]
        }),
    );

    let mut paths = Map::new();
    add_health(&mut paths);
    add_registry(&mut paths);
    add_runs(&mut paths);
    add_games(&mut paths);
    add_reports(&mut paths);

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Playtester API",
            "version": API_VERSION,
            "description":
                "Localhost HTTP + Server-Sent Events surface for the \
                 playtester engine. See docs/api-contract.md for \
                 versioning policy, SSE semantics, and error catalog.",
        },
        "paths": Value::Object(paths),
        "components": {
            "schemas": Value::Object(schemas),
        },
    })
}

/// Insert a referenceable schema into the generator's definitions map
/// as a side effect. Return value is discarded — we only care that the
/// type now lives in `components.schemas`.
fn register<T: ?Sized + JsonSchema>(generator: &mut SchemaGenerator) {
    let _ = generator.subschema_for::<T>();
}

// ---- Per-route-group helpers ------------------------------------------------

fn add_health(paths: &mut Map<String, Value>) {
    paths.insert(
        "/api/health".to_owned(),
        json!({
            "get": {
                "summary": "Liveness probe plus wire-contract version.",
                "description":
                    "Always returns 200 if the server is up. \
                     `data.api_version` equals the running server's \
                     `API_VERSION`; clients SHOULD refuse further \
                     requests if the major version differs from what \
                     they compiled against.",
                "responses": {
                    "200": json_response(
                        "Liveness probe succeeded.",
                        envelope_ref("HealthBody"),
                        Some(json!({
                            "api_version": API_VERSION,
                            "data": {"status": "ok", "api_version": API_VERSION},
                            "errors": []
                        })),
                    ),
                }
            }
        }),
    );
}

fn add_registry(paths: &mut Map<String, Value>) {
    paths.insert(
        "/api/games-registry".to_owned(),
        json!({
            "get": {
                "summary": "List registered games.",
                "description":
                    "Returns every game the server can dispatch. Use \
                     the `id` field in a `CreateRunRequest.game`.",
                "responses": {
                    "200": json_response(
                        "Known game catalog.",
                        envelope_array_ref("GameRegistryEntry"),
                        None,
                    ),
                }
            }
        }),
    );

    paths.insert(
        "/api/agents-registry".to_owned(),
        json!({
            "get": {
                "summary": "List registered agent kinds.",
                "description":
                    "Returns every agent kind the server can \
                     dispatch. Use the `id` field in \
                     `CreateRunRequest.agents`.",
                "responses": {
                    "200": json_response(
                        "Known agent catalog.",
                        envelope_array_ref("AgentRegistryEntry"),
                        None,
                    ),
                }
            }
        }),
    );
}

fn add_runs(paths: &mut Map<String, Value>) {
    paths.insert(
        "/api/runs".to_owned(),
        json!({
            "post": {
                "summary": "Create a run and start playing games.",
                "description":
                    "The run is accepted synchronously and executed in \
                     the background; poll `GET /api/runs/{run_id}` or \
                     subscribe to `/api/runs/{run_id}/stream` to \
                     observe progress.",
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": schema_ref("CreateRunRequest"),
                        }
                    }
                },
                "responses": {
                    "200": json_response(
                        "Run accepted.",
                        envelope_ref("RunSummary"),
                        None,
                    ),
                    "400": error_response(
                        "Validation failure (unknown game, unknown agent, bad `games_count`, wrong agent count).",
                    ),
                    "500": error_response("Internal failure registering the run."),
                },
            },
            "get": {
                "summary": "List active and recently-completed runs.",
                "responses": {
                    "200": json_response(
                        "Run summaries, newest first.",
                        envelope_array_ref("RunSummary"),
                        None,
                    ),
                }
            }
        }),
    );

    paths.insert(
        "/api/runs/{run_id}".to_owned(),
        json!({
            "get": {
                "summary": "Fetch one run's summary.",
                "parameters": [path_param("run_id", "UUID of the run.")],
                "responses": {
                    "200": json_response(
                        "Run summary.",
                        envelope_ref("RunSummary"),
                        None,
                    ),
                    "404": error_response("`run_id` is not a known run."),
                }
            }
        }),
    );

    paths.insert(
        "/api/runs/{run_id}/stream".to_owned(),
        json!({
            "get": {
                "summary": "SSE stream of run-level events.",
                "description":
                    "Server-Sent Events stream. Frames fire when games \
                     inside this run start and finish, plus one \
                     terminal `run_complete` frame. Clients may \
                     reconnect with `Last-Event-ID` but run-level \
                     frames are not re-delivered — use the per-game \
                     stream for replay semantics.",
                "parameters": [path_param("run_id", "UUID of the run.")],
                "responses": {
                    "200": sse_response(
                        "Infinite SSE stream of `{game_started, game_finished, run_complete}` frames.",
                    ),
                    "404": error_response("`run_id` is not a known run."),
                }
            }
        }),
    );
}

fn add_games(paths: &mut Map<String, Value>) {
    paths.insert(
        "/api/runs/{run_id}/games".to_owned(),
        json!({
            "get": {
                "summary": "List games inside a run.",
                "parameters": [path_param("run_id", "UUID of the run.")],
                "responses": {
                    "200": json_response(
                        "Game summaries, in start order.",
                        envelope_array_ref("GameSummary"),
                        None,
                    ),
                    "404": error_response("`run_id` is not a known run."),
                }
            }
        }),
    );

    paths.insert(
        "/api/runs/{run_id}/games/{game_id}".to_owned(),
        json!({
            "get": {
                "summary": "Fetch one game's full metadata.",
                "parameters": [
                    path_param("run_id", "UUID of the run."),
                    path_param("game_id", "Server-assigned game id (log filename stem)."),
                ],
                "responses": {
                    "200": json_response(
                        "Game metadata row.",
                        envelope_ref("GameMetadata"),
                        None,
                    ),
                    "404": error_response("`run_id` or `game_id` unknown."),
                }
            }
        }),
    );

    paths.insert(
        "/api/runs/{run_id}/games/{game_id}/events".to_owned(),
        json!({
            "get": {
                "summary": "Paginated page of log lines.",
                "description":
                    "Returns a window of records from the game's \
                     JSONL log. Records are returned in on-disk order: \
                     header, zero or more events, then a final record \
                     (if the game ended cleanly). Use `offset` + \
                     `limit` to page forward; the response's `total` \
                     field is the current line count of the log.",
                "parameters": [
                    path_param("run_id", "UUID of the run."),
                    path_param("game_id", "Server-assigned game id."),
                    query_param("offset", "Zero-based starting offset.", "integer", false),
                    query_param("limit", "Max records in the page (1..=10000). Default 100.", "integer", false),
                ],
                "responses": {
                    "200": json_response(
                        "One page of records.",
                        envelope_ref("EventPage"),
                        None,
                    ),
                    "400": error_response("`limit` out of range."),
                    "404": error_response("`run_id` or `game_id` unknown."),
                }
            }
        }),
    );

    add_actions_endpoint(paths);

    paths.insert(
        "/api/runs/{run_id}/games/{game_id}/stream".to_owned(),
        json!({
            "get": {
                "summary": "Live SSE stream of a game's log.",
                "description":
                    "Server-Sent Events stream. On connect the server \
                     replays any events up to the current tail of the \
                     on-disk log, then switches to the live feed. \
                     Reconnecting clients SHOULD send \
                     `Last-Event-ID: <tick>`; the server will skip any \
                     tick id less than or equal to that value. Each \
                     `event:`-typed frame carries its tick as the \
                     frame's SSE `id`. `heartbeat` frames carry no tick.",
                "parameters": [
                    path_param("run_id", "UUID of the run."),
                    path_param("game_id", "Server-assigned game id."),
                    last_event_id_header_param(),
                ],
                "responses": {
                    "200": sse_response(
                        "Infinite SSE stream of `SseFrame` values. \
                         Ends on the first `final` frame.",
                    ),
                    "404": error_response("`run_id` or `game_id` unknown."),
                }
            }
        }),
    );
}

fn add_actions_endpoint(paths: &mut Map<String, Value>) {
    paths.insert(
        "/api/runs/{run_id}/games/{game_id}/actions".to_owned(),
        json!({
            "post": {
                "summary": "Submit an action_index for a pending turn_prompt (Phase 2.5).",
                "description":
                    "Inbound path for browser-driven play. The body \
                     carries `{seat, prompt_id, action_index}`. The \
                     server validates against the pending prompt on \
                     the per-game `TurnCoordinator`: seat must have a \
                     registered `http-remote` agent, a prompt must be \
                     pending for that seat, the submitted `prompt_id` \
                     must match, and `action_index` must be in range. \
                     Success returns 200 with `{accepted: true}`. The \
                     next `event` frame on the SSE stream is the real \
                     signal that the action was applied.",
                "parameters": [
                    path_param("run_id", "UUID of the run."),
                    path_param("game_id", "Server-assigned game id."),
                ],
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": schema_ref("SubmitActionBody"),
                        }
                    }
                },
                "responses": {
                    "200": json_response(
                        "Action accepted; game advances asynchronously.",
                        envelope_ref("SubmitActionResponse"),
                        None,
                    ),
                    "400": error_response(
                        "Submission rejected: `StaleTick`, `IllegalActionIndex`, \
                         `NotYourTurn`, or `NoRemoteAgentAtSeat`.",
                    ),
                    "404": error_response("`run_id` or `game_id` unknown."),
                },
            }
        }),
    );
}

fn add_reports(paths: &mut Map<String, Value>) {
    // All three endpoints are stubbed at 501 in this phase; the real
    // implementation lands in a later unit (see the Phase 2 plan).
    paths.insert(
        "/api/reports".to_owned(),
        json!({
            "post": {
                "summary": "Build a markdown report for a run (stub).",
                "description":
                    "NOT IMPLEMENTED. Returns HTTP 501 + \
                     `ApiErrorCode::NotImplemented` until a later \
                     unit wires up report generation.",
                "responses": {
                    "501": error_response("Endpoint scaffolded but not yet implemented."),
                }
            }
        }),
    );

    paths.insert(
        "/api/reports/{report_id}".to_owned(),
        json!({
            "get": {
                "summary": "Fetch report metadata (stub).",
                "description":
                    "NOT IMPLEMENTED. Returns HTTP 501 + \
                     `ApiErrorCode::NotImplemented`.",
                "parameters": [path_param("report_id", "Report id.")],
                "responses": {
                    "501": error_response("Endpoint scaffolded but not yet implemented."),
                }
            }
        }),
    );

    paths.insert(
        "/api/reports/{report_id}/markdown".to_owned(),
        json!({
            "get": {
                "summary": "Fetch raw markdown body of a report (stub).",
                "description":
                    "NOT IMPLEMENTED. Returns HTTP 501 + \
                     `ApiErrorCode::NotImplemented`.",
                "parameters": [path_param("report_id", "Report id.")],
                "responses": {
                    "501": error_response("Endpoint scaffolded but not yet implemented."),
                }
            }
        }),
    );
}

// ---- Shape helpers ----------------------------------------------------------

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

/// Build an `ApiResponse<T>`-shaped schema for a specific `T` name.
/// The generated `ApiResponse` definition is parameterized as
/// `ApiResponse_for_Value` (schemars uses `Value` because we register
/// `ApiResponse<Value>`), so every endpoint response here uses an
/// inline `allOf` plus a tighter `data` shape.
fn envelope_ref(data_schema_name: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "api_version": {"type": "string"},
            "data": schema_ref(data_schema_name),
            "errors": {
                "type": "array",
                "items": schema_ref("ApiError"),
            }
        },
        "required": ["api_version", "errors"],
    })
}

fn envelope_array_ref(data_schema_name: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "api_version": {"type": "string"},
            "data": {
                "type": "array",
                "items": schema_ref(data_schema_name),
            },
            "errors": {
                "type": "array",
                "items": schema_ref("ApiError"),
            }
        },
        "required": ["api_version", "errors"],
    })
}

fn json_response(description: &str, schema: Value, example: Option<Value>) -> Value {
    let mut media = Map::new();
    media.insert("schema".to_owned(), schema);
    if let Some(ex) = example {
        media.insert("example".to_owned(), ex);
    }
    let mut content = Map::new();
    content.insert("application/json".to_owned(), Value::Object(media));
    json!({
        "description": description,
        "content": Value::Object(content),
    })
}

fn error_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": {
            "application/json": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "api_version": {"type": "string"},
                        "data": {"type": "null"},
                        "errors": {
                            "type": "array",
                            "items": schema_ref("ApiError"),
                        }
                    },
                    "required": ["api_version", "errors"],
                }
            }
        }
    })
}

fn sse_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": {
            "text/event-stream": {
                "schema": schema_ref("SseFrame"),
            }
        }
    })
}

fn path_param(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "in": "path",
        "required": true,
        "description": description,
        "schema": {"type": "string"},
    })
}

fn query_param(name: &str, description: &str, ty: &str, required: bool) -> Value {
    json!({
        "name": name,
        "in": "query",
        "required": required,
        "description": description,
        "schema": {"type": ty},
    })
}

fn last_event_id_header_param() -> Value {
    json!({
        "name": "Last-Event-ID",
        "in": "header",
        "required": false,
        "description":
            "Tick id of the last `event`-typed SSE frame the client \
             received. On reconnect the server skips any frame with a \
             tick id <= this value.",
        "schema": {"type": "string"},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_has_expected_top_level() {
        let doc = openapi_json();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], API_VERSION);
        assert!(doc["paths"].is_object());
        assert!(doc["components"]["schemas"].is_object());
    }
}
