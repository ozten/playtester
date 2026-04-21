//! Card types for ShipWreck.
//!
//! [`Card`] is the top-level tagged enum — every shuffle-able and
//! player-hand-able thing in the game is a [`Card`]. Each variant
//! carries a small payload struct; the enum is serde-tagged so the
//! JSONL event log reads naturally (`{"kind":"Player","id":"Professor",...}`
//! rather than an opaque positional form).
//!
//! Kind inventory (all sourced from `docs/shipwreck.md`):
//! - `Player` — the seven named castaways (Professor, MaryAnne, …)
//! - `BaseRaft` — one of two fixed slots at each end of a raft
//! - `RaftExtension` — inserted between base-left and base-right
//! - `Equipment` — built by spending resources onto a raft slot
//! - `Item` — a single wreckage resource card
//! - `Event` — Shark, Typhoon, FlyingFish
//!
//! Pool quantities are in [`crate::pool`]; cost tables are driven by
//! the [`EquipmentKind`] enum so tests can enumerate them without
//! magic-string comparisons.

use serde::{Deserialize, Serialize};

use crate::resource::{Resource, ResourceCost};

/// A named player-character card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlayerCardId {
    Professor,
    MaryAnne,
    MovieStar,
    Gillagun,
    Millionaire,
    MillionHeiress,
    Wilson,
}

/// Per-character unique skill. Spec sections map as follows:
/// - Professor → [`PlayerSkill::ConstructWithOneFewerResource`]
/// - MaryAnne  → [`PlayerSkill::ExtraReachOne`]
/// - Gillagun  → [`PlayerSkill::FreeSkipFoodOncePerGame`]
///
/// Other characters have no skill (represented as
/// `skill: None` on [`PlayerCard`]); keeping this enum small avoids a
/// no-op "NoSkill" variant that no test would want to match against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerSkill {
    /// Can construct an equipment card with one fewer resource of any
    /// single type (Professor).
    ConstructWithOneFewerResource,
    /// Can reach one extra space when picking wreckage (MaryAnne).
    ExtraReachOne,
    /// Can skip paying food once per game (Gillagun).
    FreeSkipFoodOncePerGame,
}

/// A named castaway — rescue-point worth, per-turn food cost, optional
/// skill. Data matches `docs/shipwreck.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerCard {
    pub id: PlayerCardId,
    pub rescue_points: u8,
    pub food_cost: u8,
    pub skill: Option<PlayerSkill>,
}

impl PlayerCard {
    #[must_use]
    pub const fn new(
        id: PlayerCardId,
        rescue_points: u8,
        food_cost: u8,
        skill: Option<PlayerSkill>,
    ) -> Self {
        Self {
            id,
            rescue_points,
            food_cost,
            skill,
        }
    }
}

/// Base raft side — one lives at each end of every raft.
///
/// Both bases are structurally identical (each has one upgrade slot),
/// but naming the side makes [`SlotId::BaseLeft`] / [`SlotId::BaseRight`]
/// references unambiguous in tests and in the event log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BaseRaftSide {
    Left,
    Right,
}

/// A base-raft card. Carries its side so the log can identify which
/// end of the raft it sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BaseRaftCard {
    pub side: BaseRaftSide,
}

impl BaseRaftCard {
    #[must_use]
    pub const fn new(side: BaseRaftSide) -> Self {
        Self { side }
    }
}

/// A raft-extension card. Extensions are interchangeable (`docs/shipwreck.md`
/// calls out "say 40 in the deck"); we carry a `serial` so the event
/// log can disambiguate which physical card was placed where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RaftExtensionCard {
    pub serial: u16,
}

impl RaftExtensionCard {
    #[must_use]
    pub const fn new(serial: u16) -> Self {
        Self { serial }
    }
}

/// One of the five equipment kinds. Costs and effects are captured
/// on the kind; [`EquipmentCard`] carries a `serial` so duplicates of
/// the same kind are distinguishable in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EquipmentKind {
    /// Gives 2 food per round. Cost: 1 wood, 1 rope, 1 wire.
    FishingPoles,
    /// Reach one extra space when picking wreckage. Cost: 2 wood, 1 plastic, 1 wire.
    Telescope,
    /// Grab two wreckage cards each turn. Cost: 1 cloth, 1 rope, 1 wood.
    AutoNets,
    /// Defends against a shark attack. Cost: 2 wire, 1 plastic.
    SteelCordage,
    /// Gives 1 food per round. Cost: 2 cloth, 1 wood.
    RainCatcher,
}

impl EquipmentKind {
    /// Build cost of this equipment. Indexed by [`Resource::index`].
    #[must_use]
    pub const fn cost(self) -> ResourceCost {
        // Index order: [Plastic, Wood, Rope, Cloth, Wire]
        match self {
            Self::FishingPoles => ResourceCost::new([0, 1, 1, 0, 1]),
            Self::Telescope => ResourceCost::new([1, 2, 0, 0, 1]),
            Self::AutoNets => ResourceCost::new([0, 1, 1, 1, 0]),
            Self::SteelCordage => ResourceCost::new([1, 0, 0, 0, 2]),
            Self::RainCatcher => ResourceCost::new([0, 1, 0, 2, 0]),
        }
    }
}

/// A concrete equipment card. `kind` determines cost and effect;
/// `serial` distinguishes duplicates in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EquipmentCard {
    pub kind: EquipmentKind,
    pub serial: u16,
}

impl EquipmentCard {
    #[must_use]
    pub const fn new(kind: EquipmentKind, serial: u16) -> Self {
        Self { kind, serial }
    }

    /// Forward to [`EquipmentKind::cost`].
    #[must_use]
    pub const fn cost(&self) -> ResourceCost {
        self.kind.cost()
    }
}

/// A single wreckage resource card. Thin wrapper around [`Resource`]
/// so that `Card::Item(ItemCard(Resource::Wood))` reads naturally and
/// the serde tag is consistent across variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemCard(pub Resource);

impl ItemCard {
    #[must_use]
    pub const fn new(resource: Resource) -> Self {
        Self(resource)
    }

    #[must_use]
    pub const fn resource(self) -> Resource {
        self.0
    }
}

/// Event cards disrupt the normal turn flow. Resolution logic lives in
/// later units; this unit only models the cards themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCard {
    /// Choose a player and one of their extensions/upgrades to destroy.
    /// Steel-cordage defends and is destroyed instead.
    Shark,
    /// All players lose one extension or upgrade.
    Typhoon,
    /// Treat as extra food for one turn.
    FlyingFish,
}

/// Top-level card enum. Serde-tagged so JSONL event payloads are
/// self-describing (`{"card_type":"Event",...}` rather than
/// positional). The tag name is `card_type` — not the more obvious
/// `kind` — because [`EquipmentCard`] already has a `kind` field
/// (discriminating Telescope/AutoNets/…), and using the same tag name
/// at two nesting levels causes a serde duplicate-field error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "card_type")]
pub enum Card {
    Player(PlayerCard),
    BaseRaft(BaseRaftCard),
    RaftExtension(RaftExtensionCard),
    Equipment(EquipmentCard),
    Item(ItemCard),
    Event(EventCard),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_cost_telescope_matches_spec() {
        let cost = EquipmentKind::Telescope.cost();
        // 2 wood + 1 plastic + 1 wire — indices 1, 0, 4
        assert_eq!(cost.amount_of(Resource::Plastic), 1);
        assert_eq!(cost.amount_of(Resource::Wood), 2);
        assert_eq!(cost.amount_of(Resource::Wire), 1);
        assert_eq!(cost.amount_of(Resource::Rope), 0);
        assert_eq!(cost.amount_of(Resource::Cloth), 0);
    }

    #[test]
    fn item_card_round_trips_through_resource() {
        for r in Resource::ALL {
            let item = ItemCard::new(r);
            assert_eq!(item.resource(), r);
        }
    }
}
