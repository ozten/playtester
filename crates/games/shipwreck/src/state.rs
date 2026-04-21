//! ShipWreck game state.
//!
//! State is passive data — `apply_event` is the only thing that
//! mutates it, and rules live in `rules.rs` (Unit 22). Public fields
//! rather than accessors: tests, metrics, and `public_view` all need
//! to read the same fields, and private accessors with trivial bodies
//! would just be noise.
//!
//! Shape notes:
//! - `face_up_pools[i]` is the face-up pool for `PlayerId(i)`. Per
//!   `docs/shipwreck.md` face-up cards are per-player, not shared.
//! - `wreckage_deck` is empty after setup; it exists as a field
//!   primarily to support mid-setup snapshots and any future rule that
//!   refills it.
//! - `event_resolution_stack` holds *multi-step* event resolutions
//!   (typhoon). Shark and FlyingFish resolve immediately on play and
//!   never enter this stack.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::PlayerId;
use crate::card::{Card, PlayerCard};
use crate::config::ShipWreckConfig;
use crate::phase::Phase;
use crate::raft::Raft;

/// A multi-step event in progress. Only Typhoon produces one of these
/// today; the enum keeps the shape ready for future multi-step events
/// without forcing a log schema migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingEventKind {
    /// All remaining resolvers must each answer with a
    /// [`crate::action::EventResolution`].
    Typhoon,
}

/// One entry on the `event_resolution_stack`. `remaining_resolvers`
/// drives which player's input is required next; the engine's
/// `next_actor` consults the front of the queue during
/// `Phase::ResolvingEvent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub kind: PendingEventKind,
    pub remaining_resolvers: VecDeque<PlayerId>,
}

impl PendingEvent {
    /// Construct a pending typhoon resolution, listing the players who
    /// must still respond (turn order — usually everyone except the
    /// caster; spec leaves whether the caster is included somewhat
    /// ambiguous, resolved in Unit 23).
    #[must_use]
    pub fn typhoon(remaining: VecDeque<PlayerId>) -> Self {
        Self {
            kind: PendingEventKind::Typhoon,
            remaining_resolvers: remaining,
        }
    }
}

/// Per-seat state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerState {
    /// Cards in hand. Mix of player cards and wreckage cards — player
    /// cards stay here until the player chooses to place them on a
    /// raft slot via `Action::PlacePlayerCard`.
    pub hand: Vec<Card>,
    /// The player's raft (base cards + extensions + installed
    /// upgrades).
    pub raft: Raft,
    /// Player cards that have been placed on raft slots. For Unit 21
    /// this is a flat list; Unit 22 may migrate to a slot map if the
    /// legal-action enumerator needs it. Entries correspond to cards
    /// removed from `hand` by a successful `PlacePlayerCard`.
    pub played_players: Vec<PlayerCard>,
    /// Food reserves carried forward from prior turns (primarily from
    /// RainCatcher / FlyingFish). Kept as `i16` because future spec
    /// refinements may allow transient negative balances during
    /// resolution.
    pub food_counter: i16,
    /// Per-resource counts, indexed by [`crate::resource::Resource::index`].
    pub inventory: [u8; 5],
}

impl PlayerState {
    /// Construct a fresh seat with the given fresh raft, empty hand,
    /// zero food, zero inventory.
    #[must_use]
    pub fn fresh(raft: Raft) -> Self {
        Self {
            hand: Vec::new(),
            raft,
            played_players: Vec::new(),
            food_counter: 0,
            inventory: [0; 5],
        }
    }
}

/// Full state of a ShipWreck game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub config: ShipWreckConfig,
    /// One `PlayerState` per seat, indexed by `PlayerId`.
    pub players: Vec<PlayerState>,
    /// Remaining face-down wreckage cards. Empty after setup; kept as
    /// a field to support mid-setup snapshots and future "redeal"
    /// variants.
    pub wreckage_deck: Vec<Card>,
    /// Per-player face-up pool. `face_up_pools[i]` belongs to
    /// `PlayerId(i)`. Length always equals `config.num_players`.
    pub face_up_pools: Vec<Vec<Card>>,
    /// Whose turn it is during `Phase::Play`. During
    /// `Phase::ResolvingEvent` the actor is derived from the top of
    /// `event_resolution_stack` instead.
    pub current_player: PlayerId,
    pub phase: Phase,
    /// Stack of multi-step event resolutions in progress. Empty
    /// during normal play.
    pub event_resolution_stack: Vec<PendingEvent>,
}

impl GameState {
    /// Build an empty shell of a game state with the correct player
    /// count and empty decks. Primarily a helper for `setup.rs`.
    pub(crate) fn empty_for(config: ShipWreckConfig) -> Self {
        let n = config.num_players as usize;
        let mut players = Vec::with_capacity(n);
        for _ in 0..n {
            // Each player gets fresh base rafts. `crate::pool::base_raft_pair`
            // returns the canonical (left, right) ordering.
            let (left, right) = crate::pool::base_raft_pair();
            players.push(PlayerState::fresh(Raft::new(left, right)));
        }
        let face_up_pools = (0..n).map(|_| Vec::new()).collect();
        Self {
            config,
            players,
            wreckage_deck: Vec::new(),
            face_up_pools,
            current_player: 0,
            phase: Phase::Setup,
            event_resolution_stack: Vec::new(),
        }
    }
}
