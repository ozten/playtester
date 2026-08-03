//! Unit 3 integration tests: active per-turn budget bonuses and the
//! three special Phase-2 draw sources.

use playtest_adapters::{ProductionRng, StubRng};
use playtest_core::{Actor, Game, GameError};
use playtest_greatgyre::state::PlacedCard;
use playtest_greatgyre::{
    Action, Card, CardInstanceId, CardKind, GreatGyreConfig, GreatGyreGame, ModificationKind,
    Phase, Resource, SurvivorId,
};

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

fn placed_survivor(id: u32, s: SurvivorId) -> PlacedCard {
    PlacedCard {
        card: Card::new(CardInstanceId(id), CardKind::Survivor(s)),
        hungry: false,
    }
}

fn placed_mod(id: u32, m: ModificationKind) -> PlacedCard {
    PlacedCard {
        card: Card::new(CardInstanceId(id), CardKind::Modification(m)),
        hungry: false,
    }
}

fn resource_card(id: u32, r: Resource) -> Card {
    Card::new(CardInstanceId(id), CardKind::Resource(r))
}

// ---------- Phase 1: add_bonus ---------------------------------------------

#[test]
fn millionaire_and_sail_increase_phase1_add_count() {
    // Turn-order rotation for n=2, first_player starting at 0:
    // round 1 = [seat0, seat1]; round 1 completing passes the First
    // Player token to seat 1, so round 2 = [seat1, seat0] — seat 1
    // plays twice in a row (last of round 1, first of round 2) before
    // seat 0's second turn begins. Drive exactly that many turns so
    // the final `FinishActions` call's event batch contains seat 0's
    // *second* `TurnStarted`/`CurrentCardAdded` — the first place the
    // Millionaire + Sail bonus (placed before any of this) can show up.
    let (game, mut state) = setup_2p(1);
    state.players[0].placed = vec![
        placed_survivor(60_000, SurvivorId::Millionaire),
        placed_mod(60_001, ModificationKind::Sail),
    ];
    state.players[0].hand.clear();
    state.players[1].hand.clear();

    apply(game, &mut state, 0, Action::FinishDrawing); // seat0, round1
    apply(game, &mut state, 0, Action::FinishActions);
    assert_eq!(state.current_player, 1);

    apply(game, &mut state, 1, Action::FinishDrawing); // seat1, round1 (last)
    let events_round1_last = game
        .apply_action(&state, 1, &Action::FinishActions)
        .unwrap();
    for e in &events_round1_last {
        game.apply_event(&mut state, e);
    }
    assert!(
        events_round1_last
            .iter()
            .any(|e| matches!(e, playtest_greatgyre::Event::CurrentsPassed { .. })),
        "round 1 should complete here: {events_round1_last:#?}"
    );
    assert_eq!(state.current_player, 1, "seat 1 goes first in round 2 too");

    apply(game, &mut state, 1, Action::FinishDrawing); // seat1, round2 (first)
    let events_round2_first = game
        .apply_action(&state, 1, &Action::FinishActions)
        .unwrap();
    let add_events_seat0 = events_round2_first
        .iter()
        .filter(|e| matches!(e, playtest_greatgyre::Event::CurrentCardAdded { player: 0, .. }))
        .count();
    // Seat 0's second turn begins here: base 1 + Sail(+1) + Millionaire(+1) = 3.
    assert_eq!(add_events_seat0, 3, "{events_round2_first:#?}");
}

// ---------- Phase 2: draw_bonus + special sources ---------------------------

#[test]
fn net_and_athlete_increase_draw_budget() {
    let (game, mut state) = setup_2p(3);
    state.players[0].placed = vec![
        placed_mod(62_000, ModificationKind::Net),
        placed_mod(62_001, ModificationKind::Net),
        placed_survivor(62_002, SurvivorId::Athlete),
    ];
    // draws_remaining was already set once at PostDraftSetup without
    // these mods; force a fresh TurnStarted to observe the bonus.
    let ev = playtest_greatgyre::Event::TurnStarted { player: 0 };
    game.apply_event(&mut state, &ev);
    assert_eq!(state.players[0].draws_remaining, 1 + 2 + 1);
}

#[test]
fn toolkit_and_first_mate_increase_action_budget() {
    let (game, mut state) = setup_2p(4);
    state.players[0].placed = vec![
        placed_mod(63_000, ModificationKind::Toolkit),
        placed_survivor(63_001, SurvivorId::FirstMate),
    ];
    let ev = playtest_greatgyre::Event::TurnStarted { player: 0 };
    game.apply_event(&mut state, &ev);
    assert_eq!(state.players[0].actions_remaining, 1 + 1 + 1);
}

#[test]
fn porter_draws_top_of_discard_pile_and_is_usable_once() {
    let (game, mut state) = setup_2p(5);
    state.players[0].placed = vec![placed_survivor(64_000, SurvivorId::Porter)];
    state.players[0].draws_remaining = 2;
    let top = resource_card(64_001, Resource::Rope);
    state.discard_pile.push(top);
    let hand_before = state.players[0].hand.len();

    let legal = game.legal_actions(&state, 0);
    assert!(
        legal.contains(&Action::DrawFromDiscardPile),
        "Porter should offer DrawFromDiscardPile: {legal:#?}"
    );

    apply(game, &mut state, 0, Action::DrawFromDiscardPile);
    assert_eq!(state.players[0].hand.len(), hand_before + 1);
    assert!(state.players[0].hand.iter().any(|c| c.id == top.id));
    assert!(!state.discard_pile.iter().any(|c| c.id == top.id));
    assert!(state.players[0].porter_used);

    // Used once already this Phase 2 — no longer offered even though
    // there's more in the discard pile and draws remain.
    state.discard_pile.push(resource_card(64_002, Resource::Wood));
    let legal_again = game.legal_actions(&state, 0);
    assert!(!legal_again.contains(&Action::DrawFromDiscardPile));
}

#[test]
fn swimmer_draws_from_adjacent_current() {
    let (game, mut state) = setup_2p(6);
    state.players[0].placed = vec![placed_survivor(65_000, SurvivorId::Swimmer)];
    state.players[0].draws_remaining = 1;
    let neighbor_card = state.players[1].current[0].card;

    let legal = game.legal_actions(&state, 0);
    let action = Action::DrawFromAdjacentCurrent {
        neighbor: 1,
        card: neighbor_card.id,
    };
    assert!(legal.contains(&action), "{legal:#?}");

    let neighbor_len_before = state.players[1].current.len();
    apply(game, &mut state, 0, action);
    assert!(state.players[0].hand.iter().any(|c| c.id == neighbor_card.id));
    assert_eq!(state.players[1].current.len(), neighbor_len_before - 1);
    assert!(state.players[0].swimmer_used);
}

#[test]
fn pirate_steal_goes_through_actor_chance_and_moves_a_card() {
    let (game, mut state) = setup_2p(7);
    state.players[0].placed = vec![placed_survivor(66_000, SurvivorId::Pirate)];
    state.players[0].draws_remaining = 1;
    // Force target's hand to a known, single card so the "random"
    // pick is deterministic for the assertion.
    let stolen = resource_card(66_001, Resource::Plastic);
    state.players[1].hand = vec![stolen];

    let legal = game.legal_actions(&state, 0);
    assert!(legal.contains(&Action::DrawRandomFromHand { target: 1 }));

    let events = game
        .apply_action(&state, 0, &Action::DrawRandomFromHand { target: 1 })
        .unwrap();
    for e in &events {
        game.apply_event(&mut state, e);
    }
    assert_eq!(state.phase, Phase::AwaitingPirateSteal);
    assert_eq!(game.next_actor(&state), Actor::Chance);
    assert!(game.legal_actions(&state, 0).is_empty());

    let mut rng = StubRng::seeded(42);
    let chance_event = game.resolve_chance(&state, &mut rng).unwrap();
    let playtest_greatgyre::Event::PirateStole { player, target, card } = &chance_event else {
        panic!("expected PirateStole, got {chance_event:?}");
    };
    assert_eq!(*player, 0);
    assert_eq!(*target, 1);
    assert_eq!(*card, stolen);
    game.apply_event(&mut state, &chance_event);

    assert_eq!(state.phase, Phase::Draw);
    assert!(state.players[0].hand.iter().any(|c| c.id == stolen.id));
    assert!(state.players[1].hand.is_empty());
    assert!(state.players[0].pirate_used);
}

#[test]
fn pirate_target_with_empty_hand_is_not_offered() {
    let (game, mut state) = setup_2p(8);
    state.players[0].placed = vec![placed_survivor(67_000, SurvivorId::Pirate)];
    state.players[0].draws_remaining = 1;
    state.players[1].hand.clear();
    let legal = game.legal_actions(&state, 0);
    assert!(!legal.iter().any(|a| matches!(a, Action::DrawRandomFromHand { .. })));
}

#[test]
fn special_source_without_survivor_is_illegal() {
    let (game, state) = setup_2p(9);
    // Seat 0's drafted survivor is Captain (first legal draft pick),
    // not Porter — DrawFromDiscardPile must be rejected.
    let err = game
        .apply_action(&state, 0, &Action::DrawFromDiscardPile)
        .unwrap_err();
    assert!(matches!(err, GameError::IllegalAction { .. }));
}

#[test]
fn special_draws_share_the_same_draw_budget_as_normal_draws() {
    let (game, mut state) = setup_2p(10);
    state.players[0].placed = vec![placed_survivor(68_000, SurvivorId::Porter)];
    state.players[0].draws_remaining = 1;
    state.discard_pile.push(resource_card(68_001, Resource::Rope));

    apply(game, &mut state, 0, Action::DrawFromDiscardPile);
    // Budget of 1 fully consumed by the Porter draw — phase must have
    // auto-advanced to Actions, exactly like a normal draw exhausting
    // the budget.
    assert_eq!(state.phase, Phase::Actions);
}
