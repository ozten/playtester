//! Hand-scored pegging scenarios. Each test names a concrete stack
//! and running-total and asserts which [`PegReason`]s fire.
//!
//! Per Unit 8's verification bar: at least 30 hand-scored scenarios.

use playtest_cribbage::{Card, PegReason, Rank, Suit, score_peg_play};

fn c(rank: Rank, suit: Suit) -> Card {
    Card::new(rank, suit)
}

/// Build a stack from a list of (rank, suit) tuples, in play order.
macro_rules! stack {
    ($($r:expr, $s:expr);* $(;)?) => {
        vec![$(c($r, $s)),*]
    };
}

fn running_total(stack: &[Card]) -> u8 {
    let mut t: u16 = 0;
    for card in stack {
        t += u16::from(card.value());
    }
    u8::try_from(t).expect("peg total exceeded 31 in test setup")
}

// ---------- 15 and 31 ---------------------------------------------------

#[test]
fn peg_fifteen_seven_eight() {
    let s = stack!(Rank::Seven, Suit::Clubs; Rank::Eight, Suit::Diamonds);
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Fifteen));
    assert!(!r.contains(&PegReason::ThirtyOne));
    assert_eq!(r.iter().map(|x| x.points()).sum::<u8>(), 2);
}

#[test]
fn peg_fifteen_six_nine() {
    let s = stack!(Rank::Six, Suit::Clubs; Rank::Nine, Suit::Diamonds);
    assert!(score_peg_play(&s, 15).contains(&PegReason::Fifteen));
}

#[test]
fn peg_thirty_one_three_tens_and_ace() {
    let s = stack!(
        Rank::Ten, Suit::Clubs;
        Rank::Ten, Suit::Diamonds;
        Rank::Ten, Suit::Hearts;
        Rank::Ace, Suit::Spades
    );
    assert!(score_peg_play(&s, 31).contains(&PegReason::ThirtyOne));
}

#[test]
fn peg_thirty_one_pair_does_not_also_fire_fifteen() {
    // Total jumped over 15; should not retroactively fire fifteen.
    let s = stack!(
        Rank::Ten, Suit::Clubs;
        Rank::Ten, Suit::Diamonds;
        Rank::Jack, Suit::Hearts;
        Rank::Ace, Suit::Spades
    );
    let r = score_peg_play(&s, 31);
    assert!(r.contains(&PegReason::ThirtyOne));
    assert!(!r.contains(&PegReason::Fifteen));
}

#[test]
fn peg_no_fifteen_when_total_passes_over_it() {
    let s = stack!(Rank::Eight, Suit::Clubs; Rank::Ten, Suit::Diamonds);
    assert!(!score_peg_play(&s, 18).contains(&PegReason::Fifteen));
}

// ---------- Pairs / triples / quads -------------------------------------

#[test]
fn peg_pair_same_rank_last_two() {
    let s =
        stack!(Rank::Three, Suit::Clubs; Rank::Seven, Suit::Diamonds; Rank::Seven, Suit::Hearts);
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Pair));
    assert!(!r.contains(&PegReason::Triple));
}

#[test]
fn peg_triple_same_rank_last_three() {
    let s = stack!(
        Rank::Seven, Suit::Clubs;
        Rank::Seven, Suit::Diamonds;
        Rank::Seven, Suit::Hearts
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Triple));
    assert!(!r.contains(&PegReason::Pair));
    assert_eq!(
        r.iter().filter(|x| matches!(x, PegReason::Triple)).count(),
        1
    );
}

#[test]
fn peg_quadruple_same_rank_last_four() {
    // "Double pair royal" — four of a kind, +12.
    let s = stack!(
        Rank::Seven, Suit::Clubs;
        Rank::Seven, Suit::Diamonds;
        Rank::Seven, Suit::Hearts;
        Rank::Seven, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Quadruple));
    assert_eq!(r.iter().map(|x| x.points()).sum::<u8>(), 12);
}

#[test]
fn peg_pair_uses_rank_ord_not_value_jack_queen_dont_pair() {
    // J and Q are both value=10 but different rank_ord. Not a pair.
    let s = stack!(Rank::Jack, Suit::Clubs; Rank::Queen, Suit::Diamonds);
    assert!(!score_peg_play(&s, 20).contains(&PegReason::Pair));
}

#[test]
fn peg_pair_king_king() {
    let s = stack!(Rank::King, Suit::Clubs; Rank::King, Suit::Diamonds);
    assert!(score_peg_play(&s, 20).contains(&PegReason::Pair));
}

#[test]
fn peg_pair_not_triple_when_third_from_last_differs() {
    let s = stack!(
        Rank::Three, Suit::Clubs;
        Rank::Seven, Suit::Diamonds;
        Rank::Seven, Suit::Hearts
    );
    // Last three are 3,7,7. Not a triple. Last two are 7,7 — pair.
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Pair));
    assert!(!r.contains(&PegReason::Triple));
}

// ---------- Runs --------------------------------------------------------

#[test]
fn peg_run_of_three_sorted_order() {
    let s = stack!(Rank::Three, Suit::Clubs; Rank::Four, Suit::Diamonds; Rank::Five, Suit::Hearts);
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_run_of_three_out_of_order() {
    // Classic "last N in any order" run.
    let s = stack!(Rank::Three, Suit::Clubs; Rank::Five, Suit::Diamonds; Rank::Four, Suit::Hearts);
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_run_of_three_9_5_6_ending_with_4_is_a_run() {
    // Plan example: "9, 5, 6, 4" ending with 4 — last 3 are 5, 6, 4 → run of 3.
    let s = stack!(
        Rank::Nine, Suit::Clubs;
        Rank::Five, Suit::Diamonds;
        Rank::Six, Suit::Hearts;
        Rank::Four, Suit::Spades
    );
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_run_of_four_extending_the_previous_run() {
    // 9, 5, 6, 4, 3 — last 4 are 5, 6, 4, 3 → run of 4.
    let s = stack!(
        Rank::Nine, Suit::Clubs;
        Rank::Five, Suit::Diamonds;
        Rank::Six, Suit::Hearts;
        Rank::Four, Suit::Spades;
        Rank::Three, Suit::Clubs
    );
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(4)));
}

#[test]
fn peg_run_of_five() {
    let s = stack!(
        Rank::Three, Suit::Clubs;
        Rank::Four, Suit::Diamonds;
        Rank::Two, Suit::Hearts;
        Rank::Ace, Suit::Spades;
        Rank::Five, Suit::Clubs
    );
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(5)));
}

#[test]
fn peg_run_interrupted_by_nine_is_not_a_run() {
    // Plan example: "4, 9, 5, 6" is NOT a run. Last 3 are 9, 5, 6 — 9 breaks it.
    let s = stack!(
        Rank::Four, Suit::Clubs;
        Rank::Nine, Suit::Diamonds;
        Rank::Five, Suit::Hearts;
        Rank::Six, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
}

#[test]
fn peg_run_broken_by_duplicate_at_end() {
    // 4, 5, 6, 5 — last 3 are 5, 6, 5 (dup). Last 2 are 6, 5 — not a pair.
    // No run, no pair.
    let s = stack!(
        Rank::Four, Suit::Clubs;
        Rank::Five, Suit::Diamonds;
        Rank::Six, Suit::Hearts;
        Rank::Five, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
    assert!(!r.contains(&PegReason::Pair));
}

#[test]
fn peg_run_uses_rank_ord_not_value_ten_jack_queen() {
    // 10, J, Q are consecutive via rank_ord (10, 11, 12) even though
    // all three have value=10.
    let s = stack!(
        Rank::Ten, Suit::Clubs;
        Rank::Jack, Suit::Diamonds;
        Rank::Queen, Suit::Hearts
    );
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_run_king_queen_jack() {
    let s = stack!(Rank::King, Suit::Clubs; Rank::Queen, Suit::Diamonds; Rank::Jack, Suit::Hearts);
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_run_ace_low_two_three() {
    // Ace = 1 for runs. A, 2, 3 is a valid run.
    let s = stack!(Rank::Ace, Suit::Clubs; Rank::Two, Suit::Diamonds; Rank::Three, Suit::Hearts);
    assert!(score_peg_play(&s, running_total(&s)).contains(&PegReason::Run(3)));
}

#[test]
fn peg_no_wraparound_run_king_ace_two() {
    // K (13), A (1), 2 (2). No wraparound in Cribbage.
    let s = stack!(Rank::King, Suit::Clubs; Rank::Ace, Suit::Diamonds; Rank::Two, Suit::Hearts);
    let r = score_peg_play(&s, running_total(&s));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
}

// ---------- Combinations ------------------------------------------------

#[test]
fn peg_fifteen_plus_pair() {
    // 7C, 5D, 3H, 7S — wait, that's 22. Let's do 5, 5, 5 = 15 but only
    // last 2 are 5,5 for pair.
    // Running totals: 5, 10, 15. Last card takes total to 15, and the
    // last two are a pair (5, 5). That's fifteen + pair.
    let s = stack!(
        Rank::Five, Suit::Clubs;
        Rank::Five, Suit::Diamonds;
        Rank::Five, Suit::Hearts
    );
    let r = score_peg_play(&s, 15);
    assert!(r.contains(&PegReason::Fifteen));
    // But also: all three are rank 5 → triple (+6) fires.
    assert!(r.contains(&PegReason::Triple));
    assert_eq!(r.iter().map(|x| x.points()).sum::<u8>(), 2 + 6);
}

#[test]
fn peg_fifteen_plus_run() {
    // 4, 5, 6 = 15 total AND run of 3.
    let s = stack!(Rank::Four, Suit::Clubs; Rank::Five, Suit::Diamonds; Rank::Six, Suit::Hearts);
    let r = score_peg_play(&s, 15);
    assert!(r.contains(&PegReason::Fifteen));
    assert!(r.contains(&PegReason::Run(3)));
    assert_eq!(r.iter().map(|x| x.points()).sum::<u8>(), 2 + 3);
}

#[test]
fn peg_triple_with_total_21() {
    let s = stack!(
        Rank::Seven, Suit::Clubs;
        Rank::Seven, Suit::Diamonds;
        Rank::Seven, Suit::Hearts
    );
    let r = score_peg_play(&s, 21);
    assert!(r.contains(&PegReason::Triple));
    assert!(!r.contains(&PegReason::Fifteen));
    assert_eq!(r.iter().map(|x| x.points()).sum::<u8>(), 6);
}

// ---------- Edge cases --------------------------------------------------

#[test]
fn peg_empty_stack_scores_nothing() {
    assert!(score_peg_play(&[], 0).is_empty());
}

#[test]
fn peg_single_card_scores_nothing() {
    let s = stack!(Rank::Seven, Suit::Clubs);
    assert!(score_peg_play(&s, 7).is_empty());
}

#[test]
fn peg_single_card_ending_at_fifteen_not_possible_but_if_it_were() {
    // A single card can't reach 15 (max single-card value is 10), but
    // for algorithm robustness: a 1-card stack at "total 15" shouldn't
    // score a pair/triple/run. It would still score fifteen.
    let s = stack!(Rank::Five, Suit::Clubs);
    let r = score_peg_play(&s, 15);
    assert!(r.contains(&PegReason::Fifteen));
    assert!(!r.contains(&PegReason::Pair));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
}

#[test]
fn peg_pair_after_a_run() {
    // 3, 4, 5, 5 — last 3 are 4, 5, 5 (not a run, has dup).
    // Last 2 are 5, 5 — pair.
    let s = stack!(
        Rank::Three, Suit::Clubs;
        Rank::Four, Suit::Diamonds;
        Rank::Five, Suit::Hearts;
        Rank::Five, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Pair));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
}

#[test]
fn peg_longer_run_preferred_over_shorter() {
    // 2, 3, 4 is a run of 3. 1, 2, 3, 4 is a run of 4. We should
    // return the longer one.
    let s = stack!(
        Rank::Ace, Suit::Clubs;
        Rank::Two, Suit::Diamonds;
        Rank::Three, Suit::Hearts;
        Rank::Four, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Run(4)));
    assert!(!r.contains(&PegReason::Run(3)));
}

#[test]
fn peg_no_short_run_inside_longer() {
    // 4, 2, 3, 5 — last 3 are 2, 3, 5 → not a run. Last 4 are 4, 2, 3, 5 → run of 4.
    let s = stack!(
        Rank::Four, Suit::Clubs;
        Rank::Two, Suit::Diamonds;
        Rank::Three, Suit::Hearts;
        Rank::Five, Suit::Spades
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Run(4)));
}

#[test]
fn peg_run_and_pair_not_both_across_same_boundary() {
    // After playing the second 7: last 2 are 7,7 (pair). Last 3 are
    // 6, 7, 7 — has dup, not a run.
    let s = stack!(
        Rank::Six, Suit::Clubs;
        Rank::Seven, Suit::Diamonds;
        Rank::Seven, Suit::Hearts
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Pair));
    assert!(!r.iter().any(|x| matches!(x, PegReason::Run(_))));
}

#[test]
fn peg_last_card_reason_has_one_point_value() {
    assert_eq!(PegReason::LastCard.points(), 1);
}

#[test]
fn peg_run_of_seven_maximum_practical() {
    // Maximum run in Cribbage: 7 cards (A-2-3-4-5-6-7 sums to 28 < 31).
    let s = stack!(
        Rank::Ace, Suit::Clubs;
        Rank::Two, Suit::Diamonds;
        Rank::Three, Suit::Hearts;
        Rank::Four, Suit::Spades;
        Rank::Five, Suit::Clubs;
        Rank::Six, Suit::Diamonds;
        Rank::Seven, Suit::Hearts
    );
    let r = score_peg_play(&s, running_total(&s));
    assert!(r.contains(&PegReason::Run(7)));
}
