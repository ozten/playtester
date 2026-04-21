//! `ShipWreckPublicView` — the redacted view of game state from one
//! observer's seat.
//!
//! **What's public in ShipWreck** (rationale — most of the information
//! in this game is public in practice):
//! - every player's raft (extensions + installed upgrades)
//! - every player's face-up pool (face-up is visible to all)
//! - every player's `played_players` list (placement onto a raft slot
//!   is a public action)
//! - every player's food counter (food is public — it changes via
//!   public `FoodConsumed` events emitted at `EndTurn`)
//! - every player's inventory (inventory changes via public
//!   `PickedWreckage` / `ResourceSpent` / `BuiltEquipment` events, so
//!   any observer with full event-log access can derive it. Exposing
//!   it simplifies the public view and the determinize invariant)
//! - the current equipment-deck top card
//! - the current player index, phase, and any pending event chains
//! - *the observer's* own hand
//!
//! **What's private**:
//! - other players' hands. Per Unit 22's hidden-information analysis,
//!   hands can contain cards dealt during setup (via
//!   `Event::DealWreckageHand`) that were never publicly revealed. As
//!   soon as a held card is placed / spent / picked, its identity
//!   becomes public via the corresponding event, but the observer
//!   doesn't know *which specific dealt card* any given hand slot
//!   holds until that happens.
//!
//! The `PartialEq`/`Eq` derives are load-bearing: the determinize
//! property test compares public views before and after
//! determinization to verify that only private state was resampled.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::{Card, EquipmentCard, EventCard};
use crate::phase::Phase;
use crate::raft::Raft;
use crate::state::{GameState, PendingEvent, PlacedPlayerCard};

/// One opponent seat as visible to the observer. No private data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpponentView {
    /// Opponent's seat index.
    pub player: PlayerId,
    /// Opaque hand-size. Card identities are not revealed.
    pub hand_size: usize,
    pub raft: Raft,
    pub played_players: Vec<PlacedPlayerCard>,
    pub food_counter: i16,
    pub inventory: [u8; 5],
    pub face_up_pool: Vec<Card>,
}

/// What the observer (`OwnView`) sees about themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnView {
    pub player: PlayerId,
    pub hand: Vec<Card>,
    pub raft: Raft,
    pub played_players: Vec<PlacedPlayerCard>,
    pub food_counter: i16,
    pub inventory: [u8; 5],
    pub face_up_pool: Vec<Card>,
}

/// Full observer-visible snapshot. Every field is derived directly
/// from `GameState` + observer index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipWreckPublicView {
    pub observer: PlayerId,
    pub own: OwnView,
    /// Indexed by `PlayerId`. For `p == observer`, the slot is `None`
    /// (the observer's data lives on `own` instead). `Vec<Option<..>>`
    /// rather than `BTreeMap<PlayerId, OpponentView>` keeps the shape
    /// fixed-length and easy to compare.
    pub opponents: Vec<Option<OpponentView>>,
    pub current_player: PlayerId,
    pub phase: Phase,
    pub current_equipment: Option<EquipmentCard>,
    pub equipment_deck_remaining: usize,
    pub event_resolution_stack: Vec<PendingEvent>,
    /// Remaining cards in the shared face-down wreckage deck. After
    /// setup this is always zero — no mid-game redeal in Unit 22 — but
    /// carrying the count keeps the view forward-compatible.
    pub wreckage_deck_size: usize,
    /// Event cards that have been played and consumed this game (in
    /// play order). Public because every play emits an
    /// `EventCardPlayed` log record.
    pub discarded_event_cards: Vec<EventCard>,
}

/// Build the public view for `observer` from `state`.
#[must_use]
pub fn public_view(state: &GameState, observer: PlayerId) -> ShipWreckPublicView {
    let observer_idx = observer as usize;
    let own = OwnView {
        player: observer,
        hand: state.players[observer_idx].hand.clone(),
        raft: state.players[observer_idx].raft.clone(),
        played_players: state.players[observer_idx].played_players.clone(),
        food_counter: state.players[observer_idx].food_counter,
        inventory: state.players[observer_idx].inventory,
        face_up_pool: state.face_up_pools[observer_idx].clone(),
    };

    let opponents: Vec<Option<OpponentView>> = state
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == observer_idx {
                None
            } else {
                Some(OpponentView {
                    player: u8::try_from(i).expect("seat index fits in u8"),
                    hand_size: p.hand.len(),
                    raft: p.raft.clone(),
                    played_players: p.played_players.clone(),
                    food_counter: p.food_counter,
                    inventory: p.inventory,
                    face_up_pool: state.face_up_pools[i].clone(),
                })
            }
        })
        .collect();

    ShipWreckPublicView {
        observer,
        own,
        opponents,
        current_player: state.current_player,
        phase: state.phase,
        current_equipment: state.current_equipment(),
        equipment_deck_remaining: state.equipment_deck.len(),
        event_resolution_stack: state.event_resolution_stack.clone(),
        wreckage_deck_size: state.wreckage_deck.len(),
        discarded_event_cards: state.discarded_event_cards.clone(),
    }
}
