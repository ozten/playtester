//! Log-record events for ShipWreck.
//!
//! The name clash is real: "event" means two different things in
//! ShipWreck. This module owns the **log records** emitted by
//! `apply_action` / `resolve_chance` — the JSONL event stream that
//! replays the game's history. The *in-game* event cards (Shark,
//! Typhoon, FlyingFish) live in [`crate::card::EventCard`]; when one
//! of those cards is played, the action produces several log `Event`s
//! here (`EventCardPlayed`, `EventResolved`, …).
//!
//! Every variant derives [`Serialize`] so `playtest-log` can write it
//! to disk without any bespoke conversion. `Deserialize` is derived so
//! replay tools can read the log back into memory with the same types.

use serde::{Deserialize, Serialize};

use crate::action::{EventCardKind, EventTarget};
use crate::card::{Card, EquipmentKind, PlayerCardId};
use crate::raft::SlotId;
use crate::resource::Resource;
use crate::{EndReason, PlayerId};

/// Outcome of resolving an in-game event card. Each enum variant names
/// the mechanical result so metrics can count "how many sharks were
/// defended" vs. "how many landed" without re-running the rule logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventOutcome {
    /// Shark attack succeeded; the targeted slot's contents were destroyed.
    SharkDestroyed { target: PlayerId, slot: SlotId },
    /// Shark attack defended; the target's SteelCordage was consumed
    /// instead of the originally-targeted slot.
    SharkDefended { target: PlayerId },
    /// Player sacrificed a specific slot to a Typhoon.
    TyphoonLost { player: PlayerId, slot: SlotId },
    /// Player had nothing to lose to a Typhoon (base rafts are not
    /// forfeitable per `docs/shipwreck.md`).
    TyphoonPass { player: PlayerId },
    /// FlyingFish granted one unit of food to the casting player.
    FlyingFishGranted { player: PlayerId },
}

/// Per-player final score. Captures the primary score (`rescue_points`)
/// and both tie-breakers so downstream metrics can read winner *and*
/// margin without re-deriving the ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerScore {
    pub player: PlayerId,
    pub rescue_points: u16,
    /// Raft length (base cards + extensions). First tie-breaker.
    pub raft_length: u16,
    /// Number of equipment inventions installed. Second tie-breaker.
    pub invention_count: u16,
}

/// Every observable change to a ShipWreck game. `apply_event` folds
/// one of these into `GameState`; the log persists the full sequence.
///
/// Setup events (first three variants) are chance events — emitted by
/// the engine during initial deal, not by any agent action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Chance: a player card was dealt to `player`.
    DealPlayerCard { player: PlayerId, card: PlayerCardId },

    /// Chance: six wreckage cards were dealt to `player`'s hand.
    /// Emitted once per player during setup.
    DealWreckageHand { player: PlayerId, cards: Vec<Card> },

    /// Chance: one wreckage card was dealt face-up to `player`'s pool.
    /// Emitted repeatedly during the round-robin face-up distribution.
    DealWreckageFaceUp { player: PlayerId, card: Card },

    /// An agent picked a specific face-up card into their hand.
    PickedWreckage {
        player: PlayerId,
        from_pool: PlayerId,
        card: Card,
    },

    /// A player card moved from hand onto a raft slot.
    PlacedPlayerCard {
        player: PlayerId,
        card: PlayerCardId,
        slot: SlotId,
    },

    /// A raft-extension card from hand was installed after
    /// `insert_after`. `extension_serial` identifies which physical
    /// extension card was placed (matches
    /// [`crate::card::RaftExtensionCard::serial`]).
    ExtendedRaft {
        player: PlayerId,
        extension_serial: u16,
        insert_after: SlotId,
    },

    /// Equipment was built on `slot`. Resource costs are emitted as
    /// separate [`Event::ResourceSpent`] events immediately before
    /// this one, so `apply_event` can decrement inventory without
    /// needing the cost table.
    BuiltEquipment {
        player: PlayerId,
        equipment_kind: EquipmentKind,
        slot: SlotId,
    },

    /// Resources left a player's inventory (equipment construction).
    ResourceSpent {
        player: PlayerId,
        resource: Resource,
        amount: u8,
    },

    /// An event card was played. Resolution happens via subsequent
    /// [`Event::EventResolved`] event(s), possibly after intervening
    /// [`crate::action::Action::ResolveEvent`] actions from other
    /// players (typhoon).
    EventCardPlayed {
        player: PlayerId,
        card: EventCardKind,
        target: EventTarget,
    },

    /// One step of event resolution produced its mechanical outcome.
    /// For shark/flying-fish there is exactly one such event per
    /// `EventCardPlayed`; for typhoon there is one per player who
    /// responded.
    EventResolved {
        player: PlayerId,
        outcome: EventOutcome,
    },

    /// End-of-turn food consumption. `starved` is true if the player
    /// could not pay the food cost and a placed player card was
    /// discarded as a result.
    FoodConsumed { player: PlayerId, starved: bool },

    /// Player ended their turn — emitted once per turn, after any
    /// food-consumption events.
    EndTurn { player: PlayerId },

    /// The game ended. No further events follow this one.
    EndGame {
        winner: Option<PlayerId>,
        reason: EndReason,
        final_scores: Vec<PlayerScore>,
    },
}
