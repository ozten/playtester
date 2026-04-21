//! Stub `Rng`: either returns a preset sequence of `u64` values (and
//! falls through to a seeded ChaCha20 once exhausted) or is seeded
//! directly. Deterministic either way.

use core::ops::Range;

use playtest_ports::{Rng, RngError};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

#[derive(Debug)]
pub struct StubRng {
    sequence: Vec<u64>,
    cursor: usize,
    fallback: ChaCha20Rng,
}

impl StubRng {
    /// Seed the fallback ChaCha20 directly; no preset sequence.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self {
            sequence: Vec::new(),
            cursor: 0,
            fallback: ChaCha20Rng::seed_from_u64(seed),
        }
    }

    /// Return values from `sequence` in order for `next_u64`; once
    /// exhausted, fall through to a ChaCha20 seeded with `fallback_seed`.
    /// `gen_range` always uses the fallback to avoid biasing over a
    /// fixed-width sequence.
    #[must_use]
    pub fn with_sequence(sequence: Vec<u64>, fallback_seed: u64) -> Self {
        Self {
            sequence,
            cursor: 0,
            fallback: ChaCha20Rng::seed_from_u64(fallback_seed),
        }
    }
}

impl Rng for StubRng {
    fn next_u64(&mut self) -> u64 {
        if let Some(&v) = self.sequence.get(self.cursor) {
            self.cursor += 1;
            v
        } else {
            self.fallback.next_u64()
        }
    }

    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError> {
        if range.start >= range.end {
            return Err(RngError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        let span = range.end - range.start;
        Ok(range.start + self.fallback.next_u64() % span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_is_deterministic() {
        let mut a = StubRng::seeded(42);
        let mut b = StubRng::seeded(42);
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn sequence_returns_preset_values_then_falls_through() {
        let mut r = StubRng::with_sequence(vec![100, 200, 300], 42);
        assert_eq!(r.next_u64(), 100);
        assert_eq!(r.next_u64(), 200);
        assert_eq!(r.next_u64(), 300);
        let a = r.next_u64();
        let b = r.next_u64();
        assert_ne!(a, b);
    }

    #[test]
    fn gen_range_rejects_empty_range() {
        let mut r = StubRng::seeded(42);
        let err = r.gen_range(5..5).unwrap_err();
        assert!(matches!(err, RngError::InvalidRange { .. }));
    }

    #[test]
    fn gen_range_respects_bounds() {
        let mut r = StubRng::seeded(42);
        for _ in 0..100 {
            let v = r.gen_range(10..20).unwrap();
            assert!((10..20).contains(&v));
        }
    }
}
