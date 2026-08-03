//! Determinization for Great Gyre — placeholder.
//!
//! **Scope note**: search agents (ISMCTS) aren't wired up until Unit
//! 5, and the real hidden-information resampling depends on the
//! redacted `public_view` (also Unit 5). Until then, `determinize` is
//! the identity clone — matching `Game::determinize`'s documented
//! escape hatch ("games with no hidden information can return
//! `state.clone()`"). This is a deliberate placeholder, not a claim
//! that Great Gyre has no hidden information (it does: opponent hands
//! and face-down Current cards) — Unit 5 replaces this with a real
//! resample and adds the `public_view(determinize(..)) ==
//! public_view(..)` property test.

use playtest_core::PlayerId;
use playtest_ports::Rng;

use crate::state::GameState;

pub(crate) fn determinize(state: &GameState, _observer: PlayerId, _rng: &mut dyn Rng) -> GameState {
    state.clone()
}
