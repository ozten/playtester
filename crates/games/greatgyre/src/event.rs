//! Log-record events for Great Gyre.
//!
//! Every variant derives [`Serialize`]/[`Deserialize`] so `apply_event`
//! can fold the JSONL event stream back into state and replay tools
//! can read logs off disk. `apply_action` is the only place that
//! produces these; `apply_event` is deliberately infallible.

use playtest_core::{EndReason, PlayerId};
use serde::{Deserialize, Serialize};

use crate::card::Card;
use crate::state::PendingDecisionKind;

/// One player's final hope total, for the `end_game` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreRow {
    pub player: PlayerId,
    pub hope: u32,
}

/// Every observable change to a Great Gyre game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    // ---------- setup / draft (Unit 1) ----------------------------
    // Raft assignment needs no event: it is deterministic from the
    // catalog and baked directly into `Game::initial_state` (tick 0),
    // the same way ShipWreck's fresh `Raft::new()` needs no event.
    /// `Phase::SurvivorDraft`: `player` claimed `survivor`, placed
    /// directly face-up on their raft (see `setup.rs` doc comment for
    /// the `[A]` ruling on why the draft places rather than hands out).
    SurvivorDrafted { player: PlayerId, survivor: Card },
    /// Chance: the post-draft shuffle-and-deal. Bundled into one event
    /// because `resolve_chance` returns exactly one event per call and
    /// this is the only place Great Gyre needs the `Rng` port
    /// mid-game. Also carries the very first Phase-1 add for seat 0,
    /// so the very next actor is a real decision (`Phase::Draw`), not
    /// another no-op prompt.
    PostDraftSetup {
        /// `hands[p]` = the 3 cards dealt to seat `p`'s hand.
        hands: Vec<Vec<Card>>,
        /// `currents[p]` = the 3 cards dealt face-up to seat `p`'s Current.
        currents: Vec<Vec<Card>>,
        /// `event_hand_cards[p]` = the 1 event card dealt to seat `p`.
        event_hand_cards: Vec<Card>,
        deep_sea_deck: Vec<Card>,
        final_round_deck: Vec<Card>,
        event_deck: Vec<Card>,
        /// Seat 0's opening Phase-1 add, if the deck had a card.
        first_add: Option<Card>,
    },

    // ---------- turn machine (Unit 2) ------------------------------
    /// A new turn segment begins for `player` (Phase 1's owner).
    TurnStarted { player: PlayerId },
    /// The Deep Sea Deck emptied during a Phase-1 draw; this round is
    /// now the Final Round.
    FinalRoundTriggered,
    /// Phase 1 (auto): one card added face-down to `player`'s own Current.
    CurrentCardAdded { player: PlayerId, card: Card },

    /// Phase 2: `player` drew `card` from their own Current into hand.
    CardDrawnFromCurrent { player: PlayerId, card: Card },
    /// Phase 2, Porter: `player` drew `card` from the top of the
    /// shared Discard Pile instead of a normal draw.
    DrewFromDiscardPile { player: PlayerId, card: Card },
    /// Phase 2, Swimmer: `player` drew `card` from `neighbor`'s
    /// Current instead of a normal draw.
    DrewFromAdjacentCurrent {
        player: PlayerId,
        neighbor: PlayerId,
        card: Card,
    },
    /// Phase 2, Pirate: `player` initiated a random steal from
    /// `target`'s hand. The actual card is picked by the following
    /// chance step (`Event::PirateStole`) — this event only records
    /// the choice of target and transitions to
    /// `Phase::AwaitingPirateSteal`.
    PirateStealInitiated { player: PlayerId, target: PlayerId },
    /// Chance: the uniformly-random card `player`'s Pirate stole from
    /// `target`'s hand.
    PirateStole {
        player: PlayerId,
        target: PlayerId,
        card: Card,
    },
    /// Phase 2 ends (budget exhausted or `finish_drawing`).
    DrawingFinished { player: PlayerId },

    /// Phase 3: `player` played `card` (a survivor) from hand onto the raft.
    SurvivorPlayed { player: PlayerId, card: Card },
    /// A resource card left `player`'s hand to the Discard Pile, part
    /// of paying for a build.
    ResourceDiscarded { player: PlayerId, card: Card },
    /// Phase 3: `player` built `card` (a modification) from hand.
    ModificationBuilt { player: PlayerId, card: Card },
    /// Phase 3: `player` built a raft extension from the shared pile.
    ExtensionBuilt { player: PlayerId, extension: Card },
    /// Phase 3 ends (budget exhausted or `finish_actions`).
    ActionsFinished { player: PlayerId },

    /// A new pending decision opened for `player`.
    PendingDecisionOpened { player: PlayerId, decision: PendingDecisionKind },
    /// Hand-limit discard: `card` moved from hand to `player`'s own
    /// Current, face-down.
    CardDiscarded { player: PlayerId, card: Card },
    /// Phase 4: `survivor` turned Hungry (sideways).
    SurvivorMadeHungry { player: PlayerId, survivor: Card },
    /// Phase 4: `survivor` (already Hungry) was returned to the
    /// Current, face-up, because there weren't enough standing
    /// survivors to cover the food deficit.
    SurvivorAbandoned { player: PlayerId, survivor: Card },
    /// Phase 4: `survivor` stood back up.
    SurvivorStoodUp { player: PlayerId, survivor: Card },

    /// `player`'s turn segment (Phases 1-4) is complete.
    TurnEnded { player: PlayerId },
    /// Phase 5 (auto): every Current passed to the left neighbor,
    /// preserving face state, and the First Player token passed left.
    CurrentsPassed { new_first_player: PlayerId },

    /// The game ended.
    EndGame {
        winners: Vec<PlayerId>,
        reason: EndReason,
        final_scores: Vec<ScoreRow>,
    },
}
