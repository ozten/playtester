//! ShipWreck — castaways on a raft salvage wreckage for rescue points.
//!
//! Players start with a two-card base raft and try to collect drifting
//! wreckage: food, resources, raft extensions, and equipment upgrades.
//! Player cards (placed on raft slots) score "rescue points" at game
//! end. Resources (plastic, wood, rope, cloth, wire) are spent to build
//! equipment upgrades. Event cards (shark, typhoon, flying fish)
//! interrupt the turn flow.
//!
//! Unit 20 shipped the atomic primitives (cards, card pool, raft,
//! resources). Unit 21 (this unit) adds the state machine types and
//! the setup/deal flow. No `Game` trait implementation yet — that
//! arrives in Unit 22.
//!
//! See `docs/shipwreck.md` for the full game spec.

pub mod action;
pub mod card;
pub mod config;
pub mod event;
pub mod phase;
pub mod pool;
pub mod raft;
pub mod resource;
pub mod state;

#[doc(hidden)]
pub mod setup;

// Local re-definitions of two `playtest-core` types. Unit 21 must not
// depend on `playtest-core` (the `Game` trait impl lands in Unit 22),
// but `PlayerId` and `EndReason` are needed on state/action/event
// signatures. Defined identically in shape here so Unit 22's type
// aliases can swap them out (or this crate can grow the dep at that
// point) without a schema migration.

/// Zero-based player index. Identical shape to
/// `playtest_core::PlayerId`.
pub type PlayerId = u8;

/// Why a game ended. Identical shape to `playtest_core::EndReason`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EndReason {
    Victory,
    Draw,
    Stalemate,
    Other(String),
}

pub use action::{Action, EventCardKind, EventResolution, EventTarget};
pub use card::{
    BaseRaftCard, Card, EquipmentCard, EquipmentKind, EventCard, ItemCard, PlayerCard,
    PlayerCardId, PlayerSkill, RaftExtensionCard,
};
pub use config::{ConfigError, MAX_PLAYERS, MIN_PLAYERS, ShipWreckConfig};
pub use event::{Event, EventOutcome, PlayerScore};
pub use phase::Phase;
pub use pool::{
    DEFAULT_FLYING_FISH_COUNT, DEFAULT_SHARK_COUNT, DEFAULT_TYPHOON_COUNT, ITEM_COUNT_PER_RESOURCE,
    RAFT_EXTENSION_COUNT, all_equipment, all_player_cards, all_wreckage_cards,
};
pub use raft::{Raft, RaftError, SlotId};
pub use resource::{InsufficientResources, Resource, ResourceCost};
pub use state::{GameState, PendingEvent, PendingEventKind, PlayerState};
