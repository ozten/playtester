//! Typhoon resolution.
//!
//! A typhoon is a multi-player resolution: every seat in turn order,
//! starting with the caster, must sacrifice one upgrade or extension
//! (or pass, if they have nothing to lose). We model this as a
//! `PendingEvent::Typhoon { remaining_resolvers }` on the
//! `event_resolution_stack`; `next_actor` peeks the queue front, and
//! each `ResolveEvent` action pops the front on `apply_event`.
//!
//! `TyphoonPass` is legal *only* when the player owns no extensions
//! and no upgrades — ensuring the legal-actions set is never empty
//! during resolution (at minimum `[TyphoonPass]` is offered) and
//! preventing a player from "opting out" when they have losable
//! resources.

use std::collections::VecDeque;

use playtest_core::PlayerId;

use crate::action::{EventCardKind, EventResolution, EventTarget};
use crate::card::{Card, EventCard};
use crate::event::{Event, EventOutcome};
use crate::raft::SlotId;
use crate::state::{GameState, PlayerState};

use super::shark::destroy_slot;

/// Build the `[EventCardPlayed]` event for a typhoon play. Typhoons
/// do not immediately resolve — the follow-up work happens via
/// subsequent `ResolveEvent` actions once the engine transitions to
/// `Phase::ResolvingEvent`.
#[must_use]
pub(crate) fn apply_typhoon(caster: PlayerId) -> Vec<Event> {
    vec![Event::EventCardPlayed {
        player: caster,
        card: EventCardKind::Typhoon,
        target: EventTarget::None,
    }]
}

/// The seat order for a typhoon resolution, starting with the
/// initiator and wrapping through every other seat. All seats
/// participate, even the caster — per the Unit 23 spec.
#[must_use]
pub(crate) fn typhoon_resolver_order(
    initiator: PlayerId,
    num_players: u8,
) -> VecDeque<PlayerId> {
    let mut q = VecDeque::with_capacity(num_players as usize);
    let n = num_players;
    for offset in 0..n {
        q.push_back((initiator + offset) % n);
    }
    q
}

/// Which `EventResolution`s are legal for `player` mid-typhoon. If the
/// player owns at least one extension or upgrade, they must
/// `TyphoonLose(slot)` for some losable slot; if they own none,
/// `TyphoonPass` is the only option. The set is never empty.
#[must_use]
pub(crate) fn legal_typhoon_resolutions(
    player_state: &PlayerState,
) -> Vec<EventResolution> {
    let mut out = Vec::new();
    // Every extension slot (by index) is losable.
    for i in 0..player_state.raft.extensions.len() {
        let idx = u16::try_from(i).expect("extension index fits in u16");
        out.push(EventResolution::TyphoonLose(SlotId::Extension(idx)));
    }
    // Upgrades on base slots are losable (the base is protected, but
    // the upgrade on top of it is not).
    for slot in [SlotId::BaseLeft, SlotId::BaseRight] {
        if player_state.raft.upgrade_at(slot).is_some() {
            out.push(EventResolution::TyphoonLose(slot));
        }
    }
    if out.is_empty() {
        out.push(EventResolution::TyphoonPass);
    }
    out
}

/// True iff `slot` is a legal typhoon sacrifice for this player — i.e.
/// it's either an extension index that currently exists or a base slot
/// that carries an upgrade.
#[must_use]
pub(crate) fn is_legal_typhoon_slot(
    player_state: &PlayerState,
    slot: SlotId,
) -> bool {
    match slot {
        SlotId::Extension(i) => usize::from(i) < player_state.raft.extensions.len(),
        SlotId::BaseLeft | SlotId::BaseRight => {
            player_state.raft.upgrade_at(slot).is_some()
        }
    }
}

/// Build the `[EventResolved]` event for one player's typhoon
/// response. Caller is responsible for having validated legality via
/// [`legal_typhoon_resolutions`] / [`is_legal_typhoon_slot`].
#[must_use]
pub(crate) fn apply_typhoon_resolution(
    resolver: PlayerId,
    resolution: EventResolution,
) -> Vec<Event> {
    let outcome = match resolution {
        EventResolution::TyphoonLose(slot) => EventOutcome::TyphoonLost {
            player: resolver,
            slot,
        },
        EventResolution::TyphoonPass => EventOutcome::TyphoonPass { player: resolver },
    };
    vec![Event::EventResolved {
        player: resolver,
        outcome,
    }]
}

/// Apply the state mutation for an `EventResolved` emitted by a
/// typhoon step — destroy the named slot on the resolver's raft (or
/// do nothing on a pass), then pop the front of the pending-event
/// queue. When the queue drains, the pending event is popped off the
/// stack and the phase returns to `Phase::Play` with `current_player`
/// restored to the initiator.
pub(crate) fn apply_typhoon_resolution_event(
    state: &mut GameState,
    outcome: EventOutcome,
) {
    match outcome {
        EventOutcome::TyphoonLost { player, slot } => {
            destroy_slot(&mut state.players[player as usize], slot);
        }
        EventOutcome::TyphoonPass { .. } => {
            // No state mutation — the pass itself is the outcome.
        }
        _ => return,
    }

    // Pop the front of the queue on the top pending event. If the
    // stack top is not a typhoon, or is empty, we bail out silently —
    // replay robustness only needs the typhoon-shaped mutations.
    if let Some(top) = state.event_resolution_stack.last_mut() {
        top.remaining_resolvers.pop_front();
        if top.remaining_resolvers.is_empty() {
            let finished = state
                .event_resolution_stack
                .pop()
                .expect("top exists — just read it");
            // Restore phase and current_player.
            state.phase = crate::phase::Phase::Play;
            state.current_player = finished.initiator;
        }
    }
}

/// Remove the first `Card::Event(EventCard::Typhoon)` found in `hand`.
/// Caller is responsible for appending to `state.discarded_event_cards`.
pub(crate) fn discard_typhoon_from_hand(player_state: &mut PlayerState) -> bool {
    if let Some(pos) = player_state
        .hand
        .iter()
        .position(|c| matches!(c, Card::Event(EventCard::Typhoon)))
    {
        player_state.hand.remove(pos);
        true
    } else {
        false
    }
}
