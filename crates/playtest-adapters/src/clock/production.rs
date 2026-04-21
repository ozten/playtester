//! Production `Clock`: wraps `std::time::SystemTime::now()`.
//!
//! This is the **only** place in the workspace that is allowed to call
//! `SystemTime::now()` directly. Any other caller is a determinism bug.

use std::time::{SystemTime, UNIX_EPOCH};

use playtest_ports::{Clock, UnixMillis};

#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionClock;

impl ProductionClock {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for ProductionClock {
    fn now(&mut self) -> UnixMillis {
        // Times before the Unix epoch are not physically possible on any
        // machine that can run this binary; clamp to 0 rather than panic
        // in the unlikely event the system clock is misconfigured.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_clock_returns_monotonically_nondecreasing_values() {
        let mut c = ProductionClock::new();
        let a = c.now();
        let b = c.now();
        assert!(b >= a, "system clock went backwards: {a} -> {b}");
    }
}
