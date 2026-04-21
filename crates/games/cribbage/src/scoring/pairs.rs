//! Show-scoring rule: **pairs**.
//!
//! Every distinct pair of cards sharing a rank scores 2 points. For a
//! rank with `c` cards present, there are `C(c,2) = c*(c-1)/2` pairs
//! (and `c*(c-1)` points). So: pair = 2, triple = 6, quadruple = 12.

use crate::card::Card;

/// Score the pairs across the counting set.
#[must_use]
pub fn score(cards: &[Card; 5]) -> u8 {
    let mut counts: [u8; 14] = [0; 14];
    for c in cards {
        counts[c.rank_ord() as usize] += 1;
    }
    let mut total: u8 = 0;
    for &c in &counts {
        if c >= 2 {
            total = total.saturating_add(c * (c - 1));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    #[test]
    fn no_pairs_gives_zero() {
        let cards = [
            c(Rank::Two, Suit::Spades),
            c(Rank::Four, Suit::Hearts),
            c(Rank::Six, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
            c(Rank::Queen, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 0);
    }

    #[test]
    fn single_pair_scores_two() {
        let cards = [
            c(Rank::Seven, Suit::Spades),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
            c(Rank::Queen, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 2);
    }

    #[test]
    fn triple_scores_six() {
        let cards = [
            c(Rank::Seven, Suit::Spades),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Seven, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
            c(Rank::Queen, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 6);
    }

    #[test]
    fn quadruple_scores_twelve() {
        let cards = [
            c(Rank::Seven, Suit::Spades),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Seven, Suit::Diamonds),
            c(Rank::Seven, Suit::Clubs),
            c(Rank::Queen, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 12);
    }

    #[test]
    fn two_separate_pairs_score_four() {
        let cards = [
            c(Rank::Seven, Suit::Spades),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Three, Suit::Diamonds),
            c(Rank::Three, Suit::Clubs),
            c(Rank::Queen, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 4);
    }

    #[test]
    fn face_cards_do_not_pair_despite_equal_values() {
        // J, Q both value=10 but different rank_ord. Not a pair.
        let cards = [
            c(Rank::Jack, Suit::Spades),
            c(Rank::Queen, Suit::Hearts),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::Three, Suit::Clubs),
            c(Rank::Four, Suit::Hearts),
        ];
        assert_eq!(score(&cards), 0);
    }
}
