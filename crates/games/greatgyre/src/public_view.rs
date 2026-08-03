//! `GreatGyrePublicView` — the redacted view of game state from one
//! observer's seat.
//!
//! Per `docs/greatgyre.md`'s "Hidden information & views" section
//! (read together with `state.rs`'s `Face` doc comment, which
//! resolves an ambiguity in the spec's summary line — see below):
//!
//! **Public to every observer:**
//! - the observer's own hand (full identity)
//! - every player's face-*up* Current cards (identity; position
//!   matters, since Swimmer's blind picks target a position)
//! - every player's raft: base Left/Right, built extensions, placed
//!   survivors/modifications (incl. `hungry` markers), Walrus
//!   placements
//! - every player's per-turn budget counters (`draws_remaining`,
//!   `actions_remaining`, `porter_used`/`swimmer_used`/`pirate_used`,
//!   `work_day_active`) — all derivable from the public event log
//! - Deep Sea / Final Round / Event deck *counts* (not identities)
//! - the Discard Pile, in full, in order (top is drawable)
//! - the extension pile's remaining *count* (extensions are
//!   interchangeable — no identity to hide)
//! - the undrafted-survivor pool (the 12 survivors are named and
//!   fixed; "not yet drafted" is derivable from the public
//!   `SurvivorDrafted` log)
//! - first player, current player, phase, final-round flag, the
//!   pending-decision stack, and any pending chance step (none of
//!   these carry a hidden card — see the `EventReaction::held_card`
//!   note below)
//!
//! **Hidden from observer P** (redacted to a count or omitted):
//! - every *other* player's hand (count only)
//! - every player's face-*down* Current cards, **including the
//!   observer's own** — the spec's summary line says "own Current
//!   (full)" but its own Zones section is explicit: "Face-down
//!   Current cards are hidden from *everyone* until drawn (the owner
//!   effectively sees them only as draw options during their Phase
//!   2)". The Zones section — and `state.rs`'s `Face` doc comment,
//!   written from it — is authoritative; the summary line is
//!   imprecise. `CurrentSlotView::Down` therefore carries no card
//!   data, for every seat, observer included.
//!
//! A `PendingDecisionKind::EventReaction`'s `held_card` (the only
//! `Card` embedded in the pending-decision stack) is always a Walrus
//! that was just played via a public `EventCardPlayed` log record, so
//! passing the whole `pending_decisions` stack through unredacted
//! leaks nothing.
//!
//! The `PartialEq`/`Eq` derives are load-bearing: the determinize
//! property test compares public views before and after
//! determinization to verify that only private state was resampled.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::Card;
use crate::state::{
    CurrentCard, Face, GameState, PendingChance, PendingDecision, PlacedCard, Phase,
};

/// One slot in a player's Current pile, redacted per the "hidden from
/// everyone including the owner until drawn" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "face", rename_all = "snake_case")]
pub enum CurrentSlotView {
    /// Face-up: identity is public.
    Up { card: Card },
    /// Face-down: identity is hidden from everyone, including the
    /// owner, until drawn. Carries no card data by construction — the
    /// type itself is the redaction.
    Down,
}

/// The fixed pass direction (`docs/greatgyre.md`'s `[A]` ruling —
/// Currents always pass left). Modeled as an explicit view field
/// rather than a hardcoded UI assumption, so a future rules variant
/// could change it without a wire-format break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassDirection {
    Left,
}

/// Redacted per-seat raft/turn-state fields shared by [`OwnView`] and
/// [`OpponentView`]. Everything here is publicly observable via the
/// event log (raft placements, builds, per-turn ability usage) —
/// see the module doc's "Public to every observer" list.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors PlayerState — each bool is an independent once-per-turn usage flag; see its allow for rationale"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatPublic {
    pub player: PlayerId,
    /// Own-order Current pile; face-down slots carry no identity.
    pub current: Vec<CurrentSlotView>,
    pub raft_left: Card,
    pub raft_right: Card,
    pub built_extensions: Vec<Card>,
    pub placed: Vec<PlacedCard>,
    pub draws_remaining: u8,
    pub actions_remaining: u8,
    pub porter_used: bool,
    pub swimmer_used: bool,
    pub pirate_used: bool,
    pub blocked_by_walrus: Vec<Card>,
    pub work_day_active: bool,
}

/// What the observer sees about themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnView {
    pub seat: SeatPublic,
    pub hand: Vec<Card>,
}

/// One opponent seat as visible to the observer. No hand contents —
/// only a count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpponentView {
    pub seat: SeatPublic,
    pub hand_size: usize,
}

/// Full observer-visible snapshot. Every field is derived directly
/// from `GameState` + observer index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreatGyrePublicView {
    pub observer: PlayerId,
    pub own: OwnView,
    /// Indexed by `PlayerId`. `None` at `observer`'s own slot (see
    /// `own` instead) — mirrors ShipWreck's `Vec<Option<..>>` shape.
    pub opponents: Vec<Option<OpponentView>>,
    /// Survivors not yet drafted (only non-empty during
    /// `Phase::SurvivorDraft`) — public per the module doc.
    pub undrafted_survivors: Vec<Card>,
    pub deep_sea_deck_remaining: usize,
    pub final_round_deck_remaining: usize,
    pub event_deck_remaining: usize,
    /// Full, ordered; `.last()` is the top (drawable by Porter).
    pub discard_pile: Vec<Card>,
    pub extension_pile_remaining: usize,
    pub phase: Phase,
    pub current_player: PlayerId,
    pub first_player: PlayerId,
    pub pass_direction: PassDirection,
    pub final_round: bool,
    pub pending_decisions: Vec<PendingDecision>,
    pub pending_chance: Option<PendingChance>,
}

fn current_view(current: &[CurrentCard]) -> Vec<CurrentSlotView> {
    current
        .iter()
        .map(|cc| match cc.face {
            Face::Up => CurrentSlotView::Up { card: cc.card },
            Face::Down => CurrentSlotView::Down,
        })
        .collect()
}

fn seat_public(state: &GameState, idx: usize) -> SeatPublic {
    let p = &state.players[idx];
    SeatPublic {
        player: u8::try_from(idx).expect("seat index fits in u8"),
        current: current_view(&p.current),
        raft_left: p.raft_left,
        raft_right: p.raft_right,
        built_extensions: p.built_extensions.clone(),
        placed: p.placed.clone(),
        draws_remaining: p.draws_remaining,
        actions_remaining: p.actions_remaining,
        porter_used: p.porter_used,
        swimmer_used: p.swimmer_used,
        pirate_used: p.pirate_used,
        blocked_by_walrus: p.blocked_by_walrus.clone(),
        work_day_active: p.work_day_active,
    }
}

/// Build the public view for `observer` from `state`.
#[must_use]
pub fn public_view(state: &GameState, observer: PlayerId) -> GreatGyrePublicView {
    let observer_idx = observer as usize;

    let own = OwnView {
        seat: seat_public(state, observer_idx),
        hand: state.players[observer_idx].hand.clone(),
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
                    seat: seat_public(state, i),
                    hand_size: p.hand.len(),
                })
            }
        })
        .collect();

    GreatGyrePublicView {
        observer,
        own,
        opponents,
        undrafted_survivors: state.undrafted_survivors.clone(),
        deep_sea_deck_remaining: state.deep_sea_deck.len(),
        final_round_deck_remaining: state.final_round_deck.len(),
        event_deck_remaining: state.event_deck.len(),
        discard_pile: state.discard_pile.clone(),
        extension_pile_remaining: state.extension_pile.len(),
        phase: state.phase,
        current_player: state.current_player,
        first_player: state.first_player,
        pass_direction: PassDirection::Left,
        final_round: state.final_round,
        pending_decisions: state.pending_decisions.clone(),
        pending_chance: state.pending_chance,
    }
}
