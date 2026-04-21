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
//! resources). Unit 21 added the state machine types and the
//! setup/deal flow. Unit 22 (this unit) lands the `Game` trait impl
//! (`ShipWreckGame`) *minus* event-card resolution, plus determinize
//! and a public view.
//!
//! See `docs/shipwreck.md` for the full game spec.

pub mod action;
pub mod card;
pub mod config;
pub mod determinize;
pub mod event;
pub(crate) mod events;
pub mod phase;
pub mod pool;
pub mod public_view;
pub mod raft;
pub mod resource;
pub mod rules;
pub mod state;
pub mod turns;

#[doc(hidden)]
pub mod setup;

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
pub use public_view::{ShipWreckPublicView, public_view};
pub use raft::{Raft, RaftError, SlotId};
pub use resource::{InsufficientResources, Resource, ResourceCost};
pub use rules::ShipWreckGame;
pub use state::{GameState, PendingEvent, PendingEventKind, PlacedPlayerCard, PlayerState};
