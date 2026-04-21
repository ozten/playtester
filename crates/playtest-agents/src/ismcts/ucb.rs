//! UCB1 selection for Monte Carlo Tree Search.
//!
//! The Upper Confidence Bound 1 (UCB1) formula balances exploitation
//! (the child's empirical mean reward) against exploration (how
//! many times we've sampled the child relative to its parent):
//!
//! ```text
//! UCB1 = mean + c * sqrt(ln(parent_visits) / child_visits)
//! ```
//!
//! - Unvisited children return `f64::INFINITY` so the selector picks
//!   them on the first pass — every child is sampled at least once
//!   before any gets sampled twice.
//! - `c` is the exploration constant. Classical choice is `sqrt(2)`;
//!   games with sparse rewards or long horizons often benefit from a
//!   slightly larger value (1.4–2.0).

/// UCB1 score for a child at this parent.
///
/// `child_value_mean` is already-divided total_value / visits; `c` is
/// the exploration constant. Returns `f64::INFINITY` for unvisited
/// children so they are always preferred during initial expansion.
#[must_use]
pub fn ucb1(
    child_value_mean: f64,
    child_visits: u32,
    parent_visits: u32,
    c: f64,
) -> f64 {
    if child_visits == 0 {
        return f64::INFINITY;
    }
    let parent = f64::from(parent_visits.max(1));
    let visits = f64::from(child_visits);
    child_value_mean + c * (parent.ln() / visits).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unvisited_child_returns_infinity() {
        let v = ucb1(0.5, 0, 100, std::f64::consts::SQRT_2);
        assert!(v.is_infinite() && v > 0.0);
    }

    #[test]
    fn exploration_term_shrinks_with_more_visits() {
        let few = ucb1(0.5, 1, 100, std::f64::consts::SQRT_2);
        let many = ucb1(0.5, 100, 100, std::f64::consts::SQRT_2);
        assert!(few > many);
    }

    #[test]
    fn higher_mean_beats_lower_mean_at_same_visits() {
        let lo = ucb1(0.2, 10, 100, std::f64::consts::SQRT_2);
        let hi = ucb1(0.8, 10, 100, std::f64::consts::SQRT_2);
        assert!(hi > lo);
    }

    #[test]
    fn higher_c_gives_more_exploration_weight() {
        let cool = ucb1(0.5, 10, 100, 0.5);
        let hot = ucb1(0.5, 10, 100, 3.0);
        assert!(hot > cool);
    }
}
