//! 52-card deck construction and shuffling.
//!
//! All randomness flows through the [`Rng`] port — no direct
//! `thread_rng`, no `rand::random`, per the project's determinism
//! invariant.

use playtest_ports::{Rng, RngError};

use crate::card::{Card, Rank, Suit};

/// Number of cards in a standard French-suited deck.
pub const DECK_SIZE: usize = 52;

/// Produce a fresh 52-card deck in canonical order: all Clubs A..K,
/// then Diamonds A..K, Hearts A..K, Spades A..K. Tests and record
/// tapes depend on this ordering being stable.
#[must_use]
pub fn fresh() -> [Card; DECK_SIZE] {
    let mut out = [Card::new(Rank::Ace, Suit::Clubs); DECK_SIZE];
    let mut i = 0;
    for suit in Suit::ALL {
        for rank in Rank::ALL {
            out[i] = Card::new(rank, suit);
            i += 1;
        }
    }
    out
}

/// Shuffle a deck in place via Fisher-Yates, drawing randomness from
/// the [`Rng`] port.
///
/// Implemented by hand (rather than calling [`Rng::shuffle`]) because
/// the trait's default shuffle has a `Self: Sized` bound and is
/// unavailable through `&mut dyn Rng`. See the `Rng` trait docs.
///
/// # Errors
/// Propagates [`RngError`] from the underlying port.
pub fn shuffle(deck: &mut [Card; DECK_SIZE], rng: &mut dyn Rng) -> Result<(), RngError> {
    let n = deck.len();
    if n < 2 {
        return Ok(());
    }
    let mut i = n - 1;
    while i > 0 {
        let upper = u64::try_from(i).expect("usize fits in u64") + 1;
        let j_u64 = rng.gen_range(0..upper)?;
        let j = usize::try_from(j_u64).expect("j < upper fits in usize");
        deck.swap(i, j);
        i -= 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn fresh_deck_has_fifty_two_unique_cards() {
        let deck = fresh();
        assert_eq!(deck.len(), 52);
        let set: HashSet<_> = deck.iter().collect();
        assert_eq!(set.len(), 52, "deck contains duplicates");
    }

    #[test]
    fn fresh_deck_covers_every_rank_suit_combination() {
        let deck = fresh();
        for suit in Suit::ALL {
            for rank in Rank::ALL {
                assert!(
                    deck.iter().any(|c| c.rank == rank && c.suit == suit),
                    "missing {rank:?} of {suit:?}"
                );
            }
        }
    }
}
