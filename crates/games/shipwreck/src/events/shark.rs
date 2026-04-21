//! Shark resolution.
//!
//! Per `docs/shipwreck.md` a shark "attacks a player and an invension
//! [sic] or raft extension." The caster picks the target player and
//! the precise slot; base rafts are never legal targets (they are
//! structurally protected by the spec). If the target player has a
//! Steel Cordage upgrade *anywhere* on their raft, the cordage defends
//! and is destroyed instead.
//!
//! Legal targets for a shark play are enumerated by
//! [`legal_shark_targets`] — they go into `legal_actions` as
//! `Action::PlayEventCard { card: Shark, target: … }`.
//!
//! Resolution is immediate: the engine emits `EventCardPlayed` *and*
//! `EventResolved` in the same `apply_action` response, so there is
//! never a `Phase::ResolvingEvent` entry for a shark.

use playtest_core::PlayerId;

use crate::action::{EventCardKind, EventTarget};
use crate::card::{Card, EquipmentKind, EventCard};
use crate::event::{Event, EventOutcome};
use crate::raft::SlotId;
use crate::state::{GameState, PlayerState};

/// All legal shark targets against `target_player` from `caster`'s
/// point of view. `caster == target_player` is never legal
/// (self-targeting is spec-forbidden); base-raft slots are never
/// legal; a slot is legal iff it is either an extension position or a
/// non-base slot that currently carries an upgrade.
#[must_use]
pub(crate) fn legal_shark_targets_against(
    caster: PlayerId,
    target_player: PlayerId,
    target_state: &PlayerState,
) -> Vec<EventTarget> {
    if caster == target_player {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Extension slots — each one is a losable "structure" regardless
    // of whether it carries an upgrade (the extension itself is the
    // prize the shark chomps).
    for i in 0..target_state.raft.extensions.len() {
        let idx = u16::try_from(i).expect("extension index fits in u16");
        out.push(EventTarget::SingleSlot {
            player: target_player,
            slot: SlotId::Extension(idx),
        });
    }
    // Base slots that carry an upgrade: the base itself is protected
    // but the upgrade on top of it is fair game.
    for slot in [SlotId::BaseLeft, SlotId::BaseRight] {
        if target_state.raft.upgrade_at(slot).is_some() {
            out.push(EventTarget::SingleSlot {
                player: target_player,
                slot,
            });
        }
    }
    out
}

/// All legal shark plays across every opponent. Flat list — one
/// [`EventTarget`] per (opponent × losable slot).
#[must_use]
pub(crate) fn all_legal_shark_targets(
    caster: PlayerId,
    state: &GameState,
) -> Vec<EventTarget> {
    let mut out = Vec::new();
    for (i, p) in state.players.iter().enumerate() {
        let pid = u8::try_from(i).expect("seat fits in u8");
        if pid == caster {
            continue;
        }
        out.extend(legal_shark_targets_against(caster, pid, p));
    }
    out
}

/// Build the paired `[EventCardPlayed, EventResolved]` events for a
/// shark play, applying Steel Cordage defense if it is installed on
/// the target player's raft. Assumes the target has already been
/// validated against [`all_legal_shark_targets`].
#[must_use]
pub(crate) fn apply_shark(
    state: &GameState,
    caster: PlayerId,
    target: EventTarget,
) -> Vec<Event> {
    let (target_player, nominated_slot) = match target {
        EventTarget::SingleSlot { player, slot } => (player, slot),
        EventTarget::None => {
            // Should be caught in `legal_actions`/`apply_action`
            // validation before we get here; fall back to a no-op
            // play so the state machine doesn't wedge.
            return vec![Event::EventCardPlayed {
                player: caster,
                card: EventCardKind::Shark,
                target,
            }];
        }
    };

    let played = Event::EventCardPlayed {
        player: caster,
        card: EventCardKind::Shark,
        target,
    };

    // Scan the target player's raft for Steel Cordage. If found, the
    // cordage defends and is destroyed instead of the nominated slot.
    let victim = &state.players[target_player as usize];
    let cordage_slot = victim
        .raft
        .upgrades
        .iter()
        .find(|(_, eq)| eq.kind == EquipmentKind::SteelCordage)
        .map(|(slot, _)| *slot);

    let resolved = if let Some(_cordage) = cordage_slot {
        Event::EventResolved {
            player: caster,
            outcome: EventOutcome::SharkDefended {
                target: target_player,
            },
        }
    } else {
        Event::EventResolved {
            player: caster,
            outcome: EventOutcome::SharkDestroyed {
                target: target_player,
                slot: nominated_slot,
            },
        }
    };

    vec![played, resolved]
}

/// Apply the state mutation for an `EventResolved` emitted by a shark
/// play. Splits on `EventOutcome` so `apply_event_impl` in `rules.rs`
/// can dispatch into it without a huge nested match.
pub(crate) fn apply_shark_resolution(
    state: &mut GameState,
    outcome: EventOutcome,
) {
    match outcome {
        EventOutcome::SharkDefended { target } => {
            // Find & destroy the Steel Cordage upgrade on `target`'s
            // raft. We search `raft.upgrades` rather than assuming a
            // fixed slot — the cordage can live on any slot.
            let victim = &mut state.players[target as usize];
            if let Some(slot) = victim
                .raft
                .upgrades
                .iter()
                .find(|(_, eq)| eq.kind == EquipmentKind::SteelCordage)
                .map(|(s, _)| *s)
            {
                victim.raft.upgrades.remove(&slot);
            }
        }
        EventOutcome::SharkDestroyed { target, slot } => {
            destroy_slot(&mut state.players[target as usize], slot);
        }
        // Non-shark outcomes are handled elsewhere.
        _ => {}
    }
}

/// Remove whatever sits on `slot` — upgrade, player card, and (if the
/// slot is an extension) the extension itself, re-indexing downstream
/// extensions as needed. Base slots only drop their upgrade; the base
/// is not forfeitable.
pub(crate) fn destroy_slot(player_state: &mut PlayerState, slot: SlotId) {
    // 1. Drop any player card sitting on the slot.
    if let Some(pos) = player_state
        .played_players
        .iter()
        .position(|pp| pp.slot == slot)
    {
        player_state.played_players.remove(pos);
    }
    // 2. Drop the upgrade, if any.
    player_state.raft.upgrades.remove(&slot);

    // 3. If the slot is an extension, remove the extension card and
    //    re-index every later extension's upgrades / placed player
    //    cards that were anchored to a later slot.
    if let SlotId::Extension(removed_idx) = slot {
        let removed_pos = usize::from(removed_idx);
        if removed_pos < player_state.raft.extensions.len() {
            player_state.raft.extensions.remove(removed_pos);
            // Re-key upgrades on extensions with index > removed_pos
            // to shift left by one.
            let old = std::mem::take(&mut player_state.raft.upgrades);
            for (old_slot, eq) in old {
                let new_slot = match old_slot {
                    SlotId::Extension(i) if usize::from(i) > removed_pos => {
                        SlotId::Extension(
                            i.checked_sub(1).expect("extension index > 0 here"),
                        )
                    }
                    other => other,
                };
                player_state.raft.upgrades.insert(new_slot, eq);
            }
            // Re-key placed player cards the same way.
            for pp in &mut player_state.played_players {
                if let SlotId::Extension(i) = pp.slot
                    && usize::from(i) > removed_pos
                {
                    pp.slot = SlotId::Extension(
                        i.checked_sub(1).expect("extension index > 0 here"),
                    );
                }
            }
        }
    }
}

/// Remove the first `Card::Event(EventCard::Shark)` found in `hand`.
/// Caller is responsible for appending to `state.discarded_event_cards`.
pub(crate) fn discard_shark_from_hand(player_state: &mut PlayerState) -> bool {
    if let Some(pos) = player_state
        .hand
        .iter()
        .position(|c| matches!(c, Card::Event(EventCard::Shark)))
    {
        player_state.hand.remove(pos);
        true
    } else {
        false
    }
}
