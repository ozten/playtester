//! Flying Fish resolution.
//!
//! Per `docs/shipwreck.md`, Flying Fish reads "Treat as extra food for
//! one turn." We model that as an immediate `+1` to the caster's
//! `food_counter`. The extra food is indistinguishable from any other
//! food on the counter once it lands; it will be spent on the next
//! `EndTurn` food-consumption pass (the card effect lasts "one turn"
//! because end-of-turn consumption uses it up).
//!
//! Resolution is immediate, so `apply_action` emits both the
//! `EventCardPlayed` and `EventResolved` events together. No entry to
//! [`crate::phase::Phase::ResolvingEvent`].

use playtest_core::PlayerId;

use crate::action::{EventCardKind, EventTarget};
use crate::card::{Card, EventCard};
use crate::event::{Event, EventOutcome};
use crate::state::{GameState, PlayerState};

/// Fixed amount of food granted per Flying Fish play. Spec says "one
/// turn" → one food counter unit; the card has no other effect.
pub(crate) const FLYING_FISH_FOOD: i16 = 1;

/// Build the `[EventCardPlayed, EventResolved]` pair for a Flying
/// Fish play.
#[must_use]
pub(crate) fn apply_flying_fish(_state: &GameState, caster: PlayerId) -> Vec<Event> {
    vec![
        Event::EventCardPlayed {
            player: caster,
            card: EventCardKind::FlyingFish,
            target: EventTarget::None,
        },
        Event::EventResolved {
            player: caster,
            outcome: EventOutcome::FlyingFishGranted { player: caster },
        },
    ]
}

/// Apply the state mutation for a Flying Fish resolution — bump the
/// caster's food counter by [`FLYING_FISH_FOOD`].
pub(crate) fn apply_flying_fish_resolution(state: &mut GameState, caster: PlayerId) {
    let me = &mut state.players[caster as usize];
    me.food_counter = me.food_counter.saturating_add(FLYING_FISH_FOOD);
}

/// Remove the first `Card::Event(EventCard::FlyingFish)` found in
/// `hand`. Caller is responsible for appending to
/// `state.discarded_event_cards`.
pub(crate) fn discard_flying_fish_from_hand(player_state: &mut PlayerState) -> bool {
    if let Some(pos) = player_state
        .hand
        .iter()
        .position(|c| matches!(c, Card::Event(EventCard::FlyingFish)))
    {
        player_state.hand.remove(pos);
        true
    } else {
        false
    }
}
