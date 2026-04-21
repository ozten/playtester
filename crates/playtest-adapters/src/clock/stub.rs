//! Stub `Clock`: returns a fixed or programmable sequence of times.
//!
//! No OS interaction — the entire point is that unit tests can pin the
//! engine's view of time without touching `SystemTime`.

use playtest_ports::{Clock, UnixMillis};

/// Test double that returns times from a preset sequence, or a fixed
/// value once the sequence runs out.
#[derive(Debug, Clone)]
pub struct StubClock {
    sequence: Vec<UnixMillis>,
    cursor: usize,
    tail: UnixMillis,
}

impl StubClock {
    /// Return `fixed` for every call.
    #[must_use]
    pub fn fixed(fixed: UnixMillis) -> Self {
        Self {
            sequence: Vec::new(),
            cursor: 0,
            tail: fixed,
        }
    }

    /// Return values from `sequence` in order; after exhaustion, keep
    /// returning the last value.
    #[must_use]
    pub fn from_sequence(sequence: Vec<UnixMillis>) -> Self {
        let tail = sequence.last().copied().unwrap_or(0);
        Self {
            sequence,
            cursor: 0,
            tail,
        }
    }
}

impl Clock for StubClock {
    fn now(&mut self) -> UnixMillis {
        if let Some(&v) = self.sequence.get(self.cursor) {
            self.cursor += 1;
            v
        } else {
            self.tail
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_returns_same_value() {
        let mut c = StubClock::fixed(42);
        assert_eq!(c.now(), 42);
        assert_eq!(c.now(), 42);
    }

    #[test]
    fn sequence_returns_values_in_order_then_sticks_at_tail() {
        let mut c = StubClock::from_sequence(vec![1, 2, 3]);
        assert_eq!(c.now(), 1);
        assert_eq!(c.now(), 2);
        assert_eq!(c.now(), 3);
        assert_eq!(c.now(), 3);
    }

    #[test]
    fn empty_sequence_returns_zero() {
        let mut c = StubClock::from_sequence(vec![]);
        assert_eq!(c.now(), 0);
    }
}
