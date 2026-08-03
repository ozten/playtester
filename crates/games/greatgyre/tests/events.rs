//! Unit 4 integration tests: play_event targeting legality, reactions
//! (Dead Fish / Fisher), each event's effect, Telescope activation,
//! Walrus removal, and Work Day.

use playtest_adapters::ProductionRng;
use playtest_core::{Actor, Game, GameError};
use playtest_greatgyre::state::{PendingDecisionKind, PlacedCard};
use playtest_greatgyre::{
    Action, Card, CardInstanceId, CardKind, DecisionChoice, EventKind, EventTarget,
    GreatGyreConfig, GreatGyreGame, ModificationKind, Phase, Resource, SurvivorId,
};

fn setup_np(seed: u64, n: u8) -> (GreatGyreGame, playtest_greatgyre::GameState) {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(n).unwrap();
    let mut state = game.initial_state(seed, &cfg);
    while state.phase == Phase::SurvivorDraft {
        let Actor::Player(p) = game.next_actor(&state) else {
            unreachable!()
        };
        let legal = game.legal_actions(&state, p);
        let events = game.apply_action(&state, p, &legal[0]).unwrap();
        for e in &events {
            game.apply_event(&mut state, e);
        }
    }
    let mut rng = ProductionRng::from_seed(seed ^ 0x1234);
    let ev = game.resolve_chance(&state, &mut rng).unwrap();
    game.apply_event(&mut state, &ev);
    (game, state)
}

fn setup_2p(seed: u64) -> (GreatGyreGame, playtest_greatgyre::GameState) {
    setup_np(seed, 2)
}

fn apply(game: GreatGyreGame, state: &mut playtest_greatgyre::GameState, player: u8, action: Action) {
    let events = game.apply_action(state, player, &action).unwrap();
    for e in &events {
        game.apply_event(state, e);
    }
}

fn survivor_card(id: u32, s: SurvivorId) -> Card {
    Card::new(CardInstanceId(id), CardKind::Survivor(s))
}
fn placed_survivor(id: u32, s: SurvivorId) -> PlacedCard {
    PlacedCard {
        card: survivor_card(id, s),
        hungry: false,
    }
}
fn mod_card(id: u32, m: ModificationKind) -> Card {
    Card::new(CardInstanceId(id), CardKind::Modification(m))
}
fn placed_mod(id: u32, m: ModificationKind) -> PlacedCard {
    PlacedCard {
        card: mod_card(id, m),
        hungry: false,
    }
}
fn event_card(id: u32, e: EventKind) -> Card {
    Card::new(CardInstanceId(id), CardKind::Event(e))
}
fn resource_card(id: u32, r: Resource) -> Card {
    Card::new(CardInstanceId(id), CardKind::Resource(r))
}
fn dead_fish(id: u32) -> Card {
    Card::new(CardInstanceId(id), CardKind::DeadFish)
}

/// Bring `state` to the start of `player`'s Phase 3 (Actions) with a
/// clean hand/actions budget, so tests can inject exactly the cards
/// they need. Leaves 2 actions available — one to spend on the event
/// under test, one left over so a test can confirm control actually
/// returns to the attacker's `Phase::Actions` (rather than immediately
/// racing on into end-of-turn because the budget hit 0).
fn enter_actions_phase(game: GreatGyreGame, state: &mut playtest_greatgyre::GameState, player: u8) {
    if state.phase == Phase::Draw && state.current_player == player {
        apply(game, state, player, Action::FinishDrawing);
    }
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, player);
    state.players[player as usize].actions_remaining = 2;
}

// ---------- Shark Attack ----------------------------------------------------

#[test]
fn shark_attack_targeting_requires_two_survivors() {
    let (game, mut state) = setup_2p(1);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(90_000, EventKind::SharkAttack)];
    state.players[1].placed = vec![placed_survivor(90_001, SurvivorId::Fisher)]; // only 1

    let legal = game.legal_actions(&state, 0);
    assert!(
        !legal.iter().any(|a| matches!(a, Action::PlayEvent { .. })),
        "Shark Attack should not be offered against a target with <2 survivors: {legal:#?}"
    );

    state.players[1]
        .placed
        .push(placed_survivor(90_002, SurvivorId::Porter));
    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::PlayEvent {
        card: CardInstanceId(90_000),
        target: EventTarget::Player { target: 1 },
    }));
}

#[test]
fn shark_attack_declined_lets_target_choose_which_survivor_to_lose() {
    let (game, mut state) = setup_2p(2);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(91_000, EventKind::SharkAttack)];
    state.players[1].placed = vec![
        placed_survivor(91_001, SurvivorId::Fisher),
        placed_survivor(91_002, SurvivorId::Porter),
    ];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(91_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    assert_eq!(state.phase, Phase::ResolvingDecision);
    assert_eq!(state.current_pending().unwrap().player, 1);
    assert!(matches!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::EventReaction { .. }
    ));
    // The event card discards immediately regardless of outcome.
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 91_000));

    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::DeclineReaction,
        },
    );
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::SharkChooseSurvivor { attacker: 0 }
    );
    let legal = game.legal_actions(&state, 1);
    assert_eq!(legal.len(), 2);

    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::LoseSurvivorToShark {
                survivor: CardInstanceId(91_001),
            },
        },
    );
    assert_eq!(state.players[1].placed.len(), 1);
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 91_001));
    // Control returns to the attacker's Phase::Actions.
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

#[test]
fn dead_fish_negates_shark_attack() {
    let (game, mut state) = setup_2p(3);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(92_000, EventKind::SharkAttack)];
    state.players[1].placed = vec![
        placed_survivor(92_001, SurvivorId::Fisher),
        placed_survivor(92_002, SurvivorId::Porter),
    ];
    state.players[1].hand = vec![dead_fish(92_003)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(92_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::ReactWithDeadFish {
                card: CardInstanceId(92_003),
            },
        },
    );
    // Negated: both survivors survive, Dead Fish is spent.
    assert_eq!(state.players[1].placed.len(), 2);
    assert!(state.players[1].hand.is_empty());
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 92_003));
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

#[test]
fn fisher_negates_shark_by_discarding_any_hand_card() {
    let (game, mut state) = setup_2p(4);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(93_000, EventKind::SharkAttack)];
    state.players[1].placed = vec![
        placed_survivor(93_001, SurvivorId::Fisher),
        placed_survivor(93_002, SurvivorId::Porter),
    ];
    state.players[1].hand = vec![resource_card(93_003, Resource::Rope)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(93_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    let legal = game.legal_actions(&state, 1);
    assert!(legal.contains(&Action::ResolveDecision {
        choice: DecisionChoice::ReactWithFisher {
            card: CardInstanceId(93_003)
        }
    }));
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::ReactWithFisher {
                card: CardInstanceId(93_003),
            },
        },
    );
    assert_eq!(state.players[1].placed.len(), 2);
    assert!(state.players[1].hand.is_empty());
}

#[test]
fn fisher_does_not_negate_walrus() {
    let (game, mut state) = setup_2p(5);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(94_000, EventKind::Walrus)];
    state.players[1].placed = vec![placed_survivor(94_001, SurvivorId::Fisher)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(94_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    let legal = game.legal_actions(&state, 1);
    assert!(
        !legal.iter().any(|a| matches!(
            a,
            Action::ResolveDecision {
                choice: DecisionChoice::ReactWithFisher { .. }
            }
        )),
        "Fisher must not be offered against Walrus: {legal:#?}"
    );
}

// ---------- Octopus Attack ---------------------------------------------------

#[test]
fn octopus_attack_requires_target_extension_and_returns_it_to_the_pile() {
    let (game, mut state) = setup_2p(6);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(95_000, EventKind::OctopusAttack)];
    let ext = Card::new(CardInstanceId(95_001), CardKind::RaftExtension);
    state.players[1].built_extensions = vec![ext];
    let pile_before = state.extension_pile.len();

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(95_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::DeclineReaction,
        },
    );
    assert!(state.players[1].built_extensions.is_empty());
    assert_eq!(state.extension_pile.len(), pile_before + 1);
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

#[test]
fn octopus_attack_capacity_shortfall_opens_relocate_decision() {
    let (game, mut state) = setup_2p(7);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(96_000, EventKind::OctopusAttack)];
    // Target: base raft (2 spaces) + 1 extension (2 spaces) = 4 total,
    // fully occupied by 4 survivors. Losing the extension drops
    // capacity to 2, leaving a deficit of 2.
    state.players[1].built_extensions = vec![Card::new(CardInstanceId(96_001), CardKind::RaftExtension)];
    state.players[1].placed = vec![
        placed_survivor(96_002, SurvivorId::Fisher),
        placed_survivor(96_003, SurvivorId::Porter),
        placed_survivor(96_004, SurvivorId::Athlete),
        placed_survivor(96_005, SurvivorId::FirstMate),
    ];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(96_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::DeclineReaction,
        },
    );
    assert_eq!(state.phase, Phase::ResolvingDecision);
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::OctopusRelocate { needed: 2, attacker: 0 }
    );

    let face_up_before = state.players[1]
        .current
        .iter()
        .filter(|c| c.face == playtest_greatgyre::Face::Up)
        .count();
    for _ in 0..2 {
        let legal = game.legal_actions(&state, 1);
        apply(game, &mut state, 1, legal[0]);
    }
    assert_eq!(state.players[1].placed.len(), 2);
    let face_up_after = state.players[1]
        .current
        .iter()
        .filter(|c| c.face == playtest_greatgyre::Face::Up)
        .count();
    assert_eq!(face_up_after, face_up_before + 2);
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

// ---------- Walrus -----------------------------------------------------------

#[test]
fn walrus_targeting_requires_a_free_space_and_blocks_one_when_placed() {
    let (game, mut state) = setup_2p(8);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(97_000, EventKind::Walrus)];
    // Fill the target's raft completely: no legal Walrus target.
    state.players[1].placed = vec![
        placed_survivor(97_001, SurvivorId::Fisher),
        placed_survivor(97_002, SurvivorId::Porter),
    ];
    let legal = game.legal_actions(&state, 0);
    assert!(!legal.iter().any(|a| matches!(a, Action::PlayEvent { .. })));

    // Free a space.
    state.players[1].placed.pop();
    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::PlayEvent {
        card: CardInstanceId(97_000),
        target: EventTarget::Player { target: 1 },
    }));

    let free_before = playtest_greatgyre::turns::free_spaces(&state.players[1]);
    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(97_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::DeclineReaction,
        },
    );
    assert_eq!(state.players[1].blocked_by_walrus.len(), 1);
    // Walrus never touches the discard pile when it lands.
    assert!(!state.discard_pile.iter().any(|c| c.id.0 == 97_000));
    let free_after = playtest_greatgyre::turns::free_spaces(&state.players[1]);
    assert_eq!(free_after, free_before - 1);
}

#[test]
fn dead_fish_negates_walrus_and_it_never_lands() {
    let (game, mut state) = setup_2p(9);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(98_000, EventKind::Walrus)];
    state.players[1].hand = vec![dead_fish(98_001)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(98_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::ReactWithDeadFish {
                card: CardInstanceId(98_001),
            },
        },
    );
    assert!(state.players[1].blocked_by_walrus.is_empty());
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 98_000));
}

#[test]
fn remove_walrus_action_discards_dead_fish_and_walrus() {
    let (game, mut state) = setup_2p(10);
    enter_actions_phase(game, &mut state, 0);
    let walrus = Card::new(CardInstanceId(99_000), CardKind::Event(EventKind::Walrus));
    state.players[0].blocked_by_walrus = vec![walrus];
    state.players[0].hand = vec![dead_fish(99_001)];

    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::RemoveWalrus {
        dead_fish: CardInstanceId(99_001),
        walrus: CardInstanceId(99_000),
    }));
    apply(
        game,
        &mut state,
        0,
        Action::RemoveWalrus {
            dead_fish: CardInstanceId(99_001),
            walrus: CardInstanceId(99_000),
        },
    );
    assert!(state.players[0].blocked_by_walrus.is_empty());
    assert!(state.players[0].hand.is_empty());
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 99_000));
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 99_001));
}

// ---------- Love Boat ---------------------------------------------------------

#[test]
fn love_boat_requires_casters_free_space_and_targets_survivor_choice() {
    let (game, mut state) = setup_2p(11);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(100_000, EventKind::LoveBoat)];
    // Fill caster's own raft: no free space, so no legal Love Boat play.
    state.players[0].placed = vec![
        placed_survivor(100_001, SurvivorId::Fisher),
        placed_survivor(100_002, SurvivorId::Porter),
    ];
    state.players[1].placed = vec![
        placed_survivor(100_003, SurvivorId::Athlete),
        placed_survivor(100_004, SurvivorId::FirstMate),
    ];
    let legal = game.legal_actions(&state, 0);
    assert!(!legal.iter().any(|a| matches!(a, Action::PlayEvent { .. })));

    state.players[0].placed.pop();
    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::PlayEvent {
        card: CardInstanceId(100_000),
        target: EventTarget::Player { target: 1 },
    }));

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(100_000),
            target: EventTarget::Player { target: 1 },
        },
    );
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::LoveBoatChooseSurvivor { attacker: 0 }
    );
    assert_eq!(state.current_pending().unwrap().player, 1);

    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::GiveSurvivorToLoveBoat {
                survivor: CardInstanceId(100_003),
            },
        },
    );
    assert_eq!(state.players[1].placed.len(), 1);
    assert!(
        state.players[0]
            .placed
            .iter()
            .any(|pc| pc.card.id.0 == 100_003)
    );
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

// ---------- Storm --------------------------------------------------------------

#[test]
fn storm_visits_every_other_player_in_down_current_order() {
    let (game, mut state) = setup_np(12, 3);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(101_000, EventKind::Storm)];
    state.players[1].placed = vec![placed_survivor(101_001, SurvivorId::Fisher)];
    state.players[2].placed = vec![placed_survivor(101_002, SurvivorId::Porter)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(101_000),
            target: EventTarget::None,
        },
    );
    // Down-current from seat 0 is seat 1, then seat 2.
    assert_eq!(state.current_pending().unwrap().player, 1);
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::StormChoice {
            attacker: 0,
            queue_tail: vec![2],
        }
    );

    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::StormRemoveCard {
                card: CardInstanceId(101_001),
            },
        },
    );
    assert!(state.players[1].placed.is_empty());
    assert!(
        state.players[1]
            .current
            .iter()
            .any(|c| c.card.id.0 == 101_001 && c.face == playtest_greatgyre::Face::Up)
    );
    assert_eq!(state.current_pending().unwrap().player, 2);

    apply(
        game,
        &mut state,
        2,
        Action::ResolveDecision {
            choice: DecisionChoice::StormRemoveCard {
                card: CardInstanceId(101_002),
            },
        },
    );
    // Storm resolved for everyone; control returns to the caster.
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.current_player, 0);
}

#[test]
fn storm_discard_route_excludes_event_cards_and_caps_at_two() {
    let (game, mut state) = setup_2p(13);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(102_000, EventKind::Storm)];
    state.players[1].hand = vec![
        resource_card(102_001, Resource::Rope),
        resource_card(102_002, Resource::Wood),
        resource_card(102_003, Resource::Plastic),
        event_card(102_004, EventKind::WorkDay), // not eligible for Storm discard
    ];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(102_000),
            target: EventTarget::None,
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::StormTakeDiscardRoute,
        },
    );
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::StormDiscard {
            needed: 2,
            attacker: 0,
            queue_tail: vec![],
        }
    );
    let legal = game.legal_actions(&state, 1);
    assert_eq!(legal.len(), 3, "the event card must not be offered: {legal:#?}");

    for _ in 0..2 {
        let legal = game.legal_actions(&state, 1);
        apply(game, &mut state, 1, legal[0]);
    }
    assert_eq!(state.players[1].hand.len(), 2); // 1 resource + the event card left
    assert!(
        state.players[1]
            .hand
            .iter()
            .any(|c| matches!(c.kind, CardKind::Event(_))),
        "the event card must survive Storm's discard"
    );
    assert_eq!(state.phase, Phase::Actions);
}

#[test]
fn storm_discard_route_takes_what_they_can_when_fewer_than_two_eligible() {
    let (game, mut state) = setup_2p(14);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(103_000, EventKind::Storm)];
    state.players[1].hand = vec![resource_card(103_001, Resource::Rope)];

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(103_000),
            target: EventTarget::None,
        },
    );
    apply(
        game,
        &mut state,
        1,
        Action::ResolveDecision {
            choice: DecisionChoice::StormTakeDiscardRoute,
        },
    );
    // Only 1 eligible card and no real choice of *which* card to
    // discard: forced-resolved immediately (matching the Phase-4
    // hungry/stand-up forced-case pattern) without a follow-up
    // StormDiscard decision — the lone card is discarded outright.
    assert_eq!(state.phase, Phase::Actions);
    assert!(state.players[1].hand.is_empty(), "the lone eligible card should have been discarded");
    assert!(
        state.players[1]
            .current
            .iter()
            .any(|c| c.card.id.0 == 103_001 && c.face == playtest_greatgyre::Face::Down),
        "the discarded card should be face-down in the player's own Current"
    );
}

// ---------- Work Day -----------------------------------------------------------

#[test]
fn work_day_grants_unlimited_actions_and_free_current_draws() {
    let (game, mut state) = setup_2p(15);
    enter_actions_phase(game, &mut state, 0);
    // Clear the drafted survivor so the base raft's 2 spaces are free
    // for the 2 survivors this test plays.
    state.players[0].placed.clear();
    state.players[0].hand = vec![
        event_card(104_000, EventKind::WorkDay),
        survivor_card(104_001, SurvivorId::Fisher),
        survivor_card(104_002, SurvivorId::Porter),
    ];
    state.players[0].current.push(playtest_greatgyre::CurrentCard {
        card: resource_card(104_003, Resource::Rope),
        face: playtest_greatgyre::Face::Up,
    });

    apply(
        game,
        &mut state,
        0,
        Action::PlayEvent {
            card: CardInstanceId(104_000),
            target: EventTarget::None,
        },
    );
    assert!(state.players[0].work_day_active);
    assert_eq!(state.phase, Phase::Actions, "Work Day itself doesn't leave Phase 3");

    // Actions_remaining is 0 (Work Day was itself the only action) but
    // more actions are still legal.
    let legal = game.legal_actions(&state, 0);
    assert!(legal.iter().any(|a| matches!(a, Action::PlaySurvivor { .. })));
    assert!(legal.iter().any(|a| matches!(a, Action::WorkDayDraw { .. })));

    apply(
        game,
        &mut state,
        0,
        Action::PlaySurvivor {
            card: CardInstanceId(104_001),
        },
    );
    assert_eq!(state.phase, Phase::Actions, "unlimited actions: still in Phase 3");
    apply(
        game,
        &mut state,
        0,
        Action::PlaySurvivor {
            card: CardInstanceId(104_002),
        },
    );
    assert_eq!(state.players[0].placed.len(), 2);

    let current_len_before = state.players[0].current.len();
    let draws_remaining_before = state.players[0].draws_remaining;
    apply(
        game,
        &mut state,
        0,
        Action::WorkDayDraw {
            card: CardInstanceId(104_003),
        },
    );
    assert_eq!(state.players[0].current.len(), current_len_before - 1);
    assert!(state.players[0].hand.iter().any(|c| c.id.0 == 104_003));
    // The free draw must not touch the Phase-2 draw budget.
    assert_eq!(state.players[0].draws_remaining, draws_remaining_before);
}

// ---------- Telescope + Land Sighting -------------------------------------------

#[test]
fn telescope_activation_discards_it_and_draws_an_event_card() {
    let (game, mut state) = setup_2p(16);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].placed = vec![placed_mod(105_000, ModificationKind::Telescope)];
    let top_event = state.event_deck.last().copied();

    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::ActivateTelescope {
        card: CardInstanceId(105_000),
    }));

    apply(
        game,
        &mut state,
        0,
        Action::ActivateTelescope {
            card: CardInstanceId(105_000),
        },
    );
    assert!(state.players[0].placed.is_empty());
    assert!(state.discard_pile.iter().any(|c| c.id.0 == 105_000));
    if let Some(top) = top_event {
        assert!(state.players[0].hand.iter().any(|c| c.id == top.id));
    }
}

#[test]
fn land_sighting_is_never_a_legal_play_event_target() {
    let (game, mut state) = setup_2p(17);
    enter_actions_phase(game, &mut state, 0);
    state.players[0].hand = vec![event_card(106_000, EventKind::LandSighting)];
    let legal = game.legal_actions(&state, 0);
    assert!(!legal.iter().any(|a| matches!(a, Action::PlayEvent { .. })));

    let err = game
        .apply_action(
            &state,
            0,
            &Action::PlayEvent {
                card: CardInstanceId(106_000),
                target: EventTarget::None,
            },
        )
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}
