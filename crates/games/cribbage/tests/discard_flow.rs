//! State-machine tests for deal, discard, cut, and pegging flow.
//!
//! The plan's integration test calls for "full deal→discard→cut→
//! pegging flow with two RandomAgents". Unit 9 adds the
//! `impl Game for CribbageGame` that lets `RandomAgent` play through
//! `GameLoop` naturally; in Unit 8 we substitute a minimal driver
//! that walks the same state machine by picking random legal actions
//! from two seeded `StubRng`s. Semantically identical to random-vs-
//! random play.

use playtest_adapters::StubRng;
use playtest_core::{Actor, GameError};
use playtest_cribbage::{Action, Event, GameState, PegReason, Phase, Rank};
use playtest_ports::Rng;

// ---------- Helpers -----------------------------------------------------

fn deal_all(state: &mut GameState, rng: &mut dyn Rng) {
    while state.phase == Phase::Deal {
        let ev = state.resolve_chance(rng).unwrap();
        state.apply_event(&ev);
    }
}

fn deal_and_discard_both(
    state: &mut GameState,
    chance_rng: &mut dyn Rng,
    action_rngs: &mut [StubRng; 2],
) {
    deal_all(state, chance_rng);
    while state.phase == Phase::Discard {
        let player = state.to_act;
        let legal = state.legal_actions(player);
        let idx = pick(&mut action_rngs[player as usize], legal.len());
        let events = state.apply_action(player, &legal[idx]).unwrap();
        for e in &events {
            state.apply_event(e);
        }
    }
}

fn do_cut(state: &mut GameState, chance_rng: &mut dyn Rng) {
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(chance_rng).unwrap();
        state.apply_event(&ev);
    }
}

fn pick(rng: &mut StubRng, n: usize) -> usize {
    let upper = u64::try_from(n).expect("legal count fits in u64");
    let idx = rng.gen_range(0..upper).unwrap();
    usize::try_from(idx).expect("idx < n")
}

fn run_pegging(state: &mut GameState, action_rngs: &mut [StubRng; 2]) {
    while state.phase == Phase::Pegging {
        let player = state.to_act;
        let legal = state.legal_actions(player);
        assert!(
            !legal.is_empty(),
            "pegging reached a state with no legal actions for player {player}"
        );
        let idx = pick(&mut action_rngs[player as usize], legal.len());
        let events = state.apply_action(player, &legal[idx]).unwrap();
        for e in &events {
            state.apply_event(e);
        }
    }
}

// ---------- Happy paths -------------------------------------------------

#[test]
fn deal_produces_twelve_cards_and_transitions_to_discard() {
    let mut state = GameState::new(0);
    let mut rng = StubRng::seeded(1);
    deal_all(&mut state, &mut rng);
    assert_eq!(state.phase, Phase::Discard);
    assert_eq!(state.hands[0].len(), 6);
    assert_eq!(state.hands[1].len(), 6);
    assert_eq!(state.deck.len(), 40);
    // Non-dealer discards first.
    assert_eq!(state.to_act, state.non_dealer());
}

#[test]
fn discard_fills_crib_and_transitions_to_cut() {
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(1);
    let mut act = [StubRng::seeded(2), StubRng::seeded(3)];
    deal_and_discard_both(&mut state, &mut chance, &mut act);
    assert_eq!(state.phase, Phase::Cut);
    assert_eq!(state.hands[0].len(), 4);
    assert_eq!(state.hands[1].len(), 4);
    assert_eq!(state.crib.len(), 4);
}

#[test]
fn cut_transitions_to_pegging_when_starter_is_not_a_jack() {
    // Try multiple seeds until we find a non-Jack starter, so the test
    // isn't flaky. The probability of 100 consecutive Jack cuts is
    // vanishingly small (4/52)^100.
    for seed in 1..=100u64 {
        let mut state = GameState::new(0);
        let mut chance = StubRng::seeded(seed);
        let mut act = [
            StubRng::seeded(seed.wrapping_mul(7)),
            StubRng::seeded(seed.wrapping_mul(11)),
        ];
        deal_and_discard_both(&mut state, &mut chance, &mut act);
        do_cut(&mut state, &mut chance);
        if state.starter.unwrap().rank != Rank::Jack {
            assert_eq!(state.phase, Phase::Pegging);
            assert_eq!(state.to_act, state.non_dealer());
            return;
        }
    }
    panic!("could not find a non-Jack starter across 100 seeds");
}

#[test]
fn nibs_scores_two_for_dealer_when_starter_is_jack() {
    // Find a seed that produces a Jack starter, then verify nibs.
    for seed in 1..=200u64 {
        let mut state = GameState::new(0);
        let mut chance = StubRng::seeded(seed);
        let mut act = [
            StubRng::seeded(seed.wrapping_mul(13)),
            StubRng::seeded(seed.wrapping_mul(17)),
        ];
        deal_and_discard_both(&mut state, &mut chance, &mut act);
        // Cut one card at a time so we can observe events.
        let cut_ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&cut_ev);
        let Event::CutStarter { card } = cut_ev else {
            panic!("expected CutStarter");
        };
        if card.rank == Rank::Jack {
            // Phase should still be Cut (nibs pending).
            assert_eq!(state.phase, Phase::Cut);
            let nibs_ev = state.resolve_chance(&mut chance).unwrap();
            assert!(matches!(nibs_ev, Event::NibsScored { points: 2, .. }));
            state.apply_event(&nibs_ev);
            assert_eq!(state.phase, Phase::Pegging);
            assert_eq!(state.board.score(state.dealer), 2);
            return;
        }
    }
    panic!("no Jack starter found in 200 seeds — unlikely enough to indicate a bug");
}

// ---------- Error paths -------------------------------------------------

#[test]
fn play_card_not_in_hand_is_rejected() {
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(7);
    let mut act = [StubRng::seeded(8), StubRng::seeded(9)];
    deal_and_discard_both(&mut state, &mut chance, &mut act);
    do_cut(&mut state, &mut chance);
    // If nibs is pending, resolve that too.
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&ev);
    }

    let player = state.to_act;
    let my_hand = state.hands[player as usize].cards().to_vec();
    let other_hand = state.hands[(1 - player) as usize].cards().to_vec();
    let not_mine = *other_hand
        .iter()
        .find(|c| !my_hand.contains(c))
        .expect("player hands are disjoint");
    let err = state
        .apply_action(player, &Action::PlayCard(not_mine))
        .unwrap_err();
    assert!(
        matches!(err, GameError::IllegalAction { .. }),
        "got {err:?}"
    );
}

#[test]
fn say_go_when_a_legal_play_exists_is_rejected() {
    // Directly hit the Pegging phase via a rigged state — easier than
    // trying to engineer a stuck situation.
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(42);
    let mut act = [StubRng::seeded(11), StubRng::seeded(13)];
    deal_and_discard_both(&mut state, &mut chance, &mut act);
    do_cut(&mut state, &mut chance);
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&ev);
    }

    assert_eq!(state.phase, Phase::Pegging);
    let player = state.to_act;
    // At the start of pegging, running_total = 0, so every card in
    // hand is playable. SayGo should be illegal.
    let err = state.apply_action(player, &Action::SayGo).unwrap_err();
    assert!(
        matches!(err, GameError::IllegalAction { .. }),
        "got {err:?}"
    );
}

#[test]
fn play_card_pushing_over_31_is_rejected() {
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(99);
    let mut act = [StubRng::seeded(21), StubRng::seeded(23)];
    deal_and_discard_both(&mut state, &mut chance, &mut act);
    do_cut(&mut state, &mut chance);
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&ev);
    }

    // Force running_total close to 31 by direct mutation — testing the
    // `apply_action` validator, not the path to it.
    state.running_total = 28;
    let player = state.to_act;
    let hand = state.hands[player as usize].cards().to_vec();
    // Find a card with value > 3. Faces are 10, most pips work.
    let too_big = *hand
        .iter()
        .find(|c| c.value() > 3)
        .expect("6-card hand should contain at least one card valued over 3");
    let err = state
        .apply_action(player, &Action::PlayCard(too_big))
        .unwrap_err();
    assert!(
        matches!(err, GameError::IllegalAction { .. }),
        "got {err:?}"
    );
}

// ---------- Integration: 1000 full plays --------------------------------

#[test]
fn one_thousand_random_games_reach_pegging_complete() {
    // The soak test: for every seed in 1..=1000, the state machine
    // must reach `Phase::Show` (pegging complete) without panicking
    // and without any `apply_action` returning an error.
    let mut completed = 0;
    let mut ended_early = 0;
    for seed in 1..=1000u64 {
        let mut state = GameState::new((seed % 2) as u8);
        let mut chance = StubRng::seeded(seed);
        let mut act = [
            StubRng::seeded(seed.wrapping_mul(31)),
            StubRng::seeded(seed.wrapping_mul(41)),
        ];
        deal_and_discard_both(&mut state, &mut chance, &mut act);
        do_cut(&mut state, &mut chance);
        while state.phase == Phase::Cut {
            let ev = state.resolve_chance(&mut chance).unwrap();
            state.apply_event(&ev);
        }
        run_pegging(&mut state, &mut act);

        match state.phase {
            Phase::Show => completed += 1,
            Phase::Finished => ended_early += 1,
            other => panic!("seed {seed}: unexpected terminal phase {other:?}"),
        }
    }
    // Nearly every game reaches Show. A handful could end early with
    // Phase::Finished if pegging hits 121 — from a single deal's worth
    // of pegging (max ~29 points for a run-of-7-plus-quad-plus-31
    // situation), that's impossible from a fresh board. Assert:
    assert_eq!(
        completed, 1000,
        "every game should reach Show; ended_early={ended_early}"
    );
}

#[test]
fn all_pegging_events_are_serializable_as_json() {
    // Phase 1's ingestion pass reads the JSONL log, so every event
    // variant must survive a round trip through serde_json.
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(7);
    let mut act = [StubRng::seeded(17), StubRng::seeded(19)];
    deal_and_discard_both(&mut state, &mut chance, &mut act);
    do_cut(&mut state, &mut chance);
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(&mut chance).unwrap();
        let _ = serde_json::to_string(&ev).expect("chance event serializes");
        state.apply_event(&ev);
    }
    let mut events_seen: Vec<String> = Vec::new();
    while state.phase == Phase::Pegging {
        let player = state.to_act;
        let legal = state.legal_actions(player);
        let idx = pick(&mut act[player as usize], legal.len());
        let batch = state.apply_action(player, &legal[idx]).unwrap();
        for e in &batch {
            let line = serde_json::to_string(e).expect("peg event serializes");
            events_seen.push(line);
            state.apply_event(e);
        }
    }
    // Sanity: we saw at least a few events.
    assert!(events_seen.len() >= 8);
    // And at least one PegPlayed event round-trips.
    let sample = events_seen
        .iter()
        .find(|l| l.contains("\"kind\":\"peg_played\""))
        .expect("at least one PegPlayed emitted");
    let parsed: Event = serde_json::from_str(sample).expect("round trip");
    assert!(matches!(parsed, Event::PegPlayed { .. }));
}

#[test]
fn pegging_emits_last_card_event_when_round_ends_below_thirty_one() {
    // Drive many games and verify that at least one produces a
    // `PegScored { reason: LastCard }` event. With 50 random games
    // it's overwhelmingly likely to happen.
    let mut saw_last_card = false;
    let mut saw_round_end = false;
    for seed in 1..=50u64 {
        let mut state = GameState::new(0);
        let mut chance = StubRng::seeded(seed);
        let mut act = [
            StubRng::seeded(seed.wrapping_mul(101)),
            StubRng::seeded(seed.wrapping_mul(103)),
        ];
        deal_and_discard_both(&mut state, &mut chance, &mut act);
        do_cut(&mut state, &mut chance);
        while state.phase == Phase::Cut {
            let ev = state.resolve_chance(&mut chance).unwrap();
            state.apply_event(&ev);
        }
        while state.phase == Phase::Pegging {
            let player = state.to_act;
            let legal = state.legal_actions(player);
            let idx = pick(&mut act[player as usize], legal.len());
            let batch = state.apply_action(player, &legal[idx]).unwrap();
            for e in &batch {
                match e {
                    Event::PegScored {
                        reason: PegReason::LastCard,
                        ..
                    } => saw_last_card = true,
                    Event::PeggingRoundEnd => saw_round_end = true,
                    _ => {}
                }
                state.apply_event(e);
            }
        }
    }
    assert!(
        saw_round_end,
        "50 games must produce at least one PeggingRoundEnd"
    );
    assert!(
        saw_last_card,
        "50 games must produce at least one LastCard event"
    );
}

#[test]
fn next_actor_reports_chance_during_deal_and_cut_then_players_during_discard_and_pegging() {
    let mut state = GameState::new(0);
    let mut chance = StubRng::seeded(123);
    let mut act = [StubRng::seeded(45), StubRng::seeded(67)];

    assert_eq!(state.next_actor(), Some(Actor::Chance));
    deal_all(&mut state, &mut chance);
    assert_eq!(state.next_actor(), Some(Actor::Player(state.non_dealer())));

    while state.phase == Phase::Discard {
        let player = state.to_act;
        let legal = state.legal_actions(player);
        let idx = pick(&mut act[player as usize], legal.len());
        for e in &state.apply_action(player, &legal[idx]).unwrap() {
            state.apply_event(e);
        }
    }

    assert_eq!(state.next_actor(), Some(Actor::Chance)); // Cut
    do_cut(&mut state, &mut chance);
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&ev);
    }
    assert_eq!(state.next_actor(), Some(Actor::Player(state.non_dealer())));
}
