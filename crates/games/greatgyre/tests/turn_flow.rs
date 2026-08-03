//! Unit 2 integration tests: the five-phase turn machine.
//!
//! Exercises each phase via a scripted 2-player game, plus targeted
//! edge cases (food/hungry, hand-limit discard, final-round trigger)
//! constructed by mutating state directly and driving `apply_action` /
//! `apply_event` — the same style as ShipWreck's `turn_flow.rs`.

use playtest_adapters::ProductionRng;
use playtest_core::{Actor, Game, GameError};
use playtest_greatgyre::state::{PendingDecisionKind, PlacedCard};
use playtest_greatgyre::{
    Action, CardKind, DecisionChoice, Face, GreatGyreConfig, GreatGyreGame, ModificationKind,
    Phase, Resource, SurvivorId,
};

/// Draft every seat (always the first legal survivor) and run the
/// post-draft chance step, landing on `Phase::Draw` for seat 0.
fn setup_2p(seed: u64) -> (GreatGyreGame, playtest_greatgyre::GameState) {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(2).unwrap();
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

fn apply(game: GreatGyreGame, state: &mut playtest_greatgyre::GameState, player: u8, action: Action) {
    let events = game.apply_action(state, player, &action).unwrap();
    for e in &events {
        game.apply_event(state, e);
    }
}

// ---------- Phase 2: Draw --------------------------------------------------

#[test]
fn finish_drawing_immediately_transitions_to_actions() {
    let (game, mut state) = setup_2p(1);
    assert_eq!(state.phase, Phase::Draw);
    apply(game, &mut state, 0, Action::FinishDrawing);
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.players[0].actions_remaining, 1);
}

#[test]
fn drawing_from_current_moves_card_to_hand_and_ends_draw_phase() {
    let (game, mut state) = setup_2p(1);
    let hand_before = state.players[0].hand.len();
    let current_before = state.players[0].current.len();
    let card_id = state.players[0].current[0].card.id;
    apply(game, &mut state, 0, Action::DrawFromCurrent { card: card_id });
    // Budget is 1 in Unit 2, so drawing once exhausts it and
    // auto-transitions straight to Phase::Actions — no FinishDrawing
    // needed.
    assert_eq!(state.phase, Phase::Actions);
    assert_eq!(state.players[0].hand.len(), hand_before + 1);
    assert_eq!(state.players[0].current.len(), current_before - 1);
    assert!(state.players[0].hand.iter().any(|c| c.id == card_id));
}

#[test]
fn drawing_unknown_card_is_illegal() {
    let (game, state) = setup_2p(1);
    let bogus = playtest_greatgyre::CardInstanceId(u32::MAX);
    let err = game
        .apply_action(&state, 0, &Action::DrawFromCurrent { card: bogus })
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

// ---------- Phase 3: Actions -----------------------------------------------

#[test]
fn playing_a_survivor_consumes_the_action_and_occupies_a_space() {
    let (game, mut state) = setup_2p(2);
    apply(game, &mut state, 0, Action::FinishDrawing);
    assert_eq!(state.phase, Phase::Actions);

    // Find a survivor in hand, if any; otherwise inject one so the
    // action-phase mechanics (space occupancy, budget exhaustion) are
    // exercised deterministically regardless of the shuffle.
    let survivor_in_hand = state.players[0]
        .hand
        .iter()
        .find(|c| matches!(c.kind, CardKind::Survivor(_)))
        .copied();
    let card = survivor_in_hand.unwrap_or_else(|| {
        let c = playtest_greatgyre::Card::new(
            playtest_greatgyre::CardInstanceId(90_000),
            CardKind::Survivor(SurvivorId::Fisher),
        );
        state.players[0].hand.push(c);
        c
    });

    let placed_before = state.players[0].placed.len();
    apply(game, &mut state, 0, Action::PlaySurvivor { card: card.id });
    assert_eq!(state.players[0].placed.len(), placed_before + 1);
    // Budget is 1, so the action phase auto-finished.
    assert_ne!(state.phase, Phase::Actions);
}

#[test]
fn building_modification_pays_cost_and_places_on_raft() {
    let (game, mut state) = setup_2p(3);
    apply(game, &mut state, 0, Action::FinishDrawing);

    // Force a deterministic scenario: Sail costs 1 Wood + 1 Rope.
    let sail = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_001),
        CardKind::Modification(ModificationKind::Sail),
    );
    let wood = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_002),
        CardKind::Resource(Resource::Wood),
    );
    let rope = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_003),
        CardKind::Resource(Resource::Rope),
    );
    state.players[0].hand.clear();
    state.players[0].hand.push(sail);
    state.players[0].hand.push(wood);
    state.players[0].hand.push(rope);
    state.players[0].actions_remaining = 1;

    let discard_before = state.discard_pile.len();
    apply(game, &mut state, 0, Action::BuildModification { card: sail.id });
    assert!(state.players[0].hand.is_empty());
    assert_eq!(state.discard_pile.len(), discard_before + 2);
    assert!(
        state.players[0]
            .placed
            .iter()
            .any(|p| p.card.id == sail.id)
    );
}

#[test]
fn building_modification_without_resources_is_illegal() {
    let (game, mut state) = setup_2p(3);
    apply(game, &mut state, 0, Action::FinishDrawing);
    let sail = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_010),
        CardKind::Modification(ModificationKind::Sail),
    );
    state.players[0].hand.push(sail);
    let err = game
        .apply_action(&state, 0, &Action::BuildModification { card: sail.id })
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn building_extension_takes_from_shared_pile() {
    let (game, mut state) = setup_2p(4);
    apply(game, &mut state, 0, Action::FinishDrawing);
    let wood = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_020),
        CardKind::Resource(Resource::Wood),
    );
    let rope = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_021),
        CardKind::Resource(Resource::Rope),
    );
    let plastic = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_022),
        CardKind::Resource(Resource::Plastic),
    );
    state.players[0].hand.clear();
    state.players[0].hand.extend([wood, rope, plastic]);
    state.players[0].actions_remaining = 1;
    let pile_before = state.extension_pile.len();

    apply(game, &mut state, 0, Action::BuildExtension);
    assert_eq!(state.extension_pile.len(), pile_before - 1);
    assert_eq!(state.players[0].built_extensions.len(), 1);
    assert!(state.players[0].hand.is_empty());
}

#[test]
fn stowaway_needs_no_space() {
    let (game, mut state) = setup_2p(6);
    apply(game, &mut state, 0, Action::FinishDrawing);
    // Fill both base-raft spaces with non-Stowaway survivors first.
    state.players[0].placed.push(PlacedCard {
        card: playtest_greatgyre::Card::new(
            playtest_greatgyre::CardInstanceId(90_030),
            CardKind::Survivor(SurvivorId::Fisher),
        ),
        hungry: false,
    });
    state.players[0].placed.push(PlacedCard {
        card: playtest_greatgyre::Card::new(
            playtest_greatgyre::CardInstanceId(90_031),
            CardKind::Survivor(SurvivorId::Porter),
        ),
        hungry: false,
    });
    let stowaway = playtest_greatgyre::Card::new(
        playtest_greatgyre::CardInstanceId(90_032),
        CardKind::Survivor(SurvivorId::Stowaway),
    );
    state.players[0].hand.push(stowaway);
    state.players[0].actions_remaining = 1;

    let legal = game.legal_actions(&state, 0);
    assert!(
        legal.contains(&Action::PlaySurvivor { card: stowaway.id }),
        "Stowaway must be playable even with a full raft"
    );
}

// ---------- hand-limit discard flow ----------------------------------------

#[test]
fn over_hand_limit_opens_discard_decision_and_resolves_to_own_current() {
    let (game, mut state) = setup_2p(9);
    // Hand limit is 3 with nothing placed (the draft placed one
    // survivor directly on the raft, which would otherwise add +1 —
    // clear it so the limit is the clean base value). Force 5 hand
    // cards.
    state.players[0].placed.clear();
    state.players[0].hand.clear();
    for i in 0..5u32 {
        state.players[0].hand.push(playtest_greatgyre::Card::new(
            playtest_greatgyre::CardInstanceId(80_000 + i),
            CardKind::Resource(Resource::Rope),
        ));
    }
    state.players[0].draws_remaining = 0;
    state.phase = Phase::Actions;
    state.players[0].actions_remaining = 1;

    apply(game, &mut state, 0, Action::FinishActions);
    assert_eq!(state.phase, Phase::ResolvingDecision);
    let top = state.current_pending().unwrap();
    assert_eq!(top.kind, PendingDecisionKind::DiscardDown { needed: 2 });

    let current_before = state.players[0].current.len();
    for _ in 0..2 {
        let legal = game.legal_actions(&state, 0);
        assert!(!legal.is_empty());
        let Action::ResolveDecision {
            choice: DecisionChoice::Discard { card },
        } = legal[0]
        else {
            panic!("expected a Discard choice, got {:?}", legal[0]);
        };
        apply(
            game,
            &mut state,
            0,
            Action::ResolveDecision {
                choice: DecisionChoice::Discard { card },
            },
        );
    }
    assert_eq!(state.players[0].hand.len(), 3);
    assert_eq!(state.players[0].current.len(), current_before + 2);
    // Discards go face-down.
    for c in state.players[0].current.iter().rev().take(2) {
        assert_eq!(c.face, Face::Down);
    }
    // Discard resolved, food phase auto-resolves with nothing placed,
    // so the turn continues past ResolvingDecision.
    assert_ne!(state.phase, Phase::ResolvingDecision);
}

// ---------- food / hungry edge cases ---------------------------------------

fn survivor_card(id: u32, s: SurvivorId) -> playtest_greatgyre::Card {
    playtest_greatgyre::Card::new(playtest_greatgyre::CardInstanceId(id), CardKind::Survivor(s))
}

#[test]
fn negative_food_with_enough_standing_survivors_opens_make_hungry_decision() {
    let (game, mut state) = setup_2p(11);
    // Raft Right +1, three Fishers at -1 each => food = 1 - 3 = -2.
    state.players[0].placed = vec![
        PlacedCard {
            card: survivor_card(70_000, SurvivorId::Fisher),
            hungry: false,
        },
        PlacedCard {
            card: survivor_card(70_001, SurvivorId::Fisher),
            hungry: false,
        },
        PlacedCard {
            card: survivor_card(70_002, SurvivorId::Fisher),
            hungry: false,
        },
    ];
    state.phase = Phase::Actions;
    state.players[0].actions_remaining = 1;
    state.players[0].hand.clear();

    apply(game, &mut state, 0, Action::FinishActions);
    assert_eq!(state.phase, Phase::ResolvingDecision);
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::MakeHungry { needed: 2 }
    );

    for _ in 0..2 {
        let legal = game.legal_actions(&state, 0);
        apply(game, &mut state, 0, legal[0]);
    }
    let hungry_count = state.players[0].placed.iter().filter(|p| p.hungry).count();
    assert_eq!(hungry_count, 2);
}

#[test]
fn negative_food_with_too_few_standing_survivors_forces_abandonment() {
    let (game, mut state) = setup_2p(12);
    // Raft Right +1, one Fisher at -1 => food = 0, not negative yet.
    // Add a second Fisher already Hungry (still counts, per spec) plus
    // one more standing Fisher to push food to -2 with only 1 standing.
    state.players[0].placed = vec![
        PlacedCard {
            card: survivor_card(71_000, SurvivorId::Fisher),
            hungry: false,
        },
        PlacedCard {
            card: survivor_card(71_001, SurvivorId::Fisher),
            hungry: true,
        },
        PlacedCard {
            card: survivor_card(71_002, SurvivorId::Fisher),
            hungry: true,
        },
    ];
    // food = 1 - 1 - 1 - 1 = -2; only 1 standing survivor to flip.
    state.phase = Phase::Actions;
    state.players[0].actions_remaining = 1;
    state.players[0].hand.clear();

    apply(game, &mut state, 0, Action::FinishActions);
    assert_eq!(state.phase, Phase::ResolvingDecision);
    // 1 standing forced-flipped automatically; shortfall of 1 must be
    // abandoned from the (now) 3 Hungry survivors — a real choice.
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::AbandonHungry { needed: 1 }
    );
    assert_eq!(
        state.players[0].placed.iter().filter(|p| p.hungry).count(),
        3,
        "the previously-standing Fisher should already be Hungry"
    );

    let legal = game.legal_actions(&state, 0);
    let abandoned_id = match legal[0] {
        Action::ResolveDecision {
            choice: DecisionChoice::AbandonHungry { survivor },
        } => survivor,
        other => panic!("expected AbandonHungry, got {other:?}"),
    };
    apply(game, &mut state, 0, legal[0]);
    assert_eq!(state.players[0].placed.len(), 2);
    assert!(
        state.players[0]
            .current
            .iter()
            .any(|c| c.card.id == abandoned_id && c.face == Face::Up),
        "abandoned survivor must land face-up in the Current"
    );
}

#[test]
fn positive_food_stands_up_all_hungry_when_food_covers_them() {
    let (game, mut state) = setup_2p(13);
    // Raft Right +1, one Survivalist (0 food) placed and Hungry.
    // food = 1 + 0 = 1 >= hungry_count (1) => auto-stand.
    state.players[0].placed = vec![PlacedCard {
        card: survivor_card(72_000, SurvivorId::Survivalist),
        hungry: true,
    }];
    state.phase = Phase::Actions;
    state.players[0].actions_remaining = 1;
    state.players[0].hand.clear();

    apply(game, &mut state, 0, Action::FinishActions);
    assert!(!state.players[0].placed[0].hungry, "auto-stand should fire");
    assert_ne!(state.phase, Phase::ResolvingDecision);
}

#[test]
fn positive_food_below_hungry_count_opens_stand_up_decision() {
    let (game, mut state) = setup_2p(14);
    // Raft Right +1, one Survivalist (0) hungry + one Swimmer (+1)
    // hungry => food = 1 + 0 + 1 = 2, hungry_count = 2: 2 >= 2, auto.
    // Use three hungry Survivalists instead: food = 1, hungry_count=3,
    // 1 < 3 => a real stand-up choice of 1.
    state.players[0].placed = vec![
        PlacedCard {
            card: survivor_card(73_000, SurvivorId::Survivalist),
            hungry: true,
        },
        PlacedCard {
            card: survivor_card(73_001, SurvivorId::Survivalist),
            hungry: true,
        },
        PlacedCard {
            card: survivor_card(73_002, SurvivorId::Survivalist),
            hungry: true,
        },
    ];
    state.phase = Phase::Actions;
    state.players[0].actions_remaining = 1;
    state.players[0].hand.clear();

    apply(game, &mut state, 0, Action::FinishActions);
    assert_eq!(state.phase, Phase::ResolvingDecision);
    assert_eq!(
        state.current_pending().unwrap().kind,
        PendingDecisionKind::StandUp { needed: 1 }
    );
    let legal = game.legal_actions(&state, 0);
    assert_eq!(legal.len(), 3, "any of the 3 Hungry survivors may stand");
}

// ---------- final-round trigger mid-phase-1 ---------------------------------

#[test]
fn deep_sea_deck_exhaustion_triggers_final_round_mid_round() {
    let (game, mut state) = setup_2p(15);
    // Drain the deep sea deck down to 0 so seat 1's Phase-1 add must
    // fall back to the Final Round Deck.
    state.deep_sea_deck.clear();
    assert!(!state.final_round);

    // Drive seat 0 through a minimal turn (no draws, no actions).
    apply(game, &mut state, 0, Action::FinishDrawing);
    apply(game, &mut state, 0, Action::FinishActions);

    // Seat 1's Phase 1 add should have come from the Final Round Deck
    // and flipped `final_round`.
    assert!(state.final_round, "final round must trigger");
    assert_eq!(state.current_player, 1);
    assert_eq!(state.phase, Phase::Draw);
}

#[test]
fn final_round_completes_without_phase_5_and_ends_the_game() {
    let (game, mut state) = setup_2p(16);
    state.deep_sea_deck.clear();
    state.final_round_deck.truncate(4); // plenty for 2 more adds

    // Seat 0's turn.
    apply(game, &mut state, 0, Action::FinishDrawing);
    apply(game, &mut state, 0, Action::FinishActions);
    assert!(state.final_round);
    assert_eq!(state.current_player, 1);

    // Seat 1's turn — round completes; Phase 5 must be skipped and the
    // game must end instead of passing Currents / starting a new round.
    apply(game, &mut state, 1, Action::FinishDrawing);
    apply(game, &mut state, 1, Action::FinishActions);

    assert_eq!(state.phase, Phase::Finished);
    assert!(game.game_over(&state).is_some());
}

// ---------- scripted full game ----------------------------------------------

#[test]
fn scripted_two_player_game_exercises_every_phase_and_terminates() {
    let (game, mut state) = setup_2p(2024);
    let mut ticks = 0usize;
    let mut rng = ProductionRng::from_seed(999);
    loop {
        if game.game_over(&state).is_some() {
            break;
        }
        ticks += 1;
        assert!(ticks < 20_000, "scripted game did not terminate");
        match game.next_actor(&state) {
            Actor::Chance => {
                let ev = game.resolve_chance(&state, &mut rng).unwrap();
                game.apply_event(&mut state, &ev);
            }
            Actor::Player(p) => {
                let legal = game.legal_actions(&state, p);
                assert!(!legal.is_empty(), "legal_actions must never be empty for a prompted player");
                // Prefer FinishDrawing/FinishActions to keep the
                // script terminating quickly and deterministically;
                // otherwise take the first legal option.
                let choice = legal
                    .iter()
                    .position(|a| matches!(a, Action::FinishDrawing | Action::FinishActions))
                    .unwrap_or(0);
                let events = game.apply_action(&state, p, &legal[choice]).unwrap();
                for e in &events {
                    game.apply_event(&mut state, e);
                }
            }
        }
    }
    let result = game.game_over(&state).unwrap();
    assert_eq!(result.scores.len(), 2);
}
