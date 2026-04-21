//! Integration tests for the Cribbage primitives (Unit 7).
//!
//! Covers the scenarios listed in the plan, with the `Rng` port
//! threaded through the adapter crate's `StubRng` to prove the
//! determinism invariant.

use std::collections::HashSet;

use playtest_adapters::StubRng;
use playtest_cribbage::{Board, Card, DECK_SIZE, Hand, HandError, Rank, Suit, WINNING_SCORE, deck};

// ---------- Deck: uniqueness + shuffle determinism ----------------------

#[test]
fn fresh_deck_has_fifty_two_unique_cards() {
    let d = deck::fresh();
    assert_eq!(d.len(), DECK_SIZE);
    let set: HashSet<_> = d.iter().copied().collect();
    assert_eq!(set.len(), DECK_SIZE);
}

#[test]
fn shuffle_is_deterministic_under_identical_seeds() {
    let mut deck_a = deck::fresh();
    let mut deck_b = deck::fresh();
    let mut rng_a = StubRng::seeded(1776);
    let mut rng_b = StubRng::seeded(1776);
    deck::shuffle(&mut deck_a, &mut rng_a).unwrap();
    deck::shuffle(&mut deck_b, &mut rng_b).unwrap();
    assert_eq!(deck_a, deck_b);
}

#[test]
fn shuffle_preserves_the_set_of_cards() {
    let mut d = deck::fresh();
    let mut rng = StubRng::seeded(42);
    deck::shuffle(&mut d, &mut rng).unwrap();
    let set: HashSet<_> = d.iter().copied().collect();
    assert_eq!(set.len(), DECK_SIZE);
}

#[test]
fn different_seeds_usually_produce_different_orders() {
    // Not "always" — two seeds could in principle produce identical
    // permutations by astronomical coincidence. With ChaCha20-derived
    // seeds and a 52! permutation space the probability is effectively
    // zero; an equality here would signal a real bug.
    let mut deck_a = deck::fresh();
    let mut deck_b = deck::fresh();
    deck::shuffle(&mut deck_a, &mut StubRng::seeded(1)).unwrap();
    deck::shuffle(&mut deck_b, &mut StubRng::seeded(2)).unwrap();
    assert_ne!(deck_a, deck_b);
}

// ---------- Card::value table -------------------------------------------

#[test]
fn card_value_table_matches_cribbage_rules() {
    let expected: [(Rank, u8); 13] = [
        (Rank::Ace, 1),
        (Rank::Two, 2),
        (Rank::Three, 3),
        (Rank::Four, 4),
        (Rank::Five, 5),
        (Rank::Six, 6),
        (Rank::Seven, 7),
        (Rank::Eight, 8),
        (Rank::Nine, 9),
        (Rank::Ten, 10),
        (Rank::Jack, 10),
        (Rank::Queen, 10),
        (Rank::King, 10),
    ];
    for (rank, value) in expected {
        assert_eq!(Card::new(rank, Suit::Clubs).value(), value, "{rank:?}");
    }
}

#[test]
fn face_cards_value_does_not_equal_rank_ord() {
    // The single most common Cribbage bug: treating J/Q/K as "10" for
    // run detection. This test fails loudly if value() and rank_ord()
    // are ever conflated.
    for rank in [Rank::Jack, Rank::Queen, Rank::King] {
        let c = Card::new(rank, Suit::Hearts);
        assert_eq!(c.value(), 10);
        assert_ne!(c.rank_ord(), c.value(), "{rank:?}");
    }
    assert_eq!(Card::new(Rank::Jack, Suit::Hearts).rank_ord(), 11);
    assert_eq!(Card::new(Rank::Queen, Suit::Hearts).rank_ord(), 12);
    assert_eq!(Card::new(Rank::King, Suit::Hearts).rank_ord(), 13);
}

// ---------- Board -------------------------------------------------------

#[test]
fn board_advance_happy_path() {
    let mut b = Board::new();
    b.advance(0, 5);
    assert_eq!(b.score(0), 5);
    b.advance(1, 3);
    assert_eq!(b.score(1), 3);
    assert_eq!(b.winner(), None);
}

#[test]
fn board_advance_to_exactly_winning_score_reports_winner() {
    let mut b = Board::new();
    b.advance(0, WINNING_SCORE);
    assert_eq!(b.score(0), WINNING_SCORE);
    assert_eq!(b.winner(), Some(0));
}

#[test]
fn board_advance_past_winning_score_also_reports_winner() {
    // ACC rule: first to 121 wins; overshoot still counts. We do not
    // clamp -- the score reflects the actual points pegged.
    let mut b = Board::new();
    b.advance(0, 120);
    b.advance(0, 6); // overshoot to 126
    assert_eq!(b.score(0), 126);
    assert_eq!(b.winner(), Some(0));
}

#[test]
fn board_winner_returns_first_player_to_cross_threshold_when_tied() {
    // If both players somehow end up at 121 (not achievable in real
    // play because turns alternate, but guard the invariant anyway),
    // player 0 is reported as the winner -- consistent with the
    // "first to reach" framing.
    let mut b = Board::new();
    b.advance(0, WINNING_SCORE);
    b.advance(1, WINNING_SCORE);
    assert_eq!(b.winner(), Some(0));
}

// ---------- Hand --------------------------------------------------------

#[test]
fn hand_remove_missing_card_returns_err_without_mutating() {
    let mut h = Hand::new(vec![
        Card::new(Rank::Ace, Suit::Clubs),
        Card::new(Rank::Two, Suit::Diamonds),
    ]);
    let missing = Card::new(Rank::King, Suit::Spades);
    let err = h.remove(missing).unwrap_err();
    assert_eq!(err, HandError::NotInHand(missing));
    assert_eq!(h.len(), 2, "hand must be unchanged on failed remove");
}
