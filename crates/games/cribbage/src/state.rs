//! The Cribbage game state machine.
//!
//! This module owns the state transitions for every phase implemented
//! in Unit 8 — Deal → Discard → Cut → Pegging. The show / crib /
//! winner-check phases land in Unit 9 behind the same API.
//!
//! The design mirrors the `playtest_core::Game` trait surface (legal
//! actions, apply action, apply event, resolve chance, next actor)
//! but not as a trait impl — Unit 9 wraps this module into
//! `CribbageGame: Game` once the show phase is done.
//!
//! `apply_action` is validating and returns a vector of events; every
//! other downstream effect flows through `apply_event`. `apply_event`
//! is infallible: events have already been validated.

use playtest_core::{Actor, EndReason, GameError, PlayerId};
use playtest_ports::Rng;
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::board::{Board, WINNING_SCORE};
use crate::card::{Card, Rank};
use crate::deck;
use crate::event::Event;
use crate::hand::Hand;
use crate::pegging::{PegReason, score_peg_play};
use crate::phase::Phase;
use crate::scoring::score_hand;

/// Full state of a single Cribbage game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub phase: Phase,
    pub dealer: PlayerId,
    pub hands: [Hand; 2],
    /// Cards each player has played during the current pegging phase.
    /// Grows from 0 to 4 per player; the show phase in Unit 9 reads
    /// these as the player's "counting" hand.
    pub played: [Vec<Card>; 2],
    pub crib: Vec<Card>,
    pub starter: Option<Card>,
    pub pegging_stack: Vec<Card>,
    pub running_total: u8,
    pub last_to_play: Option<PlayerId>,
    pub go_said: [bool; 2],
    pub to_act: PlayerId,
    pub board: Board,
    /// Remaining undealt deck. Starts at 52 cards in canonical order.
    /// Each `DealCard` and `CutStarter` event removes one card.
    pub deck: Vec<Card>,
    /// Show-phase cursor: 0 = non-dealer hand, 1 = dealer hand,
    /// 2 = dealer crib, 3 = ready for the `HandComplete` transition.
    /// Meaningful only while `phase == Show`.
    pub show_step: u8,
}

impl GameState {
    /// Construct a fresh game with the given dealer. Non-dealer leads
    /// both the discard and pegging phases (standard 2-player rules).
    #[must_use]
    pub fn new(dealer: PlayerId) -> Self {
        assert!(dealer < 2, "2-player only: dealer must be 0 or 1");
        Self {
            phase: Phase::Deal,
            dealer,
            hands: [Hand::empty(), Hand::empty()],
            played: [Vec::new(), Vec::new()],
            crib: Vec::new(),
            starter: None,
            pegging_stack: Vec::new(),
            running_total: 0,
            last_to_play: None,
            go_said: [false, false],
            to_act: 1 - dealer,
            board: Board::new(),
            deck: deck::fresh().to_vec(),
            show_step: 0,
        }
    }

    #[must_use]
    pub const fn non_dealer(&self) -> PlayerId {
        1 - self.dealer
    }

    /// Whoever is up next. `Show` is engine-driven (deterministic
    /// scoring, no randomness), but still exposed as `Actor::Chance`
    /// because there's no player to prompt — the loop pulls the next
    /// event via `resolve_chance`. `Finished` returns `None` to signal
    /// the game is over.
    #[must_use]
    pub const fn next_actor(&self) -> Option<Actor> {
        match self.phase {
            Phase::Deal | Phase::Cut | Phase::Show | Phase::ScoreCrib => Some(Actor::Chance),
            Phase::Discard | Phase::Pegging => Some(Actor::Player(self.to_act)),
            Phase::Finished => None,
        }
    }

    /// Terminal check. Returns `Some` once any player's pin has
    /// crossed [`WINNING_SCORE`] or `phase == Finished`.
    #[must_use]
    pub fn game_over(&self) -> Option<playtest_core::GameResult> {
        if let Some(winner) = self.board.winner() {
            return Some(playtest_core::GameResult {
                winner: Some(winner),
                reason: EndReason::Victory,
                scores: self.board.pins.iter().map(|p| i32::from(p.front)).collect(),
            });
        }
        None
    }

    /// Legal actions for `player` in the current phase. Returns an
    /// empty vector when it's not this player's turn, or during a
    /// non-action phase (Deal, Cut, Show, etc.).
    #[must_use]
    pub fn legal_actions(&self, player: PlayerId) -> Vec<Action> {
        if self.to_act != player {
            return Vec::new();
        }
        match self.phase {
            Phase::Discard => {
                let cards = self.hands[player as usize].cards();
                let mut out = Vec::with_capacity(cards.len() * (cards.len() - 1) / 2);
                for i in 0..cards.len() {
                    for j in (i + 1)..cards.len() {
                        out.push(Action::DiscardToCrib(cards[i], cards[j]));
                    }
                }
                out
            }
            Phase::Pegging => {
                let hand = self.hands[player as usize].cards();
                let playable: Vec<Action> = hand
                    .iter()
                    .filter(|c| u16::from(self.running_total) + u16::from(c.value()) <= 31)
                    .map(|&c| Action::PlayCard(c))
                    .collect();
                if playable.is_empty() && !hand.is_empty() {
                    vec![Action::SayGo]
                } else {
                    playable
                }
            }
            _ => Vec::new(),
        }
    }

    /// Validate `action` and convert it into the events it produces.
    ///
    /// # Errors
    /// Returns [`GameError::IllegalAction`] if `action` is not legal
    /// in the current state.
    pub fn apply_action(&self, player: PlayerId, action: &Action) -> Result<Vec<Event>, GameError> {
        if self.to_act != player {
            return Err(GameError::IllegalAction {
                player,
                message: format!("not player {player}'s turn (to_act={})", self.to_act),
            });
        }
        match (&self.phase, action) {
            (Phase::Discard, Action::DiscardToCrib(a, b)) => self.apply_discard(player, *a, *b),
            (Phase::Pegging, Action::PlayCard(card)) => self.apply_peg_play(player, *card),
            (Phase::Pegging, Action::SayGo) => self.apply_say_go(player),
            (phase, act) => Err(GameError::IllegalAction {
                player,
                message: format!("action {act:?} not legal in phase {phase:?}"),
            }),
        }
    }

    /// Resolve a chance event (Deal or Cut-phase draws).
    ///
    /// # Errors
    /// Returns [`GameError::RngFailed`] or [`GameError::ChanceFailed`]
    /// if the engine cannot produce the next chance event.
    pub fn resolve_chance(&self, rng: &mut dyn Rng) -> Result<Event, GameError> {
        match self.phase {
            Phase::Deal => self.resolve_chance_deal(rng),
            Phase::Cut => self.resolve_chance_cut(rng),
            Phase::Show => self.resolve_chance_show(),
            _ => Err(GameError::ChanceFailed {
                message: format!("resolve_chance called in phase {:?}", self.phase),
            }),
        }
    }

    /// Fold an event into state. Deliberately infallible — events have
    /// been validated by [`Self::apply_action`] or produced by
    /// [`Self::resolve_chance`].
    pub fn apply_event(&mut self, event: &Event) {
        match event {
            Event::DealCard { player, card } => self.fold_deal_card(*player, *card),
            Event::DiscardToCrib { player, cards } => self.fold_discard(*player, *cards),
            Event::CutStarter { card } => self.fold_cut_starter(*card),
            Event::NibsScored { player, points } => self.fold_nibs(*player, *points),
            Event::PegPlayed {
                player,
                card,
                running_total,
            } => self.fold_peg_played(*player, *card, *running_total),
            Event::PegScored { player, points, .. } => {
                self.board.advance(*player, u16::from(*points));
            }
            Event::Go { player } => {
                self.go_said[*player as usize] = true;
                self.refresh_peg_to_act();
            }
            Event::PeggingRoundEnd => self.fold_round_end(),
            Event::PeggingComplete => {
                self.phase = Phase::Show;
                self.show_step = 0;
            }
            Event::ShowScored { player, score, .. } => {
                self.board.advance(*player, u16::from(score.total));
                self.show_step += 1;
            }
            Event::HandComplete { next_dealer } => self.fold_hand_complete(*next_dealer),
            Event::EndGame { .. } => self.phase = Phase::Finished,
        }
    }

    // ---------- Chance resolvers ----------------------------------------

    fn resolve_chance_deal(&self, rng: &mut dyn Rng) -> Result<Event, GameError> {
        let dealt_total = self.hands[0].len() + self.hands[1].len();
        let recipient = if dealt_total.is_multiple_of(2) {
            self.non_dealer()
        } else {
            self.dealer
        };
        let card = self.draw_random_from_deck(rng)?;
        Ok(Event::DealCard {
            player: recipient,
            card,
        })
    }

    fn resolve_chance_cut(&self, rng: &mut dyn Rng) -> Result<Event, GameError> {
        if self.starter.is_none() {
            let card = self.draw_random_from_deck(rng)?;
            Ok(Event::CutStarter { card })
        } else {
            // Starter was a Jack; scheduled nibs is the next chance event.
            // (See `fold_cut_starter` for why we stay in Phase::Cut.)
            Ok(Event::NibsScored {
                player: self.dealer,
                points: 2,
            })
        }
    }

    fn resolve_chance_show(&self) -> Result<Event, GameError> {
        let starter = self.starter.ok_or_else(|| GameError::ChanceFailed {
            message: "show phase reached without a starter card".into(),
        })?;
        match self.show_step {
            0 => {
                let nd = self.non_dealer();
                let hand = self.counting_hand_for(nd)?;
                let score = score_hand(hand, starter, false);
                Ok(Event::ShowScored {
                    player: nd,
                    is_crib: false,
                    score,
                })
            }
            1 => {
                let hand = self.counting_hand_for(self.dealer)?;
                let score = score_hand(hand, starter, false);
                Ok(Event::ShowScored {
                    player: self.dealer,
                    is_crib: false,
                    score,
                })
            }
            2 => {
                if self.crib.len() != 4 {
                    return Err(GameError::ChanceFailed {
                        message: format!(
                            "crib has {} cards, expected 4 at show_step 2",
                            self.crib.len()
                        ),
                    });
                }
                let crib: [Card; 4] = [self.crib[0], self.crib[1], self.crib[2], self.crib[3]];
                let score = score_hand(crib, starter, true);
                Ok(Event::ShowScored {
                    player: self.dealer,
                    is_crib: true,
                    score,
                })
            }
            _ => Ok(Event::HandComplete {
                next_dealer: 1 - self.dealer,
            }),
        }
    }

    /// The 4-card hand used for show scoring — these are the cards
    /// the player actually played during pegging. Under standard
    /// Cribbage rules, every card held post-discard is played during
    /// pegging, so `played[p]` always has exactly 4 entries by the
    /// time the show phase starts.
    fn counting_hand_for(&self, p: PlayerId) -> Result<[Card; 4], GameError> {
        let played = &self.played[p as usize];
        if played.len() != 4 {
            return Err(GameError::ChanceFailed {
                message: format!(
                    "player {p} played {} cards at show time, expected 4",
                    played.len()
                ),
            });
        }
        Ok([played[0], played[1], played[2], played[3]])
    }

    fn draw_random_from_deck(&self, rng: &mut dyn Rng) -> Result<Card, GameError> {
        let n = self.deck.len();
        if n == 0 {
            return Err(GameError::ChanceFailed {
                message: "deck is empty".into(),
            });
        }
        let upper = u64::try_from(n).expect("deck size fits in u64");
        let idx_u64 = rng
            .gen_range(0..upper)
            .map_err(|source| GameError::RngFailed { source })?;
        let idx = usize::try_from(idx_u64).expect("idx < deck.len()");
        Ok(self.deck[idx])
    }

    // ---------- Action producers ----------------------------------------

    fn apply_discard(&self, player: PlayerId, a: Card, b: Card) -> Result<Vec<Event>, GameError> {
        if a == b {
            return Err(GameError::IllegalAction {
                player,
                message: format!("cannot discard the same card twice: {a}"),
            });
        }
        let hand = &self.hands[player as usize];
        if hand.len() != 6 {
            return Err(GameError::IllegalAction {
                player,
                message: format!("discard requires a 6-card hand, got {}", hand.len()),
            });
        }
        if !hand.contains(a) {
            return Err(GameError::IllegalAction {
                player,
                message: format!("card {a} not in hand"),
            });
        }
        if !hand.contains(b) {
            return Err(GameError::IllegalAction {
                player,
                message: format!("card {b} not in hand"),
            });
        }
        Ok(vec![Event::DiscardToCrib {
            player,
            cards: [a, b],
        }])
    }

    fn apply_peg_play(&self, player: PlayerId, card: Card) -> Result<Vec<Event>, GameError> {
        let hand = &self.hands[player as usize];
        if !hand.contains(card) {
            return Err(GameError::IllegalAction {
                player,
                message: format!("card {card} not in hand"),
            });
        }
        let new_total = u16::from(self.running_total) + u16::from(card.value());
        if new_total > 31 {
            return Err(GameError::IllegalAction {
                player,
                message: format!(
                    "playing {card} would push running total from {} to {new_total}, exceeding 31",
                    self.running_total
                ),
            });
        }
        let new_total_u8 = u8::try_from(new_total).expect("new_total <= 31");
        Ok(self.produce_peg_play_events(player, card, new_total_u8))
    }

    fn apply_say_go(&self, player: PlayerId) -> Result<Vec<Event>, GameError> {
        let hand = &self.hands[player as usize];
        if hand.is_empty() {
            return Err(GameError::IllegalAction {
                player,
                message: "cannot say Go with empty hand".into(),
            });
        }
        let has_playable = hand
            .cards()
            .iter()
            .any(|c| u16::from(self.running_total) + u16::from(c.value()) <= 31);
        if has_playable {
            return Err(GameError::IllegalAction {
                player,
                message: "cannot say Go when a legal play exists".into(),
            });
        }
        Ok(self.produce_go_events(player))
    }

    fn produce_peg_play_events(&self, player: PlayerId, card: Card, new_total: u8) -> Vec<Event> {
        let mut events = Vec::new();
        let mut sim = self.clone();

        let play_ev = Event::PegPlayed {
            player,
            card,
            running_total: new_total,
        };
        events.push(play_ev.clone());
        sim.apply_event(&play_ev);

        for reason in score_peg_play(&sim.pegging_stack, new_total) {
            let points = reason.points();
            let scored = Event::PegScored {
                player,
                reason,
                points,
            };
            events.push(scored.clone());
            sim.apply_event(&scored);
            if sim.board.score(player) >= WINNING_SCORE {
                events.push(Event::EndGame {
                    winner: player,
                    reason: EndReason::Victory,
                });
                return events;
            }
        }

        // Round-end handling.
        let round_ends_by_31 = new_total == 31;
        let both_stuck = !sim.has_something_to_do(0) && !sim.has_something_to_do(1);
        if round_ends_by_31 || both_stuck {
            append_round_end(&mut events, &mut sim, round_ends_by_31);
        }

        events
    }

    fn produce_go_events(&self, player: PlayerId) -> Vec<Event> {
        let mut events = Vec::new();
        let mut sim = self.clone();

        let go = Event::Go { player };
        events.push(go.clone());
        sim.apply_event(&go);

        if !sim.has_something_to_do(0) && !sim.has_something_to_do(1) {
            // Round ends. Running total is < 31 here (we only get to a
            // stuck-both-said-Go state when no play happened at 31).
            append_round_end(&mut events, &mut sim, /* by_31 */ false);
        }

        events
    }

    // ---------- Event folds ---------------------------------------------

    fn fold_deal_card(&mut self, player: PlayerId, card: Card) {
        self.remove_from_deck(card);
        self.hands[player as usize].push(card);
        if self.hands[0].len() + self.hands[1].len() == 12 {
            self.phase = Phase::Discard;
            self.to_act = self.non_dealer();
        }
    }

    fn fold_discard(&mut self, player: PlayerId, cards: [Card; 2]) {
        let hand = &mut self.hands[player as usize];
        // `apply_action` already validated the cards are present, so
        // `ok()` is safe here.
        let _ = hand.remove(cards[0]);
        let _ = hand.remove(cards[1]);
        self.crib.push(cards[0]);
        self.crib.push(cards[1]);
        if self.crib.len() == 4 {
            self.phase = Phase::Cut;
        } else {
            self.to_act = self.dealer;
        }
    }

    fn fold_cut_starter(&mut self, card: Card) {
        self.remove_from_deck(card);
        self.starter = Some(card);
        if card.rank == Rank::Jack {
            // Stay in Cut phase — the next chance event is NibsScored.
        } else {
            self.phase = Phase::Pegging;
            self.to_act = self.non_dealer();
        }
    }

    fn fold_nibs(&mut self, player: PlayerId, points: u8) {
        self.board.advance(player, u16::from(points));
        self.phase = Phase::Pegging;
        self.to_act = self.non_dealer();
    }

    fn fold_peg_played(&mut self, player: PlayerId, card: Card, running_total: u8) {
        let _ = self.hands[player as usize].remove(card);
        self.played[player as usize].push(card);
        self.pegging_stack.push(card);
        self.running_total = running_total;
        self.last_to_play = Some(player);
        self.refresh_peg_to_act();
    }

    /// Reset per-hand state and rotate the dealer. Board scores
    /// persist across hands — that's the whole point of multi-hand
    /// play toward the 121-point target.
    fn fold_hand_complete(&mut self, next_dealer: PlayerId) {
        self.hands = [Hand::empty(), Hand::empty()];
        self.played = [Vec::new(), Vec::new()];
        self.crib = Vec::new();
        self.starter = None;
        self.pegging_stack.clear();
        self.running_total = 0;
        self.last_to_play = None;
        self.go_said = [false, false];
        self.show_step = 0;
        self.deck = deck::fresh().to_vec();
        self.dealer = next_dealer;
        self.to_act = 1 - next_dealer;
        self.phase = Phase::Deal;
    }

    fn fold_round_end(&mut self) {
        let leader = self.determine_next_round_leader();
        self.pegging_stack.clear();
        self.running_total = 0;
        self.go_said = [false, false];
        self.last_to_play = None;
        if let Some(l) = leader {
            self.to_act = l;
        }
    }

    // ---------- Helpers -------------------------------------------------

    fn remove_from_deck(&mut self, card: Card) {
        if let Some(pos) = self.deck.iter().position(|c| *c == card) {
            self.deck.remove(pos);
        }
    }

    fn has_something_to_do(&self, p: PlayerId) -> bool {
        !self.go_said[p as usize] && !self.hands[p as usize].is_empty()
    }

    /// After a play or Go, pick the next to_act within the pegging
    /// phase. Prefers the opponent if they can still act; otherwise
    /// the current player stays on (Go scenario). If neither can act,
    /// to_act is left unchanged — the producer will emit
    /// `PeggingRoundEnd` before the loop observes it.
    fn refresh_peg_to_act(&mut self) {
        let current = self.to_act;
        let other = 1 - current;
        if self.has_something_to_do(other) {
            self.to_act = other;
        } else if self.has_something_to_do(current) {
            self.to_act = current;
        }
    }

    /// After a round ends, whoever did *not* play the last card leads
    /// the next round — unless they have no cards, in which case the
    /// last player continues solo. Returns `None` if both hands are
    /// empty (pegging is complete).
    fn determine_next_round_leader(&self) -> Option<PlayerId> {
        let last = self.last_to_play?;
        let other = 1 - last;
        if !self.hands[other as usize].is_empty() {
            Some(other)
        } else if !self.hands[last as usize].is_empty() {
            Some(last)
        } else {
            None
        }
    }
}

fn append_round_end(events: &mut Vec<Event>, sim: &mut GameState, ended_by_31: bool) {
    // Last-card bonus, if the round ended below 31 and someone played.
    if !ended_by_31
        && let Some(last) = sim.last_to_play
        && sim.running_total > 0
    {
        let last_card = Event::PegScored {
            player: last,
            reason: PegReason::LastCard,
            points: 1,
        };
        events.push(last_card.clone());
        sim.apply_event(&last_card);
        if sim.board.score(last) >= WINNING_SCORE {
            events.push(Event::EndGame {
                winner: last,
                reason: EndReason::Victory,
            });
            return;
        }
    }

    let round_end = Event::PeggingRoundEnd;
    events.push(round_end.clone());
    sim.apply_event(&round_end);

    if sim.hands[0].is_empty() && sim.hands[1].is_empty() {
        let complete = Event::PeggingComplete;
        events.push(complete.clone());
        sim.apply_event(&complete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    #[test]
    fn new_sets_up_fresh_deck_and_non_dealer_to_act() {
        let s = GameState::new(0);
        assert_eq!(s.phase, Phase::Deal);
        assert_eq!(s.dealer, 0);
        assert_eq!(s.non_dealer(), 1);
        assert_eq!(s.to_act, 1);
        assert_eq!(s.deck.len(), 52);
        assert!(s.hands[0].is_empty());
        assert!(s.hands[1].is_empty());
        assert_eq!(s.next_actor(), Some(Actor::Chance));
    }

    #[test]
    fn fold_deal_card_transitions_to_discard_after_12_cards() {
        let mut s = GameState::new(0);
        let fake = c(Rank::Ace, Suit::Clubs);
        for i in 0..12u8 {
            let p = 1 - (i % 2); // alternate: 1, 0, 1, 0, ...
            // Hand-craft events: use different cards so remove_from_deck
            // can find and remove each distinct one.
            let card = s.deck[0];
            s.apply_event(&Event::DealCard { player: p, card });
        }
        let _ = fake; // silence unused
        assert_eq!(s.phase, Phase::Discard);
        assert_eq!(s.to_act, 1); // non-dealer discards first
        assert_eq!(s.hands[0].len() + s.hands[1].len(), 12);
    }
}
