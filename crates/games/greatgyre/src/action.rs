//! Agent-chosen actions in Great Gyre.
//!
//! Resources are fungible within a kind, so build actions don't ask
//! the agent *which* literal Rope card to spend — `apply_action`
//! selects payment cards deterministically (first match in hand
//! order). Raft spaces are a fungible count, not positions (per the
//! plan), so `PlaySurvivor` / `BuildModification` don't take a slot
//! either — legality is a capacity check, not a placement choice.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::card::{CardInstanceId, SurvivorId};

/// Target selection for [`Action::PlayEvent`]. Shark Attack, Octopus
/// Attack, Walrus, and Love Boat each target exactly one other player
/// (the card's own legality — e.g. "≥2 survivors" — is checked at
/// apply time, not encoded here). Storm and Work Day take no target
/// (Storm affects every other player in turn order; Work Day affects
/// only the caster).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventTarget {
    Player { target: PlayerId },
    None,
}

/// Answers to an open [`crate::state::PendingDecision`]. Each variant
/// resolves exactly one unit of the decision's `needed` counter (or,
/// for the Unit 4 single-shot decision kinds, the whole decision).
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

    // ---------- Unit 4: events & reactions --------------------------
    /// Negate the pending Shark/Octopus/Walrus attack by discarding
    /// this Dead Fish from hand.
    ReactWithDeadFish { card: CardInstanceId },
    /// Negate the pending Shark/Octopus attack (Fisher's ability) by
    /// discarding this (any) hand card.
    ReactWithFisher { card: CardInstanceId },
    /// Take the attack's effect instead of reacting.
    DeclineReaction,
    /// Shark Attack: give up this placed survivor to the shared
    /// Discard Pile.
    LoseSurvivorToShark { survivor: CardInstanceId },
    /// Octopus Attack capacity shortfall: send this placed card to
    /// your own Current, face-up.
    RelocateFromOctopus { card: CardInstanceId },
    /// Love Boat: give this placed survivor to the caster.
    GiveSurvivorToLoveBoat { survivor: CardInstanceId },
    /// Storm: remove this placed card to your own Current, face-up.
    StormRemoveCard { card: CardInstanceId },
    /// Storm: take the "discard non-event hand cards" route instead
    /// of removing a placed card.
    StormTakeDiscardRoute,
    /// Storm, discard route: discard this (non-event) hand card
    /// face-down to your own Current.
    StormDiscard { card: CardInstanceId },
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
    /// `Phase::Draw`, Porter only: draw the top of the shared Discard
    /// Pile instead of a normal draw. Substitutes for (consumes) one
    /// of the player's draws; usable at most once per Phase 2.
    DrawFromDiscardPile,
    /// `Phase::Draw`, Swimmer only: draw a chosen card from an
    /// adjacent player's Current instead of a normal draw. Face-down
    /// picks are blind — the identity is revealed by the resulting
    /// event, not the action. Usable at most once per Phase 2.
    DrawFromAdjacentCurrent {
        neighbor: PlayerId,
        card: CardInstanceId,
    },
    /// `Phase::Draw`, Pirate only: draw a uniformly-random card from
    /// `target`'s hand instead of a normal draw. The player picks
    /// *whom* to steal from; which card is chance-resolved (see
    /// `Phase::AwaitingPirateSteal`). Usable at most once per Phase 2.
    DrawRandomFromHand { target: PlayerId },
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
    /// `Phase::Actions`: play an event card from hand. `target` names
    /// the other player, for the events that need one.
    PlayEvent {
        card: CardInstanceId,
        target: EventTarget,
    },
    /// `Phase::Actions`: discard a built Telescope to draw 1 Event
    /// card from the Event Deck into hand.
    ActivateTelescope { card: CardInstanceId },
    /// `Phase::Actions`: discard a Dead Fish from hand to remove a
    /// Walrus blocking one of this player's own spaces.
    RemoveWalrus {
        dead_fish: CardInstanceId,
        walrus: CardInstanceId,
    },
    /// `Phase::Actions`, Work Day only: draw a card from your own
    /// Current for free (doesn't consume a Phase-2-style draw budget).
    WorkDayDraw { card: CardInstanceId },
    /// `Phase::Actions`: stop taking actions early (always legal).
    FinishActions,

    /// Answer whichever [`crate::state::PendingDecision`] is open.
    /// Named field (not a tuple variant) so the inner `DecisionChoice`
    /// object nests under `"choice"` in the wire form instead of
    /// colliding with `Action`'s own `"kind"` tag — both enums are
    /// internally tagged with the same tag name.
    ResolveDecision { choice: DecisionChoice },
}
