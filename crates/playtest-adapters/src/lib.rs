//! Adapter implementations for every port in `playtest-ports`.
//!
//! Four variants per port:
//! - `stub` — deterministic, hardcoded behavior for unit tests
//! - `production` — real implementation (`std::fs`, `ChaCha20Rng`, `SystemTime`, etc.)
//! - `record` — wraps another adapter, tees inputs/outputs to a tape file
//! - `playback` — reads a tape file and replays stored outputs
