//! Great Gyre game state.
//!
//! State is passive data — `apply_event` (in `rules.rs`) is the only
//! thing that mutates it. Public fields throughout: tests, the turn
//! helpers in `turns.rs`, and (eventually) `public_view` all need to
//! read the same shape, and private accessors would just be noise.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::Card;
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
/// from ShipWreck's `ResolvingEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl PendingDecisionKind {
    /// How many more picks this decision needs before it's satisfied.
    #[must_use]
    pub const fn needed(self) -> u8 {
        match self {
            Self::DiscardDown { needed }
            | Self::MakeHungry { needed }
            | Self::AbandonHungry { needed }
            | Self::StandUp { needed } => needed,
        }
    }
}

/// One entry on the pending-decision stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDecision {
    pub player: PlayerId,
    pub kind: PendingDecisionKind,
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
    /// Phase 3: the active player may take actions.
    Actions,
    /// A pending decision (discard-down or Phase-4 hungry/stand-up) is
    /// open; `GameState::pending_decisions.last()` says which.
    ResolvingDecision,
    Finished,
}

/// Per-seat state.
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
    /// Draws left this Phase 2 (reset to 1 at the start of every turn
    /// — Unit 3 will source the `1 + draw_bonus` formula from raft
    /// stats).
    pub draws_remaining: u8,
    /// Actions left this Phase 3 (reset to 1 at the start of every
    /// turn — Unit 3 will source `1 + action_bonus`).
    pub actions_remaining: u8,
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
        }
    }
}

/// Full state of a Great Gyre game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub config: GreatGyreConfig,
    /// One `PlayerState` per seat, indexed by `PlayerId`.
    pub players: Vec<PlayerState>,

    /// Survivors not yet drafted. Consumed during `Phase::SurvivorDraft`;
    /// any leftovers are folded into the shuffle at
    /// `Phase::AwaitingPostDraftShuffle`.
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
}

impl GameState {
    #[must_use]
    pub fn current_pending(&self) -> Option<&PendingDecision> {
        self.pending_decisions.last()
    }
}
