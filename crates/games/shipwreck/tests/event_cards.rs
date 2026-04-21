//! Unit 23 event-card integration tests.
//!
//! Covers all three event cards (Shark, Typhoon, FlyingFish) across
//! their happy paths, edge cases, and error paths, plus a typhoon
//! replay-correctness check and a random self-play integration.

use std::collections::VecDeque;

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game, GameError, GameLoop, PlayerId};
use playtest_shipwreck::{
    Action, Card, EquipmentCard, EquipmentKind, Event, EventCard, EventCardKind, EventOutcome,
    EventResolution, EventTarget, GameState, PendingEvent, Phase, RaftExtensionCard, ShipWreckConfig,
    ShipWreckGame, SlotId,
};

// ---------- fixtures -----------------------------------------------------

fn two_player_initial() -> (ShipWreckGame, GameState) {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let state = game.initial_state(11, &cfg);
    (game, state)
}

fn three_player_initial() -> (ShipWreckGame, GameState) {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::new(3).unwrap();
    let state = game.initial_state(13, &cfg);
    (game, state)
}

/// Drop the entire hand for `player`, then push the given cards.
fn set_hand(state: &mut GameState, player: PlayerId, cards: Vec<Card>) {
    state.players[player as usize].hand = cards;
}

/// Install an extension with the given serial immediately after the
/// base-left. Returns the `SlotId` of the new extension.
fn install_extension(state: &mut GameState, player: PlayerId, serial: u16) -> SlotId {
    let me = &mut state.players[player as usize];
    me.raft
        .extend(RaftExtensionCard::new(serial), SlotId::BaseLeft)
        .unwrap();
    SlotId::Extension(0)
}

fn install_upgrade(
    state: &mut GameState,
    player: PlayerId,
    slot: SlotId,
    kind: EquipmentKind,
    serial: u16,
) {
    let me = &mut state.players[player as usize];
    me.raft
        .build_upgrade(slot, EquipmentCard::new(kind, serial))
        .unwrap();
}

fn apply_all(game: ShipWreckGame, state: &mut GameState, events: &[Event]) {
    for e in events {
        game.apply_event(state, e);
    }
}

// ---------- Shark tests --------------------------------------------------

#[test]
fn shark_destroys_nominated_extension_and_its_upgrade_cascades() {
    let (game, mut state) = two_player_initial();
    // Seat 0 has a Shark in hand.
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Shark)]);
    // Seat 1 has an extension with an upgrade on it — the target.
    let ext_slot = install_extension(&mut state, 1, 100);
    install_upgrade(&mut state, 1, ext_slot, EquipmentKind::AutoNets, 9);
    // And no Steel Cordage — so the shark lands.

    let action = Action::PlayEventCard {
        card: EventCardKind::Shark,
        target: EventTarget::SingleSlot {
            player: 1,
            slot: ext_slot,
        },
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");

    assert_eq!(events.len(), 2, "shark emits EventCardPlayed + EventResolved");
    assert!(matches!(events[0], Event::EventCardPlayed { card: EventCardKind::Shark, .. }));
    match &events[1] {
        Event::EventResolved { outcome: EventOutcome::SharkDestroyed { target, slot }, .. } => {
            assert_eq!(*target, 1);
            assert_eq!(*slot, ext_slot);
        }
        other => panic!("expected SharkDestroyed, got {other:?}"),
    }

    apply_all(game, &mut state, &events);
    assert!(state.players[1].raft.extensions.is_empty(), "extension gone");
    assert!(state.players[1].raft.upgrade_at(SlotId::BaseLeft).is_none());
    assert!(
        !state.players[0]
            .hand
            .iter()
            .any(|c| matches!(c, Card::Event(EventCard::Shark))),
        "shark discarded from caster hand"
    );
    assert_eq!(state.discarded_event_cards, vec![EventCard::Shark]);
}

#[test]
fn shark_on_steel_cordage_target_is_defended() {
    let (game, mut state) = two_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Shark)]);
    let ext_slot = install_extension(&mut state, 1, 200);
    // Steel Cordage somewhere on seat 1's raft.
    install_upgrade(&mut state, 1, SlotId::BaseRight, EquipmentKind::SteelCordage, 0);

    let action = Action::PlayEventCard {
        card: EventCardKind::Shark,
        target: EventTarget::SingleSlot {
            player: 1,
            slot: ext_slot,
        },
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");
    match &events[1] {
        Event::EventResolved { outcome: EventOutcome::SharkDefended { target }, .. } => {
            assert_eq!(*target, 1);
        }
        other => panic!("expected SharkDefended, got {other:?}"),
    }

    apply_all(game, &mut state, &events);
    // The cordage is destroyed, the extension survives.
    assert!(state.players[1].raft.upgrade_at(SlotId::BaseRight).is_none());
    assert_eq!(state.players[1].raft.extensions.len(), 1);
}

#[test]
fn shark_self_target_is_illegal() {
    let (game, mut state) = two_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Shark)]);
    let ext_slot = install_extension(&mut state, 0, 500);
    let action = Action::PlayEventCard {
        card: EventCardKind::Shark,
        target: EventTarget::SingleSlot {
            player: 0,
            slot: ext_slot,
        },
    };
    let err = game.apply_action(&state, 0, &action).unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn shark_on_opponent_with_no_losables_has_no_legal_target() {
    let (game, mut state) = two_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Shark)]);
    // Seat 1 has no extensions and no upgrades — just base rafts.
    let legal = game.legal_actions(&state, 0);
    assert!(
        !legal.iter().any(|a| matches!(
            a,
            Action::PlayEventCard { card: EventCardKind::Shark, .. }
        )),
        "no legal Shark plays when opponents have nothing to lose; got {legal:#?}"
    );
}

#[test]
fn play_event_card_not_in_hand_returns_illegal() {
    let (game, mut state) = two_player_initial();
    // Strip every event card from seat 0's hand.
    state.players[0]
        .hand
        .retain(|c| !matches!(c, Card::Event(_)));
    let action = Action::PlayEventCard {
        card: EventCardKind::Shark,
        target: EventTarget::SingleSlot {
            player: 1,
            slot: SlotId::BaseLeft,
        },
    };
    let err = game.apply_action(&state, 0, &action).unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

// ---------- Flying Fish tests -------------------------------------------

#[test]
fn flying_fish_increments_food_counter() {
    let (game, mut state) = two_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::FlyingFish)]);
    let original_food = state.players[0].food_counter;

    let action = Action::PlayEventCard {
        card: EventCardKind::FlyingFish,
        target: EventTarget::None,
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[1],
        Event::EventResolved {
            outcome: EventOutcome::FlyingFishGranted { player: 0 },
            ..
        }
    ));
    apply_all(game, &mut state, &events);
    assert_eq!(state.players[0].food_counter, original_food + 1);
    assert_eq!(state.phase, Phase::Play, "FlyingFish resolves immediately");
}

#[test]
fn flying_fish_rejects_non_empty_target() {
    let (game, mut state) = two_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::FlyingFish)]);
    let action = Action::PlayEventCard {
        card: EventCardKind::FlyingFish,
        target: EventTarget::SingleSlot {
            player: 1,
            slot: SlotId::BaseLeft,
        },
    };
    let err = game.apply_action(&state, 0, &action).unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

// ---------- Typhoon tests -----------------------------------------------

#[test]
fn typhoon_enters_resolving_event_and_seeds_all_players() {
    let (game, mut state) = three_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Typhoon)]);

    let action = Action::PlayEventCard {
        card: EventCardKind::Typhoon,
        target: EventTarget::None,
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");
    assert_eq!(events.len(), 1, "Typhoon only emits EventCardPlayed initially");
    apply_all(game, &mut state, &events);

    assert_eq!(state.phase, Phase::ResolvingEvent);
    let top = state.event_resolution_stack.last().expect("pending event present");
    // Initiator-first turn order: [0, 1, 2].
    let resolver_order: Vec<PlayerId> = top.remaining_resolvers.iter().copied().collect();
    assert_eq!(resolver_order, vec![0, 1, 2]);
    assert_eq!(top.initiator, 0);
}

#[test]
fn typhoon_resolve_cycle_drains_queue_and_restores_phase() {
    let (game, mut state) = three_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Typhoon)]);
    // Give seat 0 and seat 2 losable extensions; seat 1 has none.
    install_extension(&mut state, 0, 700);
    install_extension(&mut state, 2, 701);

    // Fire the typhoon.
    let events = game
        .apply_action(
            &state,
            0,
            &Action::PlayEventCard {
                card: EventCardKind::Typhoon,
                target: EventTarget::None,
            },
        )
        .unwrap();
    apply_all(game, &mut state, &events);
    assert_eq!(state.phase, Phase::ResolvingEvent);

    // Seat 0 resolves first.
    let a0 = game.legal_actions(&state, 0);
    assert!(
        a0.iter().all(|a| matches!(a, Action::ResolveEvent(_))),
        "only ResolveEvent legal, got {a0:?}"
    );
    let lose0 = Action::ResolveEvent(EventResolution::TyphoonLose(SlotId::Extension(0)));
    assert!(a0.contains(&lose0));
    let events = game.apply_action(&state, 0, &lose0).unwrap();
    apply_all(game, &mut state, &events);
    assert_eq!(state.phase, Phase::ResolvingEvent);

    // Seat 1 has nothing to lose → must pass.
    let a1 = game.legal_actions(&state, 1);
    assert_eq!(a1.len(), 1);
    assert_eq!(a1[0], Action::ResolveEvent(EventResolution::TyphoonPass));
    let events = game.apply_action(&state, 1, a1.first().unwrap()).unwrap();
    apply_all(game, &mut state, &events);
    assert_eq!(state.phase, Phase::ResolvingEvent);

    // Seat 2 loses its extension.
    let a2 = game.legal_actions(&state, 2);
    let lose2 = Action::ResolveEvent(EventResolution::TyphoonLose(SlotId::Extension(0)));
    assert!(a2.contains(&lose2), "seat 2 legal: {a2:?}");
    let events = game.apply_action(&state, 2, &lose2).unwrap();
    apply_all(game, &mut state, &events);

    // Queue drained: back to Play with seat 0 (initiator) current.
    assert_eq!(state.phase, Phase::Play);
    assert_eq!(state.current_player, 0);
    assert!(state.event_resolution_stack.is_empty());
    assert!(state.players[0].raft.extensions.is_empty());
    assert!(state.players[2].raft.extensions.is_empty());
}

#[test]
fn typhoon_pass_illegal_when_losable_items_exist() {
    let (game, mut state) = three_player_initial();
    // Bypass the PlayEventCard: hand-craft a pending typhoon on seat 0
    // with seat 0 at the front and an existing extension on seat 0.
    install_extension(&mut state, 0, 333);
    state.phase = Phase::ResolvingEvent;
    state.event_resolution_stack.push(PendingEvent::typhoon(
        VecDeque::from(vec![0_u8, 1, 2]),
        0,
    ));

    let err = game
        .apply_action(
            &state,
            0,
            &Action::ResolveEvent(EventResolution::TyphoonPass),
        )
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn typhoon_lose_on_nonexistent_slot_is_illegal() {
    let (game, mut state) = three_player_initial();
    state.phase = Phase::ResolvingEvent;
    state.event_resolution_stack.push(PendingEvent::typhoon(
        VecDeque::from(vec![0_u8, 1, 2]),
        0,
    ));
    // Seat 0 has no extensions.
    let err = game
        .apply_action(
            &state,
            0,
            &Action::ResolveEvent(EventResolution::TyphoonLose(SlotId::Extension(99))),
        )
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn resolve_event_during_play_phase_is_illegal() {
    let (game, state) = two_player_initial();
    let err = game
        .apply_action(
            &state,
            0,
            &Action::ResolveEvent(EventResolution::TyphoonPass),
        )
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn legal_actions_during_resolving_event_never_empty() {
    let (game, mut state) = three_player_initial();
    // Players with nothing: queue front is seat 1 (no extensions).
    state.phase = Phase::ResolvingEvent;
    state.event_resolution_stack.push(PendingEvent::typhoon(
        VecDeque::from(vec![1_u8, 2, 0]),
        0,
    ));
    let legal = game.legal_actions(&state, 1);
    assert!(!legal.is_empty());
    assert!(
        legal
            .iter()
            .all(|a| matches!(a, Action::ResolveEvent(_))),
        "only ResolveEvent allowed during ResolvingEvent, got {legal:?}"
    );
}

#[test]
fn next_actor_during_resolving_event_is_queue_front() {
    let (game, mut state) = three_player_initial();
    state.phase = Phase::ResolvingEvent;
    state.event_resolution_stack.push(PendingEvent::typhoon(
        VecDeque::from(vec![2_u8, 0, 1]),
        2,
    ));
    match game.next_actor(&state) {
        playtest_core::Actor::Player(p) => assert_eq!(p, 2),
        playtest_core::Actor::Chance => panic!("expected Player(2), got Chance"),
    }
}

// ---------- Replay correctness ------------------------------------------

#[test]
fn typhoon_replay_reconstructs_final_state() {
    let (game, mut state) = three_player_initial();
    set_hand(&mut state, 0, vec![Card::Event(EventCard::Typhoon)]);
    install_extension(&mut state, 0, 700);
    install_extension(&mut state, 2, 701);

    let mut log: Vec<Event> = Vec::new();

    let events = game
        .apply_action(
            &state,
            0,
            &Action::PlayEventCard {
                card: EventCardKind::Typhoon,
                target: EventTarget::None,
            },
        )
        .unwrap();
    log.extend(events.iter().cloned());
    apply_all(game, &mut state, &events);

    // seat 0: TyphoonLose(Ext0)
    let events = game
        .apply_action(
            &state,
            0,
            &Action::ResolveEvent(EventResolution::TyphoonLose(SlotId::Extension(0))),
        )
        .unwrap();
    log.extend(events.iter().cloned());
    apply_all(game, &mut state, &events);
    // seat 1: Pass
    let events = game
        .apply_action(
            &state,
            1,
            &Action::ResolveEvent(EventResolution::TyphoonPass),
        )
        .unwrap();
    log.extend(events.iter().cloned());
    apply_all(game, &mut state, &events);
    // seat 2: TyphoonLose(Ext0)
    let events = game
        .apply_action(
            &state,
            2,
            &Action::ResolveEvent(EventResolution::TyphoonLose(SlotId::Extension(0))),
        )
        .unwrap();
    log.extend(events.iter().cloned());
    apply_all(game, &mut state, &events);

    // Rebuild from a fresh initial state + replaying the log via
    // `apply_event` only.
    let (_, mut replayed) = three_player_initial();
    set_hand(&mut replayed, 0, vec![Card::Event(EventCard::Typhoon)]);
    install_extension(&mut replayed, 0, 700);
    install_extension(&mut replayed, 2, 701);
    apply_all(game, &mut replayed, &log);

    assert_eq!(replayed.phase, state.phase);
    assert_eq!(replayed.current_player, state.current_player);
    assert_eq!(replayed.event_resolution_stack, state.event_resolution_stack);
    assert_eq!(replayed.discarded_event_cards, state.discarded_event_cards);
    for seat in 0..3 {
        assert_eq!(
            replayed.players[seat].raft, state.players[seat].raft,
            "raft mismatch seat {seat}"
        );
    }
}

// ---------- Integration: random self-play with event cards --------------

#[tokio::test]
async fn random_self_play_2p_with_event_cards_terminates() {
    // Run a handful of random 2p games and confirm none panic. The
    // 1000-game soak test in `random_self_play.rs` is the full
    // integration — this test is the fast smoke.
    for seed in 0..20u64 {
        let game = ShipWreckGame::new();
        let cfg = ShipWreckConfig::default();
        let state = game.initial_state(seed, &cfg);
        let mut loop_ = GameLoop::new(&game, state);
        let mut chance_rng = ProductionRng::from_seed(seed);
        let mut sink = StubGameEventSink::new();
        let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = vec![
            Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(101),
            ))),
            Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(103),
            ))),
        ];
        let result = loop_
            .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
            .await
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(result.scores.len(), 2);
    }
}
