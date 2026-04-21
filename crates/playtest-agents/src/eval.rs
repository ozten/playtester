//! Evaluation function type for one-ply agents.
//!
//! `EvalFn<G>` is a plain function pointer rather than a trait object.
//! Rationale (from the Unit 25 plan):
//!
//! > Plain functions, not a trait. Agents don't compose evaluation
//! > functions at runtime; they're picked at construction. A trait
//! > would add dispatch cost without adding expressiveness.
//!
//! Convention: higher return value = better for `player`. The same
//! sign convention across every game's eval function. `GreedyAgent`
//! takes argmax over this value.

use playtest_core::{Game, PlayerId};

/// An evaluation function for a game `G`.
///
/// Takes a `PublicView` (hidden-info-respecting) and the player we're
/// scoring for; returns a scalar where higher = better for `player`.
///
/// Function pointer (not a trait object) so the call is direct — no
/// vtable dispatch on the hot path inside `GreedyAgent::choose`.
pub type EvalFn<G> =
    fn(view: &<G as Game>::PublicView, player: PlayerId) -> f64;
