//! Pure scoring for the pegging phase.
//!
//! The state machine in [`crate::state`] takes care of stack
//! management, Go semantics, and stack resets. This module is the
//! referee for a single play: "given the stack after this card was
//! added and the resulting running total, what did the player score?"
//!
//! "Last card" is **not** scored here — it's scored by the state
//! machine when a round ends, because last-card only fires on stack
//! reset and depends on whether the final card hit exactly 31.

use serde::{Deserialize, Serialize};

use crate::card::Card;

/// A single reason a player scored during pegging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PegReason {
    /// Running total landed on 15. +2.
    Fifteen,
    /// Running total landed on 31. +2, and the round ends.
    ThirtyOne,
    /// Last two cards on the stack are the same rank. +2.
    Pair,
    /// Last three cards on the stack are the same rank. +6.
    Triple,
    /// Last four cards on the stack are the same rank. +12.
    Quadruple,
    /// The last `N` cards on the stack form a run of `N` consecutive
    /// ranks (in any order). +N.
    Run(u8),
    /// Scored only by [`crate::state`] on stack reset (not by
    /// [`score_peg_play`]). +1 when the round ends below 31. No
    /// last-card bonus at exactly 31 — the 31 itself already scores.
    LastCard,
}

impl PegReason {
    #[must_use]
    pub const fn points(self) -> u8 {
        match self {
            Self::Fifteen | Self::ThirtyOne | Self::Pair => 2,
            Self::Triple => 6,
            Self::Quadruple => 12,
            Self::Run(n) => n,
            Self::LastCard => 1,
        }
    }
}

/// Score the play that just happened.
///
/// Pre-conditions:
/// - `stack` is the full pegging stack *after* the current card was
///   pushed onto the end.
/// - `running_total` is the sum of every [`Card::value`] on the stack.
///
/// Returns every reason that applies, in a deterministic order:
/// 15/31 first, then pair/triple/quadruple, then run.
#[must_use]
pub fn score_peg_play(stack: &[Card], running_total: u8) -> Vec<PegReason> {
    let mut out = Vec::new();

    if running_total == 15 {
        out.push(PegReason::Fifteen);
    }
    if running_total == 31 {
        out.push(PegReason::ThirtyOne);
    }

    if let Some(group) = trailing_same_rank_group(stack) {
        match group {
            2 => out.push(PegReason::Pair),
            3 => out.push(PegReason::Triple),
            4 => out.push(PegReason::Quadruple),
            _ => {}
        }
    }

    if let Some(run_len) = longest_run_at_end(stack) {
        out.push(PegReason::Run(run_len));
    }

    out
}

/// How many cards at the end of `stack` share the last card's rank?
/// Returns `None` if the stack is empty.
fn trailing_same_rank_group(stack: &[Card]) -> Option<usize> {
    let last_rank = stack.last()?.rank;
    let mut count = 0;
    for c in stack.iter().rev() {
        if c.rank == last_rank {
            count += 1;
        } else {
            break;
        }
    }
    Some(count)
}

/// Longest run of 3+ consecutive ranks formed by the last `N` cards
/// on the stack (order within those `N` cards does not matter, but
/// they must be exactly the last `N`). Returns `None` if no run of
/// length 3+ is present.
///
/// Examples (stack → result):
/// - `[4, 9, 5, 6]` → `None` (a run of "5, 6" needs a 4 or 7, but 9
///   is in the way — the last 3 cards are `9, 5, 6`)
/// - `[9, 5, 6, 4]` → `Some(3)` (last 3 cards `5, 6, 4` sort to `4, 5, 6`)
/// - `[9, 5, 6, 4, 3]` → `Some(4)` (last 4 cards sort to `3, 4, 5, 6`)
fn longest_run_at_end(stack: &[Card]) -> Option<u8> {
    // Check from longest possible tail down, so we return the *longest*
    // run that covers the most-recent card.
    let max_tail = stack.len().min(7); // no run longer than 7 possible with a 52-card deck
    for len in (3..=max_tail).rev() {
        let tail = &stack[stack.len() - len..];
        let mut ords: Vec<u8> = tail.iter().map(|c| c.rank_ord()).collect();
        ords.sort_unstable();
        let mut ok = true;
        for w in ords.windows(2) {
            if w[1] != w[0] + 1 {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(u8::try_from(len).expect("len <= 7 fits in u8"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank) -> Card {
        Card::new(r, Suit::Clubs)
    }

    #[test]
    fn peg_reason_point_values_are_correct() {
        assert_eq!(PegReason::Fifteen.points(), 2);
        assert_eq!(PegReason::ThirtyOne.points(), 2);
        assert_eq!(PegReason::Pair.points(), 2);
        assert_eq!(PegReason::Triple.points(), 6);
        assert_eq!(PegReason::Quadruple.points(), 12);
        assert_eq!(PegReason::Run(3).points(), 3);
        assert_eq!(PegReason::Run(7).points(), 7);
        assert_eq!(PegReason::LastCard.points(), 1);
    }

    #[test]
    fn empty_stack_scores_nothing() {
        assert!(score_peg_play(&[], 0).is_empty());
    }

    #[test]
    fn fifteen_hit_scores_fifteen() {
        let stack = vec![c(Rank::Seven), c(Rank::Eight)];
        assert!(score_peg_play(&stack, 15).contains(&PegReason::Fifteen));
    }

    #[test]
    fn thirty_one_hit_scores_thirty_one() {
        let stack = vec![c(Rank::Ten), c(Rank::Ten), c(Rank::Ten), c(Rank::Ace)];
        assert!(score_peg_play(&stack, 31).contains(&PegReason::ThirtyOne));
    }

    #[test]
    fn trailing_pair_scores_pair() {
        let stack = vec![c(Rank::Three), c(Rank::Seven), c(Rank::Seven)];
        assert!(score_peg_play(&stack, 17).contains(&PegReason::Pair));
    }

    #[test]
    fn trailing_triple_scores_triple_not_pair() {
        let stack = vec![c(Rank::Seven), c(Rank::Seven), c(Rank::Seven)];
        let reasons = score_peg_play(&stack, 21);
        assert!(reasons.contains(&PegReason::Triple));
        assert!(!reasons.contains(&PegReason::Pair));
    }
}
