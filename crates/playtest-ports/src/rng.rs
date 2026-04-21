//! RNG port: the only source of randomness inside the engine.
//!
//! Every stochastic game decision — deck shuffles, the starter cut, any
//! random tie-break inside an agent — must flow through this port. Any
//! direct use of `rand::random()`, `thread_rng()`, or platform-dependent
//! RNG outside of the `production` adapter is a determinism bug.

use core::ops::Range;

/// Errors produced by the [`Rng`] port.
#[derive(Debug, thiserror::Error)]
pub enum RngError {
    /// `gen_range` was called with a range where `start >= end`. This is a
    /// caller bug; the adapter should refuse rather than panic so the game
    /// loop can surface a clean error with tick context.
    #[error("invalid range for gen_range: start ({start}) >= end ({end})")]
    InvalidRange { start: u64, end: u64 },
}

/// A source of pseudorandom numbers.
///
/// Object-safe: `&mut dyn Rng` works. The generic `shuffle` method is
/// excluded from the vtable via `where Self: Sized` so it does not break
/// object-safety but is still available on concrete impls.
///
/// Adapter variants that must exist for this port:
/// - `stub` — deterministic programmable sequence for unit tests.
/// - `production` — `rand_chacha::ChaCha20Rng`, portable across platforms.
/// - `record` — wraps another RNG, tees every `(call, output)` pair to a tape.
/// - `playback` — reads a tape and returns stored outputs in order; panics on
///   call-pattern divergence, which is the whole test signal for
///   non-determinism.
pub trait Rng {
    /// Return a uniformly random `u64`.
    fn next_u64(&mut self) -> u64;

    /// Return a uniformly random `u64` in `[range.start, range.end)`.
    ///
    /// Returns [`RngError::InvalidRange`] if `range.start >= range.end`.
    /// Adapters must never panic on an empty range.
    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError>;

    /// Fisher-Yates shuffle, implemented on top of [`Self::gen_range`].
    ///
    /// The `where Self: Sized` bound keeps the [`Rng`] trait object-safe.
    /// In practice shuffle is always called on a concrete RNG, never via
    /// `&mut dyn Rng`, so this costs nothing.
    fn shuffle<T>(&mut self, slice: &mut [T])
    where
        Self: Sized,
    {
        let n = slice.len();
        if n < 2 {
            return;
        }
        let mut i = n - 1;
        while i > 0 {
            let upper = u64::try_from(i).expect("usize fits in u64") + 1;
            let j_u64 = self
                .gen_range(0..upper)
                .expect("0..(i+1) is never empty for i > 0");
            // `j_u64 < upper <= usize::MAX + 1`, so this fits by construction.
            let j = usize::try_from(j_u64).expect("j <= i fits in usize");
            slice.swap(i, j);
            i -= 1;
        }
    }
}
