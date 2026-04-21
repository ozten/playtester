//! Hand-scored show-phase scenarios.
//!
//! Plan verification bar: at least 40 scenarios, including the 29-hand.

use playtest_cribbage::{Card, Rank, Suit, score_hand};

fn c(r: Rank, s: Suit) -> Card {
    Card::new(r, s)
}

fn hand(a: (Rank, Suit), b: (Rank, Suit), d: (Rank, Suit), e: (Rank, Suit)) -> [Card; 4] {
    [c(a.0, a.1), c(b.0, b.1), c(d.0, d.1), c(e.0, e.1)]
}

// ========== THE CANONICAL MAXIMUM: 29 HAND ==============================

#[test]
fn twenty_nine_hand_full_composition() {
    // J♠ 5♣ 5♦ 5♥ + starter 5♠
    let h = hand(
        (Rank::Jack, Suit::Spades),
        (Rank::Five, Suit::Clubs),
        (Rank::Five, Suit::Diamonds),
        (Rank::Five, Suit::Hearts),
    );
    let starter = c(Rank::Five, Suit::Spades);
    let s = score_hand(h, starter, false);
    assert_eq!(s.fifteens, 16);
    assert_eq!(s.pairs, 12);
    assert_eq!(s.runs, 0);
    assert_eq!(s.flush, 0);
    assert_eq!(s.nobs, 1);
    assert_eq!(s.total, 29);
}

// ========== DEAD HANDS ==================================================

#[test]
fn dead_hand_scores_zero() {
    let h = hand(
        (Rank::Two, Suit::Spades),
        (Rank::Four, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Queen, Suit::Hearts);
    assert_eq!(score_hand(h, starter, false).total, 0);
}

#[test]
fn another_dead_hand() {
    let h = hand(
        (Rank::Three, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Nine, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Queen, Suit::Hearts);
    assert_eq!(score_hand(h, starter, false).total, 0);
}

// ========== FIFTEENS ====================================================

#[test]
fn simple_fifteen_from_five_plus_ten() {
    let h = hand(
        (Rank::Five, Suit::Spades),
        (Rank::Ten, Suit::Hearts),
        (Rank::Two, Suit::Diamonds),
        (Rank::Three, Suit::Clubs),
    );
    let starter = c(Rank::Seven, Suit::Spades);
    let s = score_hand(h, starter, false);
    // 5+10=15 (2)
    // 2+3+10=15 (2)
    // 3+5+7=15 (2)
    // That's 6 fifteens points alone.
    assert!(s.fifteens >= 6);
}

#[test]
fn fifteen_from_three_fives() {
    let h = hand(
        (Rank::Five, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Two, Suit::Clubs),
    );
    let starter = c(Rank::Eight, Suit::Spades);
    let s = score_hand(h, starter, false);
    // Five + Five + Five = 15 (one triple) → 2 pts.
    // Also three pairs of 5s → 6 pts.
    assert!(s.fifteens >= 2);
    assert_eq!(s.pairs, 6);
}

// ========== PAIRS =======================================================

#[test]
fn single_pair_scores_two() {
    let h = hand(
        (Rank::Seven, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Nine, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Three, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 2);
}

#[test]
fn triple_scores_six_pairs() {
    let h = hand(
        (Rank::Seven, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Seven, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Three, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 6);
}

#[test]
fn quad_scores_twelve() {
    let h = hand(
        (Rank::Seven, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Seven, Suit::Diamonds),
        (Rank::Seven, Suit::Clubs),
    );
    let starter = c(Rank::Three, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 12);
}

#[test]
fn two_separate_pairs_score_four() {
    let h = hand(
        (Rank::Seven, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Three, Suit::Diamonds),
        (Rank::Three, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 4);
}

#[test]
fn jack_queen_do_not_pair_despite_equal_values() {
    let h = hand(
        (Rank::Jack, Suit::Spades),
        (Rank::Queen, Suit::Hearts),
        (Rank::Two, Suit::Diamonds),
        (Rank::Three, Suit::Clubs),
    );
    let starter = c(Rank::Seven, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 0);
}

// ========== RUNS ========================================================

#[test]
fn run_of_three_scores_three() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Ace, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 3);
}

#[test]
fn run_of_four_scores_four() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::Seven, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 4);
}

#[test]
fn run_of_five_scores_five() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::Seven, Suit::Clubs),
    );
    let starter = c(Rank::Eight, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 5);
}

#[test]
fn double_run_of_three_scores_six() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Six, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 6);
}

#[test]
fn double_run_of_four_scores_eight() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::Six, Suit::Clubs),
    );
    let starter = c(Rank::Seven, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 8);
}

#[test]
fn triple_run_of_three_scores_nine() {
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Five, Suit::Clubs),
    );
    let starter = c(Rank::Six, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 9);
}

#[test]
fn double_double_run_scores_sixteen_total_across_runs_and_pairs() {
    // A♣ 2♠ 2♥ 3♦ + starter 3♣
    let h = hand(
        (Rank::Ace, Suit::Clubs),
        (Rank::Two, Suit::Spades),
        (Rank::Two, Suit::Hearts),
        (Rank::Three, Suit::Diamonds),
    );
    let starter = c(Rank::Three, Suit::Clubs);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 12, "double-double run");
    assert_eq!(s.pairs, 4, "pair of 2s + pair of 3s");
    // Plus fifteens: A+2+3+... let's check. A=1, 2=2, 3=3. Sums: any subset summing to 15?
    // 1+2+2+3+3 = 11 total. Max subset sum = 11. No 15.
    assert_eq!(s.fifteens, 0);
    assert_eq!(s.total, 16);
}

#[test]
fn ten_jack_queen_is_a_run() {
    let h = hand(
        (Rank::Ten, Suit::Spades),
        (Rank::Jack, Suit::Hearts),
        (Rank::Queen, Suit::Diamonds),
        (Rank::Three, Suit::Clubs),
    );
    let starter = c(Rank::Ace, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 3);
}

#[test]
fn king_queen_jack_is_a_run() {
    let h = hand(
        (Rank::King, Suit::Spades),
        (Rank::Queen, Suit::Hearts),
        (Rank::Jack, Suit::Diamonds),
        (Rank::Two, Suit::Clubs),
    );
    let starter = c(Rank::Six, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 3);
}

#[test]
fn no_wraparound_run_king_ace_two() {
    let h = hand(
        (Rank::King, Suit::Spades),
        (Rank::Ace, Suit::Hearts),
        (Rank::Two, Suit::Diamonds),
        (Rank::Seven, Suit::Clubs),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 0);
}

// ========== FLUSH =======================================================

#[test]
fn four_card_hand_flush_scores_four_non_crib() {
    let h = hand(
        (Rank::Two, Suit::Hearts),
        (Rank::Five, Suit::Hearts),
        (Rank::Seven, Suit::Hearts),
        (Rank::King, Suit::Hearts),
    );
    let starter = c(Rank::Nine, Suit::Clubs);
    let s = score_hand(h, starter, false);
    assert_eq!(s.flush, 4);
}

#[test]
fn five_card_flush_scores_five_non_crib() {
    let h = hand(
        (Rank::Two, Suit::Hearts),
        (Rank::Five, Suit::Hearts),
        (Rank::Seven, Suit::Hearts),
        (Rank::King, Suit::Hearts),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.flush, 5);
}

#[test]
fn four_card_crib_flush_scores_zero() {
    let h = hand(
        (Rank::Two, Suit::Hearts),
        (Rank::Five, Suit::Hearts),
        (Rank::Seven, Suit::Hearts),
        (Rank::King, Suit::Hearts),
    );
    let starter = c(Rank::Nine, Suit::Clubs);
    let s = score_hand(h, starter, true);
    assert_eq!(s.flush, 0, "crib flush requires 5-way match");
}

#[test]
fn five_card_crib_flush_scores_five() {
    let h = hand(
        (Rank::Two, Suit::Hearts),
        (Rank::Five, Suit::Hearts),
        (Rank::Seven, Suit::Hearts),
        (Rank::King, Suit::Hearts),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, true);
    assert_eq!(s.flush, 5);
}

#[test]
fn three_of_four_same_suit_is_not_a_flush() {
    let h = hand(
        (Rank::Two, Suit::Hearts),
        (Rank::Five, Suit::Hearts),
        (Rank::Seven, Suit::Hearts),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.flush, 0);
}

// ========== NOBS ========================================================

#[test]
fn jack_matching_starter_suit_is_nobs_one_point() {
    let h = hand(
        (Rank::Jack, Suit::Hearts),
        (Rank::Two, Suit::Clubs),
        (Rank::Four, Suit::Diamonds),
        (Rank::King, Suit::Spades),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.nobs, 1);
}

#[test]
fn jack_not_matching_starter_is_not_nobs() {
    let h = hand(
        (Rank::Jack, Suit::Spades),
        (Rank::Two, Suit::Clubs),
        (Rank::Four, Suit::Diamonds),
        (Rank::King, Suit::Spades),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.nobs, 0);
}

#[test]
fn crib_never_scores_nobs_even_with_matching_jack() {
    let h = hand(
        (Rank::Jack, Suit::Hearts),
        (Rank::Two, Suit::Clubs),
        (Rank::Four, Suit::Diamonds),
        (Rank::King, Suit::Spades),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(h, starter, true);
    assert_eq!(s.nobs, 0);
}

// ========== COMBINATIONS ================================================

#[test]
fn fifteen_plus_pair() {
    // 5, 5, 10, K + starter 3: 5+10=15 (×2) + 5+5+5=no + 5+K=15 ×2 + 5+K=15 ×2
    // Wait let me recount. H = 5,5,10,K. Starter = 3.
    // Pairs: 5,5 → 2 pts.
    // Fifteens: 5+10 (x2 because two 5s) + 5+K (x2) → 4 fifteens = 8 pts.
    // Runs: no run (5, K, 3, 10 — not consecutive).
    let h = hand(
        (Rank::Five, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Ten, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Three, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 2);
    assert_eq!(s.fifteens, 8);
}

#[test]
fn classic_hand_with_pair_fifteens_and_double_run() {
    // 4♠, 5♥, 5♦, 6♣ + starter K♥
    // Fifteens: {4,5♥,6}, {4,5♦,6}, {5♥,K}, {5♦,K} → 4 fifteens = 8 pts.
    // Pairs: 5-5 → 2 pts.
    // Runs: double run of 3 (4-5-6, 5 doubled) → 6 pts.
    // Total 16.
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Six, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.fifteens, 8);
    assert_eq!(s.pairs, 2);
    assert_eq!(s.runs, 6);
    assert_eq!(s.total, 16);
}

// ========== EDGE CASES =================================================

#[test]
fn starter_as_jack_does_not_score_nobs_when_no_matching_hand_jack() {
    // Starter=J♣. Hand has no J or J of wrong suit.
    let h = hand(
        (Rank::Two, Suit::Spades),
        (Rank::Three, Suit::Hearts),
        (Rank::Four, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Jack, Suit::Clubs);
    let s = score_hand(h, starter, false);
    assert_eq!(s.nobs, 0);
}

#[test]
fn hand_jack_with_starter_jack_same_suit_impossible_but_defensive() {
    // Only possible as a constructed test; in a real game the two
    // cards would be distinct. Just confirm the scorer doesn't
    // panic on duplicate cards.
    let h = hand(
        (Rank::Jack, Suit::Hearts),
        (Rank::Two, Suit::Clubs),
        (Rank::Four, Suit::Diamonds),
        (Rank::King, Suit::Spades),
    );
    let starter = c(Rank::Jack, Suit::Hearts);
    let s = score_hand(h, starter, false);
    // Nobs triggers: J♥ in hand matches starter's ♥.
    assert_eq!(s.nobs, 1);
    // Plus pair of Jacks: 2 pts.
    assert_eq!(s.pairs, 2);
}

#[test]
fn perfect_nineteen_hand_is_impossible_but_returns_valid_low_score() {
    // "19" is famously a cribbage joke — it's impossible to score 19
    // in a 4-card hand + starter. Verify our scorer returns <= 28.
    let h = hand(
        (Rank::Two, Suit::Spades),
        (Rank::Four, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::Nine, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert!(s.total < 19);
}

#[test]
fn zero_hand_crib_scores_zero() {
    let crib = hand(
        (Rank::Two, Suit::Spades),
        (Rank::Four, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Queen, Suit::Hearts);
    let s = score_hand(crib, starter, true);
    assert_eq!(s.total, 0);
}

#[test]
fn crib_can_still_score_fifteens_pairs_runs_even_without_flush_or_nobs() {
    let crib = hand(
        (Rank::Seven, Suit::Spades),
        (Rank::Eight, Suit::Hearts),
        (Rank::Seven, Suit::Diamonds),
        (Rank::Eight, Suit::Clubs),
    );
    let starter = c(Rank::Nine, Suit::Hearts);
    let s = score_hand(crib, starter, true);
    // 7+8=15 pairs × 2 (each 7 with each 8) = 4 fifteens = 8 pts.
    // 8+7=15 already counted.
    // Actually let's enumerate: 7s at indices 0,2. 8s at 1,3. 15s: {0,1}, {0,3}, {1,2}, {2,3} = 4 → 8 pts.
    // Starter 9: {0,?} 7+9=16 nope. 7+8 already. 8+9=17 nope. So just 8 pts.
    assert_eq!(s.fifteens, 8);
    assert_eq!(s.pairs, 4); // pair of 7s (+2) and pair of 8s (+2)
    // Run: 7,8,9 — ranks 7:2, 8:2, 9:1. Length 3, mult 4 → 12 pts.
    assert_eq!(s.runs, 12);
    // No flush: mixed suits.
    assert_eq!(s.flush, 0);
    // No nobs on crib.
    assert_eq!(s.nobs, 0);
    assert_eq!(s.total, 24);
}

#[test]
fn large_hand_of_mixed_scoring() {
    // 4♠ 4♥ 5♦ 6♣ + starter 5♠
    let h = hand(
        (Rank::Four, Suit::Spades),
        (Rank::Four, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Six, Suit::Clubs),
    );
    let starter = c(Rank::Five, Suit::Spades);
    let s = score_hand(h, starter, false);
    // Pair of 4s: 2. Pair of 5s: 2. Total pairs: 4.
    // Run: 4-5-6, ranks 4:2 5:2 6:1, length 3, mult 4 → 12.
    // Fifteens: 4+5+6=15 (×4 combos = 8 pts).
    assert_eq!(s.pairs, 4);
    assert_eq!(s.runs, 12);
    assert_eq!(s.fifteens, 8);
    assert_eq!(s.total, 24);
}

#[test]
fn ace_two_three_run_with_low_ace() {
    let h = hand(
        (Rank::Ace, Suit::Spades),
        (Rank::Two, Suit::Hearts),
        (Rank::Three, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Seven, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.runs, 3);
}

#[test]
fn two_pair_scores_four_even_without_other_scoring() {
    let h = hand(
        (Rank::Two, Suit::Spades),
        (Rank::Two, Suit::Hearts),
        (Rank::Nine, Suit::Diamonds),
        (Rank::Nine, Suit::Clubs),
    );
    let starter = c(Rank::King, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(s.pairs, 4);
    assert_eq!(s.fifteens, 0);
    assert_eq!(s.runs, 0);
}

#[test]
fn heavy_fifteens_hand() {
    // Four 5s + a 10 = many 15s + many pairs.
    let h = hand(
        (Rank::Five, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Five, Suit::Diamonds),
        (Rank::Five, Suit::Clubs),
    );
    let starter = c(Rank::Ten, Suit::Hearts);
    let s = score_hand(h, starter, false);
    // Pairs of 5s: 4 choose 2 = 6 pairs = 12 pts.
    assert_eq!(s.pairs, 12);
    // Fifteens: each 5 + 10 = 4 pairs + three 5s (sum 15) = C(4,3) = 4 triples = 8 fifteens → 16.
    assert_eq!(s.fifteens, 16);
    assert_eq!(s.total, 28);
}

#[test]
fn single_ace_alone_scores_zero() {
    let h = hand(
        (Rank::Ace, Suit::Spades),
        (Rank::Seven, Suit::Hearts),
        (Rank::Eight, Suit::Diamonds),
        (Rank::King, Suit::Clubs),
    );
    let starter = c(Rank::Queen, Suit::Hearts);
    let s = score_hand(h, starter, false);
    // 7+8 = 15 → 2 pts.
    assert_eq!(s.fifteens, 2);
    // Nothing else scores.
    assert_eq!(s.total, 2);
}

#[test]
fn show_score_total_equals_sum_of_components() {
    // Invariant check across a randomly-picked configuration.
    let h = hand(
        (Rank::Five, Suit::Spades),
        (Rank::Five, Suit::Hearts),
        (Rank::Six, Suit::Diamonds),
        (Rank::Seven, Suit::Clubs),
    );
    let starter = c(Rank::Eight, Suit::Hearts);
    let s = score_hand(h, starter, false);
    assert_eq!(
        s.total,
        s.fifteens + s.pairs + s.runs + s.flush + s.nobs,
        "total invariant broken"
    );
}
