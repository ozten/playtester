//! Great Gyre game state.
//!
//! State is passive data — `apply_event` (in `rules.rs`) is the only
//! thing that mutates it. Public fields throughout: tests, the turn
//! helpers in `turns.rs`, and (eventually) `public_view` all need to
//! read the same shape, and private accessors would just be noise.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::{Card, EventKind};
use crate::config::GreatGyreConfig;

/// Whether a Current-pile card is showing its face or not. Face-down
/// identity is hidden from *everyone* (including the owner) until
/// drawn — see `docs/greatgyre.md`'s Zones section. The engine's
/// internal `State` always carries the real `Card`; only
/// `public_view` (Unit 5) redacts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    Up,
    Down,
}

/// One card sitting in a player's Current pile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentCard {
    pub card: Card,
    pub face: Face,
}

/// A survivor or modification placed face-up on a player's raft.
/// `hungry` only has meaning for survivors — sideways survivors still
/// count their stats per `docs/greatgyre.md`'s `[A]` ruling — but is
/// harmlessly `false` for modifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedCard {
    pub card: Card,
    pub hungry: bool,
}

/// The kind of decision currently open on `GameState::pending_decisions`.
/// Modeled as a "pick one of N, repeat until satisfied" counter rather
/// than a single combinatorial action, so `legal_actions` stays a flat,
/// boundedly-sized list (per plan: the pending-decision stack pattern
/// from ShipWreck's `ResolvingEvent`). Not `Copy` — Storm's
/// `queue_tail` carries a `Vec<PlayerId>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingDecisionKind {
    /// Discard `needed` more hand cards (face-down to own Current) to
    /// reach the max hand size.
    DiscardDown { needed: u8 },
    /// Turn `needed` more standing survivors Hungry to cover a food
    /// deficit.
    MakeHungry { needed: u8 },
    /// Return `needed` more Hungry survivors to the Current (face-up)
    /// because there weren't enough standing survivors to cover the
    /// deficit.
    AbandonHungry { needed: u8 },
    /// Stand up `needed` more Hungry survivors (food surplus, fewer
    /// than the full Hungry count).
    StandUp { needed: u8 },

    // ---------- Unit 4: events & reactions --------------------------
    /// `attacker` played a negatable event (Shark/Octopus/Walrus)
    /// against this player. `held_card` is `Some` only for Walrus
    /// (which doesn't discard on play — it's held here until the
    /// reaction resolves one way or the other); `None` for Shark/
    /// Octopus (already discarded at play time).
    EventReaction {
        attacker: PlayerId,
        event: EventKind,
        held_card: Option<Card>,
    },
    /// Shark Attack landed: this player chooses which placed survivor
    /// to lose (to the shared Discard Pile).
    SharkChooseSurvivor { attacker: PlayerId },
    /// Octopus Attack landed and capacity now falls `needed` short of
    /// occupancy (the fungible-capacity reading of "cards on the lost
    /// spaces relocate" — see `docs/greatgyre.md`'s `[A]` rulings in
    /// the engine report): this player picks `needed` placed cards to
    /// send to their own Current, face-up.
    OctopusRelocate { needed: u8, attacker: PlayerId },
    /// Love Boat landed: this player chooses which placed survivor to
    /// give to `attacker`.
    LoveBoatChooseSurvivor { attacker: PlayerId },
    /// Storm: this player chooses to remove one placed card (face-up
    /// to their own Current) or discard non-event hand cards.
    /// `queue_tail` is who's left to decide after this player, in
    /// down-current order.
    StormChoice {
        attacker: PlayerId,
        queue_tail: Vec<PlayerId>,
    },
    /// Storm, discard route: discard `needed` more non-event hand
    /// cards (face-down to this player's own Current); `needed` is
    /// already clamped to however many non-event cards they hold.
    StormDiscard {
        needed: u8,
        attacker: PlayerId,
        queue_tail: Vec<PlayerId>,
    },
}

impl PendingDecisionKind {
    /// How many more picks this decision needs before it's satisfied.
    /// Only meaningful for the "pick N of M, repeat" kinds; the
    /// single-shot Unit 4 kinds (`EventReaction`,
    /// `SharkChooseSurvivor`, `LoveBoatChooseSurvivor`, `StormChoice`)
    /// resolve via bespoke logic in `rules.rs` rather than this
    /// counter, so they report `0` (unused).
    #[must_use]
    pub const fn needed(&self) -> u8 {
        match self {
            Self::DiscardDown { needed }
            | Self::MakeHungry { needed }
            | Self::AbandonHungry { needed }
            | Self::StandUp { needed }
            | Self::OctopusRelocate { needed, .. }
            | Self::StormDiscard { needed, .. } => *needed,
            Self::EventReaction { .. }
            | Self::SharkChooseSurvivor { .. }
            | Self::LoveBoatChooseSurvivor { .. }
            | Self::StormChoice { .. } => 0,
        }
    }
}

/// One entry on the pending-decision stack. Not `Copy` — see
/// `PendingDecisionKind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDecision {
    pub player: PlayerId,
    pub kind: PendingDecisionKind,
}

/// A one-off chance step in progress, mirroring `pending_decisions`'
/// stack pattern but for the (rarer) case where the *engine*, not the
/// player, needs to resolve something via the `Rng` port. Currently
/// only Pirate's random hand-steal; `Phase::AwaitingPirateSteal` is
/// the corresponding phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingChance {
    /// `player`'s Pirate draws a uniformly-random card from `target`'s hand.
    PirateSteal { player: PlayerId, target: PlayerId },
}

/// High-level state of the Great Gyre turn machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Each seat, in order, picks one of the 12 survivors.
    SurvivorDraft,
    /// The draft just completed; the post-draft shuffle-and-deal is a
    /// chance step (it needs the `Rng` port, which only
    /// `resolve_chance` has access to).
    AwaitingPostDraftShuffle,
    /// Phase 2: the active player may draw from their own Current.
    Draw,
    /// A Pirate steal was initiated; the actual card is picked by
    /// `resolve_chance` (see `GameState::pending_chance`).
    AwaitingPirateSteal,
    /// Phase 3: the active player may take actions.
    Actions,
    /// A pending decision (discard-down or Phase-4 hungry/stand-up) is
    /// open; `GameState::pending_decisions.last()` says which.
    ResolvingDecision,
    Finished,
}

/// Per-seat state.
#[allow(clippy::struct_excessive_bools, reason = "each bool is an independent once-per-turn usage flag (Porter/Swimmer/Pirate/Work Day); they don't combine into meaningful states worth an enum")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerState {
    /// Hidden hand: survivors, modifications, resources, and (inert in
    /// Unit 2) event cards.
    pub hand: Vec<Card>,
    /// The player's Current pile, oldest-added first.
    pub current: Vec<CurrentCard>,
    pub raft_left: Card,
    pub raft_right: Card,
    /// Raft extensions this player has built (each +2 spaces).
    pub built_extensions: Vec<Card>,
    /// Survivors + modifications placed face-up on this raft.
    pub placed: Vec<PlacedCard>,
    /// Draws left this Phase 2. Reset at the start of every turn to
    /// `1 + draw_bonus` (Net +1 each, Athlete +1) —
    /// `crate::turns::draw_bonus`.
    pub draws_remaining: u8,
    /// Actions left this Phase 3. Reset at the start of every turn to
    /// `1 + action_bonus` (Toolkit +1, First Mate +1) —
    /// `crate::turns::action_bonus`.
    pub actions_remaining: u8,
    /// Whether this player has already used Porter's "draw from
    /// Discard Pile" substitute source this Phase 2. Reset at
    /// `TurnStarted`.
    pub porter_used: bool,
    /// Whether this player has already used Swimmer's "draw from an
    /// adjacent Current" substitute source this Phase 2. Reset at
    /// `TurnStarted`.
    pub swimmer_used: bool,
    /// Whether this player has already used Pirate's "draw a random
    /// card from another hand" substitute source this Phase 2. Reset
    /// at `TurnStarted`.
    pub pirate_used: bool,
    /// Walrus cards currently blocking a space on *this* player's
    /// raft (placed there by another player's Walrus event). Each
    /// blocks exactly 1 space — tracked as distinct entries per the
    /// plan's design decision, unlike the fungible survivor/
    /// modification occupancy count. Cleared by discarding a Dead
    /// Fish (`Action::RemoveWalrus`).
    pub blocked_by_walrus: Vec<Card>,
    /// Whether Work Day is active for this player's *current* turn:
    /// unlimited Phase-3 actions, and drawing from their own Current
    /// becomes a free repeatable Phase-3 action. Reset to `false` at
    /// `TurnStarted`.
    pub work_day_active: bool,
}

impl PlayerState {
    #[must_use]
    pub fn fresh(raft_left: Card, raft_right: Card) -> Self {
        Self {
            hand: Vec::new(),
            current: Vec::new(),
            raft_left,
            raft_right,
            built_extensions: Vec::new(),
            placed: Vec::new(),
            draws_remaining: 0,
            actions_remaining: 0,
            porter_used: false,
            swimmer_used: false,
            pirate_used: false,
            blocked_by_walrus: Vec::new(),
            work_day_active: false,
        }
    }
}

/// Full state of a Great Gyre game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub config: GreatGyreConfig,
    /// One `PlayerState` per seat, indexed by `PlayerId`.
    pub players: Vec<PlayerState>,

    /// Survivors not yet drafted. Shrinks during `Phase::SurvivorDraft`
    /// as each seat picks one; any leftovers are folded into the
    /// shuffle pool at `Phase::AwaitingPostDraftShuffle`. Empty after
    /// `Event::PostDraftSetup` applies (same as `pending_shuffle_pool`/
    /// `pending_event_pool` below).
    pub undrafted_survivors: Vec<Card>,
    /// Modifications + Dead Fish + resources, awaiting the post-draft
    /// shuffle. Empty after `Event::PostDraftSetup` applies.
    pub pending_shuffle_pool: Vec<Card>,
    /// Event cards, awaiting the post-draft shuffle. Empty after
    /// `Event::PostDraftSetup` applies.
    pub pending_event_pool: Vec<Card>,

    /// Face-down draw pile. `deep_sea_deck.last()` is the top.
    pub deep_sea_deck: Vec<Card>,
    /// Set aside at setup (`2 * num_players`); Phase 1 draws from here
    /// once `deep_sea_deck` empties, which also triggers the Final
    /// Round.
    pub final_round_deck: Vec<Card>,
    /// Remaining event-deck cards after the 1-per-seat setup deal.
    /// Not drawn from again until Unit 4 (Telescope activation).
    pub event_deck: Vec<Card>,
    /// Shared face-up discard pile. `discard_pile.last()` is the top
    /// (public; Porter may draw from it — Unit 3).
    pub discard_pile: Vec<Card>,
    /// Shared, finite raft-extension pile.
    pub extension_pile: Vec<Card>,

    pub phase: Phase,
    pub current_player: PlayerId,
    /// Whoever holds the First Player token this round. Combined with
    /// `current_player` and `players.len()`, this is enough to derive
    /// whose turn is next and whether the round is complete —
    /// `(current_player + 1) % n == first_player` — without a separate
    /// mutable turn-order queue that would need its own event to stay
    /// in sync with replay (see `rules.rs::end_of_turn_chain`).
    pub first_player: PlayerId,
    /// Set when the Deep Sea Deck empties during a Phase 1 draw. The
    /// round in progress finishes normally; Phase 5 is skipped at its
    /// end and the game ends instead.
    pub final_round: bool,
    /// Stack of in-progress decisions. In Unit 2 this is never more
    /// than one deep (no nested events yet), but the shape matches
    /// ShipWreck's `event_resolution_stack` so Unit 4's event
    /// resolutions can reuse it.
    pub pending_decisions: Vec<PendingDecision>,
    /// The one-off chance step in progress, if `phase` is one of the
    /// `Awaiting*` variants that needs the `Rng` port. `None`
    /// otherwise.
    pub pending_chance: Option<PendingChance>,
}

impl GameState {
    #[must_use]
    pub fn current_pending(&self) -> Option<&PendingDecision> {
        self.pending_decisions.last()
    }
}
