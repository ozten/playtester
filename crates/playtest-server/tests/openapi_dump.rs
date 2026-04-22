//! Contract test for the OpenAPI dump.
//!
//! Guards three invariants:
//!
//! 1. Top-level document is OpenAPI 3.1 and advertises the live
//!    [`playtest_api::API_VERSION`].
//! 2. Every route from the wire contract appears in `paths`.
//! 3. Every public type from `playtest-api` appears in
//!    `components.schemas`.
//!
//! When a new endpoint or public type is added these assertions fail
//! until the dump is taught about it — which is the whole point of
//! checking the schema into `docs/openapi.json`.

use playtest_api::API_VERSION;
use playtest_server::openapi_json;

#[test]
fn top_level_is_openapi_3_1_and_api_version_matches() {
    let doc = openapi_json();
    assert_eq!(
        doc.get("openapi").and_then(|v| v.as_str()),
        Some("3.1.0"),
        "expected top-level openapi == \"3.1.0\""
    );
    let info_version = doc
        .get("info")
        .and_then(|i| i.get("version"))
        .and_then(|v| v.as_str())
        .expect("info.version should be a string");
    assert_eq!(info_version, API_VERSION);
}

#[test]
fn every_wire_contract_route_is_present() {
    let doc = openapi_json();
    let paths = doc
        .get("paths")
        .and_then(|p| p.as_object())
        .expect("paths should be an object");

    let required_routes = [
        "/api/health",
        "/api/games-registry",
        "/api/agents-registry",
        "/api/runs",
        "/api/runs/{run_id}",
        "/api/runs/{run_id}/stream",
        "/api/runs/{run_id}/games",
        "/api/runs/{run_id}/games/{game_id}",
        "/api/runs/{run_id}/games/{game_id}/events",
        "/api/runs/{run_id}/games/{game_id}/stream",
        "/api/runs/{run_id}/games/{game_id}/actions",
        "/api/reports",
        "/api/reports/{report_id}",
        "/api/reports/{report_id}/markdown",
    ];
    for route in required_routes {
        assert!(
            paths.contains_key(route),
            "OpenAPI dump missing route {route}"
        );
    }
}

#[test]
fn every_public_api_type_is_in_components_schemas() {
    let doc = openapi_json();
    let schemas = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .expect("components.schemas should be an object");

    // schemars generates a sanitized name per type; `ApiResponse<T>`
    // becomes `ApiResponse_for_AnyValue` when parameterized with
    // `serde_json::Value` (schemars' `JsonSchema` for `Value` reports
    // its schema name as `AnyValue`). We assert on the sanitized name.
    let required_types = [
        "ApiResponse_for_AnyValue",
        "ApiError",
        "ApiErrorCode",
        "CreateRunRequest",
        "RunSummary",
        "RunStatus",
        "GameSummary",
        "GameMetadata",
        "EventPage",
        "LogLineDto",
        "SseFrame",
        "GameRegistryEntry",
        "AgentRegistryEntry",
        "SubmitActionBody",
        "SubmitActionResponse",
    ];
    for ty in required_types {
        assert!(
            schemas.contains_key(ty),
            "OpenAPI dump missing schema for {ty}; available: {:?}",
            schemas.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn run_status_is_an_enum_of_four() {
    let doc = openapi_json();
    let run_status = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.get("RunStatus"))
        .expect("RunStatus schema should be present");

    // schemars 0.8 emits per-variant `oneOf` when each variant carries
    // a doc comment (each entry has `enum: ["Variant"]`). Flatten all
    // those nested `enum` arrays into one list and assert on that.
    let mut names: Vec<String> = Vec::new();
    if let Some(arr) = run_status.get("enum").and_then(|e| e.as_array()) {
        names.extend(arr.iter().filter_map(|v| v.as_str().map(str::to_owned)));
    }
    if let Some(one_of) = run_status.get("oneOf").and_then(|e| e.as_array()) {
        for variant in one_of {
            if let Some(arr) = variant.get("enum").and_then(|e| e.as_array()) {
                names.extend(arr.iter().filter_map(|v| v.as_str().map(str::to_owned)));
            }
        }
    }
    for expected in ["Pending", "Running", "Completed", "Failed"] {
        assert!(
            names.iter().any(|n| n == expected),
            "RunStatus missing variant {expected}; got {names:?}"
        );
    }
}

#[test]
fn health_endpoint_has_example_response() {
    let doc = openapi_json();
    let health_get = doc
        .get("paths")
        .and_then(|p| p.get("/api/health"))
        .and_then(|h| h.get("get"))
        .expect("/api/health GET operation present");
    let example = health_get
        .get("responses")
        .and_then(|r| r.get("200"))
        .and_then(|ok| ok.get("content"))
        .and_then(|c| c.get("application/json"))
        .and_then(|j| j.get("example"))
        .expect("/api/health should ship a 200 example");
    assert_eq!(
        example
            .get("api_version")
            .and_then(|v| v.as_str()),
        Some(API_VERSION)
    );
    assert_eq!(
        example
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("ok")
    );
}

#[test]
fn stream_endpoints_advertise_text_event_stream() {
    let doc = openapi_json();
    for path in [
        "/api/runs/{run_id}/stream",
        "/api/runs/{run_id}/games/{game_id}/stream",
    ] {
        let op = doc
            .get("paths")
            .and_then(|p| p.get(path))
            .and_then(|h| h.get("get"))
            .unwrap_or_else(|| panic!("{path} GET missing"));
        let content = op
            .get("responses")
            .and_then(|r| r.get("200"))
            .and_then(|ok| ok.get("content"))
            .and_then(|c| c.as_object())
            .unwrap_or_else(|| panic!("{path} 200.content missing"));
        assert!(
            content.contains_key("text/event-stream"),
            "{path} should document text/event-stream response"
        );
    }
}

#[test]
fn reports_endpoints_document_501() {
    let doc = openapi_json();
    for path in [
        "/api/reports",
        "/api/reports/{report_id}",
        "/api/reports/{report_id}/markdown",
    ] {
        let p = doc
            .get("paths")
            .and_then(|p| p.get(path))
            .unwrap_or_else(|| panic!("{path} missing"));
        // Could be GET or POST — any method should list a 501.
        let mut saw_501 = false;
        if let Some(obj) = p.as_object() {
            for op in obj.values() {
                if op
                    .get("responses")
                    .and_then(|r| r.as_object())
                    .is_some_and(|r| r.contains_key("501"))
                {
                    saw_501 = true;
                    break;
                }
            }
        }
        assert!(saw_501, "{path} should document a 501 response");
    }
}
