//! The atomic observable changes that make up a Cribbage game log.
//!
//! `apply_action` converts one [`crate::action::Action`] into a
//! sequence of [`Event`]s; those events are what `apply_event` folds
//! into state and what the event log preserves for replay.

use playtest_core::{EndReason, PlayerId};
use serde::{Deserialize, Serialize};

use crate::card::Card;
use crate::pegging::PegReason;
use crate::scoring::ShowScore;

/// Full event taxonomy for Cribbage. The `#[serde(tag = "kind")]`
/// representation gives each variant a self-describing JSON line in
/// the event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Chance event during the deal. Emitted once per card dealt in
    /// alternating dealer/non-dealer order — 12 events total.
    DealCard { player: PlayerId, card: Card },

    /// Player committed two cards to the crib. Emitted twice per game
    /// (non-dealer first, then dealer).
    DiscardToCrib { player: PlayerId, cards: [Card; 2] },

    /// Chance event: the starter card was cut from the remaining deck.
    CutStarter { card: Card },

    /// Nibs ("his heels"): starter was a Jack, dealer scores 2.
    /// Emitted immediately after `CutStarter` and only then.
    NibsScored { player: PlayerId, points: u8 },

    /// Player played a card during pegging. `running_total` is the new
    /// running total after this card.
    PegPlayed {
        player: PlayerId,
        card: Card,
        running_total: u8,
    },

    /// Pegging score: 15, 31, pair/triple/quad, or run.
    PegScored {
        player: PlayerId,
        reason: PegReason,
        points: u8,
    },

    /// Player said "Go" — they have cards left but cannot play.
    Go { player: PlayerId },

    /// The pegging stack reset (either someone hit 31 or both said Go).
    /// Happens between rounds of the pegging phase.
    PeggingRoundEnd,

    /// All cards have been played during pegging. Marks the transition
    /// out of the pegging phase.
    PeggingComplete,

    /// One of the three show-phase scoring steps fired: non-dealer
    /// hand, dealer hand, or dealer crib. `is_crib` distinguishes the
    /// crib step from the hand steps.
    ShowScored {
        player: PlayerId,
        is_crib: bool,
        score: ShowScore,
    },

    /// A hand has completed without a winner — dealer rotates and the
    /// next hand begins. Emitted by the engine between the end of show
    /// and the start of the next deal.
    HandComplete { next_dealer: PlayerId },

    /// A player crossed 121. The engine emits no further events after
    /// this one — the game is over.
    EndGame { winner: PlayerId, reason: EndReason },
}
