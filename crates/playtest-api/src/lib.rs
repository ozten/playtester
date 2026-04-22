//! Wire types for the Playtester HTTP + Server-Sent Events API.
//!
//! This crate is intentionally minimal: it contains only the request,
//! response, and SSE frame shapes that cross the boundary between
//! `playtest-server` (Rust) and the SvelteKit frontend (TypeScript).
//! It has **zero** dependency on any other workspace crate — the
//! frontend can consume a frozen wire contract without dragging
//! engine code along.
//!
//! # Design
//!
//! - Every response is wrapped in [`ApiResponse<T>`](version::ApiResponse)
//!   so the `api_version` field is always present and error shapes are
//!   uniform.
//! - Game-specific `Config` blobs and per-tick event payloads are
//!   carried as [`serde_json::Value`] so the API crate stays
//!   game-agnostic.
//! - Every public type derives [`schemars::JsonSchema`] so the server
//!   can emit an OpenAPI 3.1 spec for the TypeScript consumer.
//!
//! # Versioning
//!
//! [`API_VERSION`] is the single source of truth for the wire contract
//! version. Bump the major number on any breaking change to the JSON
//! shape; bump the minor for additive changes that existing consumers
//! can ignore.

pub mod error;
pub mod games;
pub mod registry;
pub mod runs;
pub mod sse;
pub mod version;

pub use error::{ApiError, ApiErrorCode, http_status};
pub use games::{EventPage, GameMetadata, GameSummary, LogLineDto};
pub use registry::{AgentRegistryEntry, GameRegistryEntry};
pub use runs::{CreateRunRequest, RunStatus, RunSummary};
pub use sse::{SseFrame, TurnPromptPayload};
pub use version::ApiResponse;

/// Current wire-contract version. Bumped on any breaking change to the
/// JSON shape produced or accepted by the server.
///
/// `1.1.0` — Phase 2.5: additive introduction of the `http-remote`
/// agent kind, `SseFrame::TurnPrompt`, and `POST /api/runs/{run_id}/
/// games/{game_id}/actions`. Clients built against `1.0.0` and
/// tolerant of unknown fields continue to work.
pub const API_VERSION: &str = "1.1.0";
