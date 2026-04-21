//! Response envelope: every HTTP response is wrapped so `api_version`
//! is always present and error shapes are uniform.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{API_VERSION, error::ApiError};

/// Uniform envelope wrapping every JSON response the server emits.
///
/// The envelope guarantees two things to consumers:
///
/// 1. `api_version` is always present — clients can refuse to parse
///    a response whose major version differs from what they compiled
///    against.
/// 2. Errors share one shape across every endpoint — success is
///    `data: Some(..), errors: []`, failure is `data: None, errors:
///    [..]`. Partial success (e.g. batch validation) is
///    `data: Some(..), errors: [..]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApiResponse<T> {
    /// The wire-contract version this response conforms to.
    /// Always equal to [`crate::API_VERSION`] at send time.
    pub api_version: String,

    /// The endpoint-specific payload, if the request succeeded (fully
    /// or partially).
    #[serde(default = "none::<T>")]
    pub data: Option<T>,

    /// Zero or more errors. Empty on full success; populated on
    /// partial success or outright failure.
    #[serde(default = "empty_errors")]
    pub errors: Vec<ApiError>,
}

// Explicit default fns so we do not require `T: Default`.
fn none<T>() -> Option<T> {
    None
}

fn empty_errors() -> Vec<ApiError> {
    Vec::new()
}

impl<T> ApiResponse<T> {
    /// Envelope for a fully successful response.
    #[must_use]
    pub fn ok(data: T) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            data: Some(data),
            errors: Vec::new(),
        }
    }

    /// Envelope for a failed response (no data, one or more errors).
    #[must_use]
    pub fn fail(errors: Vec<ApiError>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            data: None,
            errors,
        }
    }

    /// Envelope for a partially successful response (some data plus
    /// one or more errors, e.g. batch validation).
    #[must_use]
    pub fn partial(data: T, errors: Vec<ApiError>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            data: Some(data),
            errors,
        }
    }
}
