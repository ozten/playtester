//! ShipWreck — castaways on a raft salvage wreckage for rescue points.
//!
//! Players start with a two-card base raft and try to collect drifting
//! wreckage: food, resources, raft extensions, and equipment upgrades.
//! Player cards (placed on raft slots) score "rescue points" at game
//! end. Resources (plastic, wood, rope, cloth, wire) are spent to build
//! equipment upgrades. Event cards (shark, typhoon, flying fish)
//! interrupt the turn flow.
//!
//! This unit ships the atomic primitives only — cards, card pool,
//! raft structure, resource accounting. No `Game` trait implementation
//! yet (that arrives in Unit 22). Every later ShipWreck unit depends on
//! this module being exhaustively correct.
//!
//! See `docs/shipwreck.md` for the full game spec.

pub mod card;
pub mod pool;
pub mod raft;
pub mod resource;

pub use card::{
    BaseRaftCard, Card, EquipmentCard, EquipmentKind, EventCard, ItemCard, PlayerCard,
    PlayerCardId, PlayerSkill, RaftExtensionCard,
};
pub use pool::{
    DEFAULT_FLYING_FISH_COUNT, DEFAULT_SHARK_COUNT, DEFAULT_TYPHOON_COUNT, ITEM_COUNT_PER_RESOURCE,
    RAFT_EXTENSION_COUNT, all_equipment, all_player_cards, all_wreckage_cards,
};
pub use raft::{Raft, RaftError, SlotId};
pub use resource::{InsufficientResources, Resource, ResourceCost};
