//! Clock port: the only source of wall-clock time inside the engine.
//!
//! Direct `std::time::SystemTime::now()` calls outside of the `production`
//! adapter are a determinism bug — they make game timing non-reproducible
//! across replays. All timestamps embedded in the event log come from here.

/// Unix epoch milliseconds. 64 bits buys us ~584 million years of headroom.
pub type UnixMillis = u64;

/// A source of wall-clock time.
///
/// Adapter variants that must exist for this port:
/// - `stub` — returns a fixed or programmable fake time, no interaction with
///   the OS clock. Used by unit tests.
/// - `production` — wraps `std::time::SystemTime::now()`.
/// - `record` — wraps another clock; tees every call and its result to a tape.
/// - `playback` — reads a tape and returns the stored values in order.
///
/// `&mut self` (rather than `&self`) is intentional: record/playback adapters
/// mutate their tape cursor on every call, and we want one trait signature
/// across all four variants.
pub trait Clock {
    /// Return the current wall-clock time in Unix epoch milliseconds.
    fn now(&mut self) -> UnixMillis;
}
