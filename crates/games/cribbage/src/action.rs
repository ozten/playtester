//! Agent-chosen actions in Cribbage.
//!
//! Actions are the *intent* an agent expresses. The engine validates
//! them (is this card in my hand? does this play stay under 31?) and
//! converts them into [`crate::event::Event`]s. Only events touch
//! state; actions are the handshake between agent and engine.

use serde::{Deserialize, Serialize};

use crate::card::Card;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Discard two cards from a 6-card hand into the crib. Legal only
    /// during [`crate::phase::Phase::Discard`].
    DiscardToCrib(Card, Card),

    /// Play a card during pegging. Legal only during
    /// [`crate::phase::Phase::Pegging`], and only if the card is in
    /// the player's remaining hand and the play does not push the
    /// running total past 31.
    PlayCard(Card),

    /// Say "Go" during pegging. Legal only when the player cannot
    /// legally play any card. Returning this when a legal play exists
    /// is a cheating attempt and rejected by the engine.
    SayGo,
}
