//! The `Game` trait: the game-agnostic contract the harness runs against.
//!
//! A `Game` is a pure description of rules. It does not own state —
//! state is an associated type threaded through by the engine. Games
//! are stateless strategies; state is data.
//!
//! ## The effect-free event model
//!
//! Agents choose `Action`s. `apply_action` converts an action into a
//! list of `Event`s (one action can produce multiple observable events:
//! "discard two cards" may produce `DiscardToCrib` followed by
//! `DrawReplacement`). Events are the unit that is serialized, logged,
//! and replayed. `apply_event` folds an event into state and must
//! never fail — all validation happens in `apply_action`.
//!
//! `snapshot@tick_N = apply_event folded over events[0..N]`, from the
//! initial seeded state. No separate snapshot serialization.

use serde::Serialize;

use crate::actor::{Actor, PlayerId};
use crate::error::GameError;
use crate::result::GameResult;

/// The game-agnostic rules contract. Implementations are stateless:
/// state lives in the associated [`Game::State`] type.
pub trait Game {
    /// Full game state. Produced by [`Self::initial_state`], evolved by
    /// [`Self::apply_event`]. Not required to serialize — snapshots are
    /// replayed from events, not persisted.
    type State;

    /// An agent's chosen move. Compared by equality in `legal_actions`
    /// so must be `PartialEq`. Need not serialize — only events make it
    /// to the log.
    type Action: Clone + PartialEq;

    /// An atomic observable change to the game. This is the serialized
    /// unit — the `Serialize` bound lets the engine emit events to the
    /// [`GameEventSink`](playtest_ports::GameEventSink) without a
    /// per-game formatter.
    type Event: Clone + Serialize;

    /// What an agent sees when it's their turn. Typically a redacted
    /// view that hides opponents' private cards. No serialize bound —
    /// public views are passed to in-process agents, not persisted.
    type PublicView;

    /// Game-specific configuration (variant rules, number of rounds,
    /// etc.). Threaded into [`Self::initial_state`] by the caller.
    type Config;

    /// Produce the initial state from a seed and config. `seed` drives
    /// any pre-loop randomness the game wants to bake into its initial
    /// state; all in-loop randomness flows through the `Rng` port.
    fn initial_state(&self, seed: u64, cfg: &Self::Config) -> Self::State;

    /// Whose turn is it? The engine uses this to route between agent
    /// prompts and chance resolution.
    fn next_actor(&self, state: &Self::State) -> Actor;

    /// All legal moves for `player` in `state`. Agents return an index
    /// into this slice (see `Agent::choose`), so the slice's ordering
    /// is part of the game's public contract for that turn.
    fn legal_actions(&self, state: &Self::State, player: PlayerId) -> Vec<Self::Action>;

    /// Validate an action and convert it into the events it produces.
    /// This is the only validation point — `apply_event` assumes events
    /// are already legal.
    ///
    /// # Errors
    /// Return [`GameError::IllegalAction`] if the rules reject the action.
    fn apply_action(
        &self,
        state: &Self::State,
        player: PlayerId,
        action: &Self::Action,
    ) -> Result<Vec<Self::Event>, GameError>;

    /// Resolve a chance event (e.g., shuffle, dice) using the supplied
    /// RNG port. Returns the single event that represents what just
    /// happened. Called only when [`Self::next_actor`] returned
    /// [`Actor::Chance`].
    ///
    /// # Errors
    /// Return [`GameError::ChanceFailed`] if the game cannot resolve the
    /// chance step (should be rare — mostly for malformed state).
    fn resolve_chance(
        &self,
        state: &Self::State,
        rng: &mut dyn playtest_ports::Rng,
    ) -> Result<Self::Event, GameError>;

    /// Fold an event into state. Deliberately infallible: events are
    /// already validated by the time they reach this method.
    ///
    /// Takes `&mut State` rather than `State -> State` to avoid forcing
    /// every game's state to be `Default` or wrapped in `Option`; the
    /// semantics are identical.
    fn apply_event(&self, state: &mut Self::State, event: &Self::Event);

    /// Redacted view of state for a given player.
    fn public_view(&self, state: &Self::State, player: PlayerId) -> Self::PublicView;

    /// Produce a determinized state: a concrete sample consistent with
    /// everything `observer` knows. Hidden information (opponent hands,
    /// undealt deck, cards the observer hasn't seen) is resampled from
    /// the unknown pool using `rng`; public state is copied verbatim.
    /// Called by search algorithms (e.g. ISMCTS) that need to simulate
    /// forward from the observer's epistemic position.
    ///
    /// The invariant every implementation must satisfy:
    /// `public_view(determinize(s, p, rng), p) == public_view(s, p)`.
    /// Games with no hidden information can return `state.clone()`.
    fn determinize(
        &self,
        state: &Self::State,
        observer: PlayerId,
        rng: &mut dyn playtest_ports::Rng,
    ) -> Self::State;

    /// Has the game ended? `Some` means yes and provides the result;
    /// `None` means the loop should continue.
    fn game_over(&self, state: &Self::State) -> Option<GameResult>;
}
