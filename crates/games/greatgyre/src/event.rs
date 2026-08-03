//! Log-record events for Great Gyre.
//!
//! Every variant derives [`Serialize`]/[`Deserialize`] so `apply_event`
//! can fold the JSONL event stream back into state and replay tools
//! can read logs off disk. `apply_action` is the only place that
//! produces these; `apply_event` is deliberately infallible.

use playtest_core::{EndReason, PlayerId};
use serde::{Deserialize, Serialize};

use crate::action::EventTarget;
use crate::card::Card;
use crate::state::PendingDecisionKind;

/// One player's final score, for the `end_game` event: hope icons on
/// face-up raft cards, `+15` if Land Sighting is in hand, `-5` per
/// other player's Influencer. Signed — a heavy Influencer penalty can
/// in principle outweigh a small hope total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreRow {
    pub player: PlayerId,
    pub hope: i32,
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

    // ---------- Unit 4: events & reactions --------------------------
    /// `player` played `card` from hand, targeting `target`. Moves the
    /// card from hand to the shared Discard Pile — *except* Walrus,
    /// which is held (identified by `Event::PendingDecisionOpened`'s
    /// `EventReaction::held_card`) until the reaction resolves, since
    /// its card physically ends up on a raft, not the discard pile.
    EventCardPlayed {
        player: PlayerId,
        card: Card,
        target: EventTarget,
    },
    /// The event's target declined to react (or has no reaction
    /// available); the attack proceeds.
    ReactionDeclined { player: PlayerId },
    /// The event's target discarded a Dead Fish to negate the attack.
    ReactedWithDeadFish { player: PlayerId, card: Card },
    /// The event's target (Fisher) discarded a hand card to negate
    /// the attack.
    ReactedWithFisher { player: PlayerId, card: Card },
    /// A Walrus was placed on `player`'s raft by `attacker`,
    /// occupying (blocking) one space.
    WalrusPlaced {
        player: PlayerId,
        attacker: PlayerId,
        card: Card,
    },
    /// A reaction negated a Walrus play; `player` (the would-be
    /// attacker) never gets to place it — it goes straight to the
    /// shared Discard Pile instead.
    WalrusDiscarded { player: PlayerId, card: Card },
    /// `player` discarded a Dead Fish to remove a Walrus blocking one
    /// of their own spaces.
    WalrusRemoved {
        player: PlayerId,
        dead_fish: Card,
        walrus: Card,
    },
    /// Shark Attack: `player` gave up `survivor` to the shared Discard Pile.
    SurvivorLostToShark { player: PlayerId, survivor: Card },
    /// Octopus Attack: `player` lost `extension` back to the shared pile.
    ExtensionLostToOctopus { player: PlayerId, extension: Card },
    /// Octopus Attack capacity shortfall: `card` moved from `player`'s
    /// raft to their own Current, face-up.
    RelocatedFromOctopus { player: PlayerId, card: Card },
    /// Love Boat: `player` gave `survivor` to `recipient`; it moves
    /// straight onto `recipient`'s raft.
    SurvivorGivenToLoveBoat {
        player: PlayerId,
        recipient: PlayerId,
        survivor: Card,
    },
    /// Storm: `player` removed `card` from their raft to their own
    /// Current, face-up.
    StormCardRemoved { player: PlayerId, card: Card },
    /// Storm: `player` took the discard route (no further state
    /// change by itself; a `StormDiscard` pending decision may follow
    /// if they hold any non-event hand cards).
    StormDiscardRouteTaken { player: PlayerId },
    /// Work Day: unlimited actions and free own-Current draws are now
    /// active for `player`'s current turn.
    WorkDayActivated { player: PlayerId },
    /// Work Day: `player` drew `card` from their own Current for free
    /// (doesn't touch `draws_remaining`).
    WorkDayDrew { player: PlayerId, card: Card },
    /// `player` discarded a built Telescope to draw `drawn` from the
    /// Event Deck into hand.
    TelescopeActivated {
        player: PlayerId,
        telescope: Card,
        drawn: Card,
    },
    /// The current event-interrupt chain (reaction + effect) fully
    /// resolved; control returns to `player`'s `Phase::Actions`.
    EventResolved { player: PlayerId },

    /// The game ended.
    EndGame {
        winners: Vec<PlayerId>,
        reason: EndReason,
        final_scores: Vec<ScoreRow>,
    },
}
