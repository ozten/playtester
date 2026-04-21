//! Evaluation function for Cribbage one-ply agents.
//!
//! Signature (per `playtest_agents::eval::EvalFn`):
//! `fn(view: &PublicView, player: PlayerId) -> f64`.
//!
//! Higher = better for `player`. Weights are tuned empirically to clear
//! the R2.2 bar (heuristic beats random > 90% over 10K games).
//!
//! # Design notes
//!
//! Greedy's one-ply simulation compares views *after* the candidate
//! action has been applied. So the eval's job is to score a given
//! public view — not to rate an action. Practically that means the
//! strongest signals come from:
//!
//! - **board score delta** — pegging and show events bump the board.
//!   A view where our score jumped is strictly better.
//! - **hand value** — after discard, the 4 kept cards' show potential.
//! - **opponent's threat** — their score counts against us.
//! - **pegging tactics** — running-total + own-hand inform whether
//!   we can land 31 / 15 next.
//!
//! The main trap for a hand-tuned eval is letting a secondary signal
//! (hand EV) drown the primary one (just scored 6 points from a triple
//! during pegging). We weight pegging-score-delta heavily to avoid that.

use playtest_core::PlayerId;

use crate::board::WINNING_SCORE;
use crate::card::{Card, Rank};
use crate::hand::Hand;
use crate::phase::Phase;
use crate::rules::PublicView;
use crate::scoring::show::score_hand;

/// Strong signal: own board score (direct primary).
const W_OWN_BOARD: f64 = 4.0;
/// Mirror signal: opponent board score.
const W_OPP_BOARD: f64 = -4.0;
/// Hand EV signal: weight the best-4-card show value of own hand.
const W_HAND_EV: f64 = 1.5;
/// Crib-discard bias: when we're throwing to our own crib, value the
/// pair/run/fifteen structure left behind vs. given up.
const W_CRIB_BIAS: f64 = 0.8;
/// "Just made progress" saturation: reward a strong final-score proxy
/// so we don't suicide for short-term 2s if it lets opponent win.
const W_WIN_PROXIMITY: f64 = 2.0;

/// Score the Cribbage public view from `player`'s perspective.
#[must_use]
pub fn cribbage_eval(view: &PublicView, player: PlayerId) -> f64 {
    let opponent = 1 - player;
    let own_score = f64::from(view.board.score(player));
    let opp_score = f64::from(view.board.score(opponent));

    let hand_ev = approximate_hand_value(&view.own_hand, view.starter);

    // Win proximity: reward getting close to 121 *more* per point than
    // the linear board term alone. Quadratic on own progress encourages
    // finishing; quadratic penalty on opponent progress encourages
    // blocking.
    let own_progress = own_score / f64::from(WINNING_SCORE);
    let opp_progress = opp_score / f64::from(WINNING_SCORE);
    let win_proximity = own_progress * own_progress - opp_progress * opp_progress;

    let crib_bias = crib_bias_score(view, player);

    W_OWN_BOARD * own_score
        + W_OPP_BOARD * opp_score
        + W_HAND_EV * hand_ev
        + W_CRIB_BIAS * crib_bias
        + W_WIN_PROXIMITY * win_proximity * 100.0
}

/// Return the approximate show value of the player's hand — best 4-card
/// subset against the actual (or average) starter. Normalized to [0, 29]
/// which is the theoretical cribbage maximum.
fn approximate_hand_value(hand: &Hand, starter: Option<Card>) -> f64 {
    let cards = hand.cards();
    if cards.len() < 4 {
        return 0.0;
    }
    let starter = starter.unwrap_or_else(|| {
        // Proxy starter — 5 is the single most-valuable rank because it
        // combines with every 10-count card for 15s.
        Card::new(Rank::Five, crate::card::Suit::Clubs)
    });
    f64::from(best_four_card_show(cards, starter))
}

/// Best 4-card subset score against `starter`. For a 4-card hand, it's
/// just `score_hand`. For 5- or 6-card hands, enumerate C(n,4) subsets.
fn best_four_card_show(cards: &[Card], starter: Card) -> u8 {
    let n = cards.len();
    if n < 4 {
        return 0;
    }
    if n == 4 {
        return score_hand([cards[0], cards[1], cards[2], cards[3]], starter, false).total;
    }
    let mut best: u8 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                for l in (k + 1)..n {
                    let arr = [cards[i], cards[j], cards[k], cards[l]];
                    let s = score_hand(arr, starter, false).total;
                    if s > best {
                        best = s;
                    }
                }
            }
        }
    }
    best
}

/// Reward being "ahead" during discard/pegging by a small amount when
/// nearing 31 with a 10-count-matching card. This is a cheap tactical
/// signal that works when the primary board signal is stable.
fn crib_bias_score(view: &PublicView, _player: PlayerId) -> f64 {
    match view.phase {
        Phase::Pegging => {
            // Favor still-holding-a-card states near the 31 magnet.
            let total = view.running_total;
            if (21..=24).contains(&total) {
                let target = 31u8.saturating_sub(total);
                let has_match = view
                    .own_hand
                    .cards()
                    .iter()
                    .any(|c| c.value() == target);
                if has_match {
                    return 2.0;
                }
            }
            // Any card played (own_hand smaller) is progress toward the
            // show phase — tiny positive nudge.
            f64::from(u8::try_from(view.own_hand.len().min(4)).unwrap_or(0)) * 0.1
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::card::{Rank, Suit};
    use crate::phase::Phase;

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    fn view_with_board(me_score: u16, opp_score: u16) -> PublicView {
        let mut board = Board::new();
        board.advance(0, me_score);
        board.advance(1, opp_score);
        PublicView {
            player: 0,
            own_hand: Hand::empty(),
            crib_size: 0,
            starter: None,
            pegging_stack: Vec::new(),
            running_total: 0,
            board,
            phase: Phase::Deal,
            to_act: 0,
        }
    }

    #[test]
    fn higher_own_score_scores_higher() {
        let weak = view_with_board(10, 10);
        let strong = view_with_board(80, 10);
        assert!(cribbage_eval(&strong, 0) > cribbage_eval(&weak, 0));
    }

    #[test]
    fn higher_opp_score_scores_lower() {
        let both_low = view_with_board(10, 10);
        let opp_leading = view_with_board(10, 80);
        assert!(cribbage_eval(&both_low, 0) > cribbage_eval(&opp_leading, 0));
    }

    #[test]
    fn strong_hand_beats_dead_hand_at_same_board_state() {
        let good = Hand::new(vec![
            c(Rank::Five, Suit::Spades),
            c(Rank::Five, Suit::Clubs),
            c(Rank::Six, Suit::Diamonds),
            c(Rank::Seven, Suit::Hearts),
        ]);
        let junk = Hand::new(vec![
            c(Rank::Two, Suit::Spades),
            c(Rank::Four, Suit::Hearts),
            c(Rank::Nine, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
        ]);

        let mut v_good = view_with_board(10, 10);
        v_good.own_hand = good;
        let mut v_junk = view_with_board(10, 10);
        v_junk.own_hand = junk;

        assert!(cribbage_eval(&v_good, 0) > cribbage_eval(&v_junk, 0));
    }
}
