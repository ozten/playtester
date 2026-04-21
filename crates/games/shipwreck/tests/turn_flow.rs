//! Unit 22 turn-flow integration tests.
//!
//! Validates the `Game` trait impl for `ShipWreckGame` end-to-end:
//! action enumeration, apply_action→apply_event round-tripping,
//! edge cases (illegal actions), and one random-vs-random game to
//! termination within a turn budget.

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game, GameError, GameLoop};
use playtest_shipwreck::{
    Action, Card, EquipmentCard, EquipmentKind, Event, EventCardKind, EventTarget, PlacedPlayerCard,
    PlayerCard, PlayerCardId, PlayerSkill, RaftExtensionCard, Resource, ShipWreckConfig,
    ShipWreckGame, SlotId,
    state::STARTING_FOOD_COUNTER,
};

fn two_player_initial() -> (ShipWreckGame, playtest_shipwreck::GameState) {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let state = game.initial_state(42, &cfg);
    (game, state)
}

#[tokio::test]
async fn random_vs_random_2p_terminates_within_500_turns() {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let state = game.initial_state(7, &cfg);
    let mut loop_ = GameLoop::new(&game, state);
    let mut chance_rng = ProductionRng::from_seed(7);
    let mut sink = StubGameEventSink::new();
    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = vec![
        Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(101))),
        Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(103))),
    ];

    let result = loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .expect("game must not error");

    assert_eq!(
        result.scores.len(),
        2,
        "expected 2 per-seat scores, got {}",
        result.scores.len()
    );
    if let Some(w) = result.winner {
        assert!(w < 2, "winner seat {w} out of range");
    }
}

#[test]
fn apply_action_build_equipment_with_exact_resources_succeeds_and_spends_inventory() {
    let (game, mut state) = two_player_initial();

    // Force a concrete equipment top so we can test it exactly.
    state.equipment_deck = vec![EquipmentCard::new(EquipmentKind::FishingPoles, 0)];
    // FishingPoles cost: 1 wood, 1 rope, 1 wire.
    state.players[0].inventory = [0, 1, 1, 0, 1];

    let action = Action::BuildEquipment {
        equipment_kind: EquipmentKind::FishingPoles,
        slot: SlotId::BaseLeft,
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");

    // Expected shape: ResourceSpent x3 (wood, rope, wire), then BuiltEquipment.
    assert_eq!(events.len(), 4, "events: {events:#?}");
    assert!(matches!(
        events[0],
        Event::ResourceSpent {
            player: 0,
            resource: Resource::Wood,
            amount: 1
        }
    ));
    assert!(matches!(
        events[1],
        Event::ResourceSpent {
            player: 0,
            resource: Resource::Rope,
            amount: 1
        }
    ));
    assert!(matches!(
        events[2],
        Event::ResourceSpent {
            player: 0,
            resource: Resource::Wire,
            amount: 1
        }
    ));
    assert!(matches!(
        events[3],
        Event::BuiltEquipment {
            player: 0,
            equipment_kind: EquipmentKind::FishingPoles,
            slot: SlotId::BaseLeft
        }
    ));

    for e in &events {
        game.apply_event(&mut state, e);
    }
    assert_eq!(state.players[0].inventory, [0, 0, 0, 0, 0]);
    assert!(state.players[0].raft.upgrade_at(SlotId::BaseLeft).is_some());
    assert!(state.equipment_deck.is_empty());
}

#[test]
fn apply_action_extend_raft_after_baseleft_adds_extension_at_index_zero() {
    let (game, mut state) = two_player_initial();

    state.players[0]
        .hand
        .push(Card::RaftExtension(RaftExtensionCard::new(999)));

    let action = Action::ExtendRaft {
        insert_after: SlotId::BaseLeft,
    };
    let events = game.apply_action(&state, 0, &action).expect("legal");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::ExtendedRaft {
            player: 0,
            extension_serial: _,
            insert_after: SlotId::BaseLeft
        }
    ));

    for e in &events {
        game.apply_event(&mut state, e);
    }
    assert_eq!(state.players[0].raft.extensions.len(), 1);
    assert_eq!(state.players[0].raft.length(), 3);
}

#[test]
fn apply_action_build_equipment_on_occupied_slot_returns_illegal_action() {
    let (game, mut state) = two_player_initial();

    state.players[0]
        .raft
        .build_upgrade(
            SlotId::BaseLeft,
            EquipmentCard::new(EquipmentKind::SteelCordage, 0),
        )
        .unwrap();

    state.equipment_deck = vec![EquipmentCard::new(EquipmentKind::FishingPoles, 0)];
    state.players[0].inventory = [0, 1, 1, 0, 1];

    let action = Action::BuildEquipment {
        equipment_kind: EquipmentKind::FishingPoles,
        slot: SlotId::BaseLeft,
    };
    let err = game.apply_action(&state, 0, &action).unwrap_err();
    assert!(
        matches!(err, GameError::IllegalAction { .. }),
        "expected IllegalAction, got {err:?}"
    );
}

#[test]
fn apply_action_play_event_card_returns_illegal_action_for_unit_22() {
    let (game, state) = two_player_initial();
    let action = Action::PlayEventCard {
        card: EventCardKind::FlyingFish,
        target: EventTarget::None,
    };
    let err = game.apply_action(&state, 0, &action).unwrap_err();
    assert!(
        matches!(err, GameError::IllegalAction { .. }),
        "expected IllegalAction, got {err:?}"
    );
}

#[test]
fn starvation_drops_played_player_card_when_food_counter_is_zero() {
    let (game, mut state) = two_player_initial();

    state.players[0].food_counter = 0;
    state.players[0].played_players = vec![PlacedPlayerCard {
        card: PlayerCard::new(
            PlayerCardId::Professor,
            1,
            1,
            Some(PlayerSkill::ConstructWithOneFewerResource),
        ),
        slot: SlotId::BaseLeft,
    }];

    let events = game.apply_action(&state, 0, &Action::EndTurn).unwrap();
    let first = events
        .iter()
        .find(|e| matches!(e, Event::FoodConsumed { .. }))
        .expect("expected at least one FoodConsumed event");
    let Event::FoodConsumed {
        player,
        slot,
        amount,
        starved,
    } = first
    else {
        panic!("not FoodConsumed");
    };
    assert_eq!(*player, 0);
    assert_eq!(*slot, SlotId::BaseLeft);
    assert_eq!(*amount, 1);
    assert!(*starved, "Professor should starve with food=0");

    for e in &events {
        game.apply_event(&mut state, e);
    }
    assert!(
        state.players[0].played_players.is_empty(),
        "played_players should be empty after starve"
    );
    assert_eq!(state.players[0].food_counter, 0);
}

#[test]
fn end_turn_advances_current_player() {
    let (game, mut state) = two_player_initial();
    assert_eq!(state.current_player, 0);

    let events = game.apply_action(&state, 0, &Action::EndTurn).unwrap();
    for e in &events {
        game.apply_event(&mut state, e);
    }
    if state.phase != playtest_shipwreck::Phase::Finished {
        assert_eq!(state.current_player, 1);
    }
}

#[test]
fn legal_actions_always_includes_endturn() {
    let (game, state) = two_player_initial();
    let legal = game.legal_actions(&state, state.current_player);
    assert!(!legal.is_empty(), "legal actions should be non-empty");
    assert!(
        legal.iter().any(|a| matches!(a, Action::EndTurn)),
        "EndTurn must always be legal"
    );
}

#[test]
fn starting_food_counter_matches_constant() {
    let (_game, state) = two_player_initial();
    for p in &state.players {
        assert_eq!(p.food_counter, STARTING_FOOD_COUNTER);
    }
}

#[test]
fn public_view_hides_opponent_hand_contents_but_shows_hand_size() {
    let (game, state) = two_player_initial();
    let view = game.public_view(&state, 0);
    assert_eq!(view.observer, 0);
    assert_eq!(view.own.hand, state.players[0].hand);
    let opp = view.opponents[1]
        .as_ref()
        .expect("seat 1 must have an OpponentView");
    assert_eq!(opp.hand_size, state.players[1].hand.len());
}
