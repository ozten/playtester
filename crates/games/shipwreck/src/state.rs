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

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::{Card, EquipmentCard, EventCard, PlayerCard};
use crate::config::ShipWreckConfig;
use crate::phase::Phase;
use crate::raft::{Raft, SlotId};

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
    /// Seat that played the event card. When the pending event pops,
    /// `current_player` is restored to this value so normal turn-taking
    /// continues from the caster's turn.
    pub initiator: PlayerId,
}

impl PendingEvent {
    /// Construct a pending typhoon resolution, listing the players who
    /// must still respond (Unit 23 resolves this as "every seat,
    /// starting with the initiator, in turn order"; `initiator` is
    /// preserved so `current_player` can be restored when the queue
    /// drains).
    #[must_use]
    pub fn typhoon(remaining: VecDeque<PlayerId>, initiator: PlayerId) -> Self {
        Self {
            kind: PendingEventKind::Typhoon,
            remaining_resolvers: remaining,
            initiator,
        }
    }
}

/// A player card that has been placed on one of this player's raft
/// slots. Carries the slot so the legal-action enumerator and
/// food-consumption bookkeeping can find it without a second lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedPlayerCard {
    pub card: PlayerCard,
    pub slot: SlotId,
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
    /// Player cards currently occupying this player's raft slots. A
    /// slot may hold both a player card (here) and an equipment upgrade
    /// (on `raft.upgrades`) — the two are independent.
    pub played_players: Vec<PlacedPlayerCard>,
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

    /// True if this player already has a player card placed on `slot`.
    #[must_use]
    pub fn has_player_card_on_slot(&self, slot: SlotId) -> bool {
        self.played_players.iter().any(|pp| pp.slot == slot)
    }
}

/// Starting food counter for every seat in Unit 22's scope. Chosen to
/// keep played player cards alive long enough for a Random-vs-Random
/// game to exhaust the wreckage pools (rather than collapsing via
/// starvation as soon as anyone places a card). Tunable constant —
/// revisit if the design-balance signal in `random_self_play` shifts.
pub const STARTING_FOOD_COUNTER: i16 = 6;

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
    /// Equipment pile. `equipment_deck.last()` is the "currently
    /// available" card — the one a player can buy if they can pay for
    /// it. Popped on `BuildEquipment`.
    pub equipment_deck: Vec<EquipmentCard>,
    /// Whose turn it is during `Phase::Play`. During
    /// `Phase::ResolvingEvent` the actor is derived from the top of
    /// `event_resolution_stack` instead.
    pub current_player: PlayerId,
    pub phase: Phase,
    /// Stack of multi-step event resolutions in progress. Empty
    /// during normal play.
    pub event_resolution_stack: Vec<PendingEvent>,
    /// Event cards that have been played and consumed. Carried on
    /// state (rather than a per-player field) because once played an
    /// event card belongs to no one — it's simply out of the deck.
    /// This is public: every event-card play emits `EventCardPlayed`,
    /// so observers can reconstruct this list from the log.
    ///
    /// Determinize reads this to subtract consumed event cards from
    /// the "universe" when resampling opponent hands — otherwise a
    /// played-and-discarded event card would be falsely redealt.
    pub discarded_event_cards: Vec<EventCard>,
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
            equipment_deck: Vec::new(),
            current_player: 0,
            phase: Phase::Setup,
            event_resolution_stack: Vec::new(),
            discarded_event_cards: Vec::new(),
        }
    }

    /// Current "top of deck" equipment card offered for purchase, or
    /// `None` when the pile is exhausted.
    #[must_use]
    pub fn current_equipment(&self) -> Option<EquipmentCard> {
        self.equipment_deck.last().copied()
    }
}
