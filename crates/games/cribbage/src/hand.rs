//! A player's hand: an ordered `Vec<Card>` with the helpers the game
//! logic needs during deal / discard / pegging / show.

use serde::{Deserialize, Serialize};

use crate::card::Card;

/// Errors produced by [`Hand`] operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandError {
    /// A [`Hand::remove`] call named a card that wasn't in the hand.
    /// Surfacing this rather than silently no-oping catches bugs where
    /// the caller's state and the hand's state have drifted.
    #[error("card not in hand: {0}")]
    NotInHand(Card),
}

/// A player's hand. Order is preserved (insertion order by default);
/// [`Hand::sorted_by_rank`] returns a sorted copy when needed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hand(Vec<Card>);

impl Hand {
    #[must_use]
    pub fn new(cards: Vec<Card>) -> Self {
        Self(cards)
    }

    #[must_use]
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn cards(&self) -> &[Card] {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn contains(&self, card: Card) -> bool {
        self.0.contains(&card)
    }

    pub fn push(&mut self, card: Card) {
        self.0.push(card);
    }

    /// Remove `card` from the hand.
    ///
    /// # Errors
    /// Returns [`HandError::NotInHand`] if `card` is absent. Does not
    /// silently no-op — callers should know if their state is wrong.
    pub fn remove(&mut self, card: Card) -> Result<(), HandError> {
        match self.0.iter().position(|&c| c == card) {
            Some(i) => {
                self.0.remove(i);
                Ok(())
            }
            None => Err(HandError::NotInHand(card)),
        }
    }

    /// Return a copy sorted by [`Card::rank_ord`] ascending. Useful
    /// for show scoring (runs, pairs) where order matters and we
    /// don't want to mutate the caller's hand.
    #[must_use]
    pub fn sorted_by_rank(&self) -> Hand {
        let mut v = self.0.clone();
        v.sort_by_key(|c| c.rank_ord());
        Hand(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn card(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    #[test]
    fn new_and_empty_behave_as_expected() {
        assert!(Hand::empty().is_empty());
        let h = Hand::new(vec![card(Rank::Ace, Suit::Clubs)]);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn remove_succeeds_when_card_present() {
        let mut h = Hand::new(vec![
            card(Rank::Ace, Suit::Clubs),
            card(Rank::Two, Suit::Diamonds),
        ]);
        h.remove(card(Rank::Ace, Suit::Clubs)).unwrap();
        assert_eq!(h.len(), 1);
        assert!(!h.contains(card(Rank::Ace, Suit::Clubs)));
    }

    #[test]
    fn sorted_by_rank_orders_aces_low_kings_high_without_mutating_self() {
        let h = Hand::new(vec![
            card(Rank::King, Suit::Spades),
            card(Rank::Ace, Suit::Hearts),
            card(Rank::Seven, Suit::Diamonds),
        ]);
        let sorted = h.sorted_by_rank();
        assert_eq!(sorted.cards()[0].rank, Rank::Ace);
        assert_eq!(sorted.cards()[1].rank, Rank::Seven);
        assert_eq!(sorted.cards()[2].rank, Rank::King);
        // Original unchanged.
        assert_eq!(h.cards()[0].rank, Rank::King);
    }
}
