//! Production `Rng`: `rand_chacha::ChaCha20Rng`, seeded from a `u64`.
//!
//! Portable across platforms; identical output for identical seeds on x86,
//! ARM, and wasm32. Any other RNG source (including `rand::thread_rng`)
//! outside this module is a determinism bug.

use core::ops::Range;

use playtest_ports::{Rng, RngError};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

#[derive(Debug)]
pub struct ProductionRng {
    inner: ChaCha20Rng,
}

impl ProductionRng {
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self {
            inner: ChaCha20Rng::seed_from_u64(seed),
        }
    }
}

impl Rng for ProductionRng {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError> {
        if range.start >= range.end {
            return Err(RngError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        let span = range.end - range.start;
        Ok(range.start + self.inner.next_u64() % span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_stream() {
        let mut a = ProductionRng::from_seed(12345);
        let mut b = ProductionRng::from_seed(12345);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = ProductionRng::from_seed(1);
        let mut b = ProductionRng::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
