//! Agent-chosen actions in Great Gyre.
//!
//! Resources are fungible within a kind, so build actions don't ask
//! the agent *which* literal Rope card to spend — `apply_action`
//! selects payment cards deterministically (first match in hand
//! order). Raft spaces are a fungible count, not positions (per the
//! plan), so `PlaySurvivor` / `BuildModification` don't take a slot
//! either — legality is a capacity check, not a placement choice.

use serde::{Deserialize, Serialize};

use crate::card::{CardInstanceId, SurvivorId};

/// Answers to an open [`crate::state::PendingDecision`]. Each variant
/// resolves exactly one unit of the decision's `needed` counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionChoice {
    /// Discard this hand card face-down to the player's own Current.
    Discard { card: CardInstanceId },
    /// Turn this standing survivor Hungry.
    MakeHungry { survivor: CardInstanceId },
    /// Return this Hungry survivor to the Current, face-up.
    AbandonHungry { survivor: CardInstanceId },
    /// Stand this Hungry survivor back up.
    StandUp { survivor: CardInstanceId },
}

/// Every intent an agent can express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// `Phase::SurvivorDraft`: claim one of the 12 survivors.
    DraftSurvivor { survivor: SurvivorId },

    /// `Phase::Draw`: draw one card from the player's own Current into
    /// their hand. Which physical card it turns out to be (for
    /// face-down picks) is revealed by the resulting event, not the
    /// action.
    DrawFromCurrent { card: CardInstanceId },
    /// `Phase::Draw`: stop drawing early (always legal).
    FinishDrawing,

    /// `Phase::Actions`: play a survivor from hand onto a free space
    /// (or, for Stowaway, with no space check at all).
    PlaySurvivor { card: CardInstanceId },
    /// `Phase::Actions`: build a modification from hand, paying its
    /// resource cost from hand to the Discard Pile.
    BuildModification { card: CardInstanceId },
    /// `Phase::Actions`: build a raft extension, paying 1 Wood + 1
    /// Rope + 1 Plastic from hand and taking one instance from the
    /// shared extension pile.
    BuildExtension,
    /// `Phase::Actions`: stop taking actions early (always legal).
    FinishActions,

    /// Answer whichever [`crate::state::PendingDecision`] is open.
    /// Named field (not a tuple variant) so the inner `DecisionChoice`
    /// object nests under `"choice"` in the wire form instead of
    /// colliding with `Action`'s own `"kind"` tag — both enums are
    /// internally tagged with the same tag name.
    ResolveDecision { choice: DecisionChoice },
}
