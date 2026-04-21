//! Show-scoring rule: **runs**.
//!
//! Find the longest contiguous range of rank ordinals with count ≥ 1
//! across the counting set, where the range length is ≥ 3. Score =
//! `length × product_of_counts`. The product captures "double runs",
//! "triple runs", and "double-double runs" in one formula.
//!
//! Examples:
//! - Ranks 4,5,6 (each once): length 3, product 1·1·1 = 1 → 3 pts.
//! - Ranks 4,5,5,6 (5 twice): length 3, product 1·2·1 = 2 → 6 pts (double run).
//! - Ranks 4,5,5,5,6 (5 thrice): length 3, product 1·3·1 = 3 → 9 pts.
//! - Ranks 4,4,5,5,6 (two pairs): length 3, product 2·2·1 = 4 → 12 pts (double-double).
//! - Ranks 3,4,5,6,7 (each once): length 5, product 1 → 5 pts.

use crate::card::Card;

/// Score the runs across the counting set.
#[must_use]
pub fn score(cards: &[Card; 5]) -> u8 {
    let mut counts: [u8; 14] = [0; 14];
    for c in cards {
        counts[c.rank_ord() as usize] += 1;
    }

    // Scan the rank axis (1..=13) for maximal runs of consecutive
    // ranks with count > 0. Multiple disjoint runs are impossible in
    // 5 cards when the longest is >= 3 (the remaining 2 cards couldn't
    // form a second run of 3+), so we just find the longest.
    let mut best_len: u8 = 0;
    let mut best_mult: u32 = 0;

    let mut i: usize = 1;
    while i <= 13 {
        if counts[i] == 0 {
            i += 1;
            continue;
        }
        let start = i;
        let mut mult: u32 = 1;
        while i <= 13 && counts[i] > 0 {
            mult = mult.saturating_mul(u32::from(counts[i]));
            i += 1;
        }
        let len = u8::try_from(i - start).expect("run len fits in u8");
        if len >= 3 && (len > best_len || (len == best_len && mult > best_mult)) {
            best_len = len;
            best_mult = mult;
        }
    }

    if best_len >= 3 {
        let score = u32::from(best_len) * best_mult;
        u8::try_from(score).unwrap_or(u8::MAX)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    fn cards(ranks: [Rank; 5]) -> [Card; 5] {
        [
            c(ranks[0], Suit::Spades),
            c(ranks[1], Suit::Hearts),
            c(ranks[2], Suit::Diamonds),
            c(ranks[3], Suit::Clubs),
            c(ranks[4], Suit::Spades),
        ]
    }

    #[test]
    fn no_run_in_scattered_ranks() {
        let hand = cards([Rank::Two, Rank::Five, Rank::Eight, Rank::Jack, Rank::King]);
        assert_eq!(score(&hand), 0);
    }

    #[test]
    fn run_of_three_scores_three() {
        let hand = cards([Rank::Four, Rank::Five, Rank::Six, Rank::Nine, Rank::King]);
        assert_eq!(score(&hand), 3);
    }

    #[test]
    fn run_of_four_scores_four() {
        let hand = cards([Rank::Four, Rank::Five, Rank::Six, Rank::Seven, Rank::King]);
        assert_eq!(score(&hand), 4);
    }

    #[test]
    fn run_of_five_scores_five() {
        let hand = cards([Rank::Four, Rank::Five, Rank::Six, Rank::Seven, Rank::Eight]);
        assert_eq!(score(&hand), 5);
    }

    #[test]
    fn double_run_of_three_scores_six() {
        // 4, 5, 5, 6, plus an unrelated card.
        let hand = cards([Rank::Four, Rank::Five, Rank::Five, Rank::Six, Rank::King]);
        assert_eq!(score(&hand), 6);
    }

    #[test]
    fn triple_run_of_three_scores_nine() {
        // 4, 5, 5, 5, 6 — three 5s.
        let hand = cards([Rank::Four, Rank::Five, Rank::Five, Rank::Five, Rank::Six]);
        assert_eq!(score(&hand), 9);
    }

    #[test]
    fn double_double_run_scores_twelve() {
        // 1, 2, 2, 3, 3 — two pairs inside a run of 3.
        let hand = cards([Rank::Ace, Rank::Two, Rank::Two, Rank::Three, Rank::Three]);
        assert_eq!(score(&hand), 12);
    }

    #[test]
    fn double_run_of_four_scores_eight() {
        // 4, 5, 6, 6, 7
        let hand = cards([Rank::Four, Rank::Five, Rank::Six, Rank::Six, Rank::Seven]);
        assert_eq!(score(&hand), 8);
    }

    #[test]
    fn runs_use_rank_ord_not_value() {
        // 10, J, Q — consecutive via rank_ord (10,11,12) though all
        // value == 10.
        let hand = cards([Rank::Ten, Rank::Jack, Rank::Queen, Rank::Two, Rank::Five]);
        assert_eq!(score(&hand), 3);
    }

    #[test]
    fn no_wraparound_king_ace_two() {
        let hand = cards([Rank::King, Rank::Ace, Rank::Two, Rank::Five, Rank::Seven]);
        assert_eq!(score(&hand), 0);
    }

    #[test]
    fn ace_two_three_is_a_run_with_ace_low() {
        let hand = cards([Rank::Ace, Rank::Two, Rank::Three, Rank::Eight, Rank::King]);
        assert_eq!(score(&hand), 3);
    }
}
