//! Agent-chosen actions in ShipWreck.
//!
//! Actions are the *intent* an agent expresses. The engine validates
//! them (is this card in my hand? does this slot exist? are the
//! resources affordable?) and converts them into [`crate::event::Event`]s.
//! Only events touch state; actions are the handshake between agent
//! and engine.
//!
//! Action payloads are deliberately self-contained so that:
//! 1. `legal_actions` can enumerate every concrete action without
//!    external lookups.
//! 2. A `PublicView` serialized over the web wire can faithfully
//!    describe the choice menu to an LLM or browser client.
//!
//! See `docs/shipwreck.md` for the game-rules-level semantics of each
//! variant.

use serde::{Deserialize, Serialize};

use crate::PlayerId;
use crate::card::{EquipmentKind, PlayerCardId};
use crate::raft::SlotId;

/// The three event-card kinds, matching [`crate::card::EventCard`] but
/// carried on [`Action::PlayEventCard`] to avoid the nested
/// `Action::PlayEventCard(EventCard::Shark)` indirection. The two
/// enums are kept in lockstep by test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCardKind {
    /// Attacks one upgrade or extension on a chosen player's raft.
    /// Steel-cordage defends and is destroyed instead.
    Shark,
    /// Every player loses one upgrade or extension (chosen per-player).
    Typhoon,
    /// Grants the current player one unit of food this turn.
    FlyingFish,
}

/// Target selection for [`Action::PlayEventCard`].
///
/// - `Shark` must carry a target player + slot (their raft position
///   chosen for destruction). Base-raft slots are illegal per spec.
/// - `Typhoon` has no target — every player chooses their own
///   sacrifice during resolution. `None` is the only valid payload.
/// - `FlyingFish` has no target; it resolves on the caster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventTarget {
    /// Targets a single slot on `player`'s raft. Used by Shark.
    SingleSlot { player: PlayerId, slot: SlotId },
    /// No target. Used by Typhoon and FlyingFish.
    None,
}

/// Payload for [`Action::ResolveEvent`] — how a player answers a
/// queued event that demands their input (currently only Typhoon).
///
/// Shark and FlyingFish resolve immediately on play, so they never
/// produce a pending resolution for another player to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventResolution {
    /// Sacrifice the given slot (an extension or upgrade) to the typhoon.
    TyphoonLose(SlotId),
    /// Pass: the player has no upgrade or extension to sacrifice. Base
    /// raft cards are never forfeited per `docs/shipwreck.md`.
    TyphoonPass,
}

/// Every intent an agent can express on their turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Insert a raft-extension card from hand immediately after
    /// `insert_after`. The extension must be in the agent's hand.
    ExtendRaft { insert_after: SlotId },

    /// Move a player card from hand onto a raft slot. The slot must
    /// be on the agent's own raft and currently have no player card on
    /// it. (Slots already holding an *equipment* upgrade are still
    /// legal targets — player cards sit on slots independently of
    /// equipment; this is encoded in `PlayerState::played_players`.)
    PlacePlayerCard { card: PlayerCardId, slot: SlotId },

    /// Pick a specific face-up card from a player's pool. `from_pool`
    /// identifies the pool (normally the agent's own; with a
    /// Telescope, an adjacent pool); `card_index` is the position of
    /// the chosen card in `face_up_pools[from_pool]`.
    PickWreckage { from_pool: PlayerId, card_index: u16 },

    /// Play an event card from hand.
    PlayEventCard { card: EventCardKind, target: EventTarget },

    /// Spend resources to build the given equipment kind onto `slot`.
    /// Slot must be on the agent's raft and currently have no
    /// equipment upgrade.
    BuildEquipment { equipment_kind: EquipmentKind, slot: SlotId },

    /// End the current player's turn. Triggers end-of-turn food
    /// consumption and hands control to the next seat.
    EndTurn,

    /// Respond to a pending event that targeted this player. Legal
    /// only during [`crate::phase::Phase::ResolvingEvent`].
    ResolveEvent(EventResolution),
}
