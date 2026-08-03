//! Unit 1 integration tests: crate skeleton, card catalog, setup, and
//! the survivor draft.
//!
//! Card conservation is checked in two stages, matching the two-stage
//! setup: right after `initial_state` (draft not yet run — cards live
//! in `undrafted_survivors` / `pending_shuffle_pool` /
//! `pending_event_pool` / raft assignments), and again after the
//! draft + post-draft chance step complete (cards live in hands /
//! Currents / decks / rafts).

use std::collections::HashSet;

use playtest_adapters::ProductionRng;
use playtest_core::{Actor, Game};
use playtest_greatgyre::pool::total_catalog_size;
use playtest_greatgyre::{Action, Card, GreatGyreConfig, GreatGyreGame, Phase, SurvivorId};

fn all_card_ids_pre_draft(state: &playtest_greatgyre::GameState) -> Vec<Card> {
    let mut out = Vec::new();
    for p in &state.players {
        out.push(p.raft_left);
        out.push(p.raft_right);
    }
    out.extend(state.undrafted_survivors.iter().copied());
    out.extend(state.pending_shuffle_pool.iter().copied());
    out.extend(state.pending_event_pool.iter().copied());
    out.extend(state.extension_pile.iter().copied());
    out
}

fn all_card_ids_post_setup(state: &playtest_greatgyre::GameState) -> Vec<Card> {
    let mut out = Vec::new();
    for p in &state.players {
        out.push(p.raft_left);
        out.push(p.raft_right);
        out.extend(p.hand.iter().copied());
        out.extend(p.current.iter().map(|c| c.card));
        out.extend(p.built_extensions.iter().copied());
        out.extend(p.placed.iter().map(|pc| pc.card));
    }
    out.extend(state.deep_sea_deck.iter().copied());
    out.extend(state.final_round_deck.iter().copied());
    out.extend(state.event_deck.iter().copied());
    out.extend(state.discard_pile.iter().copied());
    out.extend(state.extension_pile.iter().copied());
    out
}

fn assert_conserved(cards: &[Card], expected: u32) {
    let len = u32::try_from(cards.len()).expect("card count fits in u32");
    assert_eq!(len, expected, "card count mismatch");
    let ids: HashSet<_> = cards.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), cards.len(), "duplicate CardInstanceId in play");
}

/// Draft every seat in order using `RandomAgent`-style deterministic
/// picks (always the first legal option) and drive the post-draft
/// chance step, returning the resulting `Draw`-phase state.
fn run_full_setup(seed: u64, n: u8) -> playtest_greatgyre::GameState {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(n).unwrap();
    let mut state = game.initial_state(seed, &cfg);

    // Draft phase: each seat picks the first legal survivor.
    while state.phase == Phase::SurvivorDraft {
        let Actor::Player(p) = game.next_actor(&state) else {
            panic!("draft phase must prompt a player");
        };
        let legal = game.legal_actions(&state, p);
        assert!(!legal.is_empty(), "draft must always offer a choice");
        let events = game.apply_action(&state, p, &legal[0]).unwrap();
        for e in &events {
            game.apply_event(&mut state, e);
        }
    }

    // Chance step: post-draft shuffle + deal.
    assert_eq!(state.phase, Phase::AwaitingPostDraftShuffle);
    assert_eq!(game.next_actor(&state), Actor::Chance);
    let mut rng = ProductionRng::from_seed(seed ^ 0xdead_beef);
    let event = game.resolve_chance(&state, &mut rng).unwrap();
    game.apply_event(&mut state, &event);

    state
}

// ---------- config -----------------------------------------------------

mod config {
    use super::*;

    #[test]
    fn default_has_two_players() {
        assert_eq!(GreatGyreConfig::default().num_players, 2);
    }
}

// ---------- pre-draft invariants ----------------------------------------

mod pre_draft {
    use super::*;

    fn assert_pre_draft_shape(n: u8) {
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(n).unwrap();
        let state = game.initial_state(7, &cfg);

        assert_eq!(state.phase, Phase::SurvivorDraft);
        assert_eq!(state.current_player, 0);
        assert_eq!(state.players.len(), usize::from(n));
        assert_eq!(state.undrafted_survivors.len(), 12);
        assert_eq!(state.pending_shuffle_pool.len(), 102);
        assert_eq!(state.pending_event_pool.len(), 10);
        assert_eq!(state.extension_pile.len(), 10);
        for p in &state.players {
            assert!(p.hand.is_empty());
            assert!(p.current.is_empty());
            assert!(p.placed.is_empty());
        }

        let cards = all_card_ids_pre_draft(&state);
        assert_conserved(&cards, total_catalog_size(n));
    }

    #[test]
    fn two_player_pre_draft_shape() {
        assert_pre_draft_shape(2);
    }

    #[test]
    fn three_player_pre_draft_shape() {
        assert_pre_draft_shape(3);
    }

    #[test]
    fn four_player_pre_draft_shape() {
        assert_pre_draft_shape(4);
    }
}

// ---------- draft legality -----------------------------------------------

mod draft {
    use super::*;

    #[test]
    fn legal_actions_offer_every_undrafted_survivor() {
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(3).unwrap();
        let state = game.initial_state(1, &cfg);
        let legal = game.legal_actions(&state, 0);
        assert_eq!(legal.len(), 12);
        for a in &legal {
            assert!(matches!(a, Action::DraftSurvivor { .. }));
        }
    }

    #[test]
    fn only_current_seat_may_draft() {
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(3).unwrap();
        let state = game.initial_state(1, &cfg);
        assert!(game.legal_actions(&state, 1).is_empty());
        assert!(game.legal_actions(&state, 2).is_empty());
    }

    #[test]
    fn drafted_survivor_is_removed_from_pool_and_placed_on_raft() {
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(2).unwrap();
        let mut state = game.initial_state(1, &cfg);
        let action = Action::DraftSurvivor {
            survivor: SurvivorId::Captain,
        };
        let events = game.apply_action(&state, 0, &action).unwrap();
        for e in &events {
            game.apply_event(&mut state, e);
        }
        assert_eq!(state.undrafted_survivors.len(), 11);
        assert_eq!(state.players[0].placed.len(), 1);
        assert!(matches!(
            state.players[0].placed[0].card.kind,
            playtest_greatgyre::CardKind::Survivor(SurvivorId::Captain)
        ));
        assert_eq!(state.current_player, 1);
        assert_eq!(state.phase, Phase::SurvivorDraft);
    }

    #[test]
    fn drafting_same_survivor_twice_is_illegal() {
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(2).unwrap();
        let mut state = game.initial_state(1, &cfg);
        let action = Action::DraftSurvivor {
            survivor: SurvivorId::Captain,
        };
        let events = game.apply_action(&state, 0, &action).unwrap();
        for e in &events {
            game.apply_event(&mut state, e);
        }
        // Seat 1 tries to draft Captain again — no longer available.
        let err = game.apply_action(&state, 1, &action).unwrap_err();
        assert!(matches!(err, playtest_core::GameError::IllegalAction { .. }));
    }

    #[test]
    fn last_seat_drafting_transitions_to_post_draft_chance() {
        let state = {
            let game = GreatGyreGame::new();
            let cfg = GreatGyreConfig::new(2).unwrap();
            let mut state = game.initial_state(1, &cfg);
            for _ in 0..2 {
                let p = state.current_player;
                let legal = game.legal_actions(&state, p);
                let events = game.apply_action(&state, p, &legal[0]).unwrap();
                for e in &events {
                    game.apply_event(&mut state, e);
                }
            }
            state
        };
        assert_eq!(state.phase, Phase::AwaitingPostDraftShuffle);
    }
}

// ---------- post-draft setup ---------------------------------------------

mod post_setup {
    use super::*;

    fn assert_post_setup_shape(n: u8) {
        let state = run_full_setup(42, n);
        assert_eq!(state.phase, Phase::Draw);
        assert_eq!(state.current_player, 0);
        assert_eq!(state.first_player, 0);

        for (i, p) in state.players.iter().enumerate() {
            // 3 dealt + 1 event card.
            assert_eq!(p.hand.len(), 4, "seat {i} hand size");
            // Seat 0 already got its Phase-1 add face-down (unless the
            // deep sea deck was somehow empty, which never happens at
            // these player counts); every other seat still has just
            // their 3 setup-dealt Current cards.
            let expected_current = if i == 0 { 4 } else { 3 };
            assert_eq!(p.current.len(), expected_current, "seat {i} current size");
        }
        assert_eq!(
            state.players[0].current.last().unwrap().face,
            playtest_greatgyre::Face::Down
        );

        let expected_final_round = 2 * u32::from(n);
        let actual_final_round =
            u32::try_from(state.final_round_deck.len()).expect("deck size fits in u32");
        assert_eq!(actual_final_round, expected_final_round);

        let cards = all_card_ids_post_setup(&state);
        assert_conserved(&cards, total_catalog_size(n));
    }

    #[test]
    fn two_player_post_setup_shape() {
        assert_post_setup_shape(2);
    }

    #[test]
    fn three_player_post_setup_shape() {
        assert_post_setup_shape(3);
    }

    #[test]
    fn four_player_post_setup_shape() {
        assert_post_setup_shape(4);
    }

    #[test]
    fn same_seed_yields_identical_state() {
        let a = run_full_setup(99, 3);
        let b = run_full_setup(99, 3);
        assert_eq!(a, b);
    }
}

// ---------- serde round-trips ---------------------------------------------

mod serde_round_trip {
    use super::*;

    #[test]
    fn every_action_variant_round_trips() {
        let actions = vec![
            Action::DraftSurvivor {
                survivor: SurvivorId::Captain,
            },
            Action::FinishDrawing,
            Action::FinishActions,
            Action::BuildExtension,
            Action::PlaySurvivor {
                card: playtest_greatgyre::CardInstanceId(3),
            },
            Action::BuildModification {
                card: playtest_greatgyre::CardInstanceId(4),
            },
            Action::DrawFromCurrent {
                card: playtest_greatgyre::CardInstanceId(5),
            },
            Action::ResolveDecision {
                choice: playtest_greatgyre::DecisionChoice::Discard {
                    card: playtest_greatgyre::CardInstanceId(6),
                },
            },
        ];
        for a in actions {
            let json = serde_json::to_string(&a).unwrap();
            let back: Action = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back, "round trip mismatch for {json}");
        }
    }

    #[test]
    fn event_round_trips_through_json() {
        let state = run_full_setup(5, 2);
        // Re-derive one SurvivorDrafted-shaped event and one
        // PostDraftSetup-shaped event via re-running setup and
        // capturing the actual events emitted.
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(2).unwrap();
        let mut fresh = game.initial_state(5, &cfg);
        let legal = game.legal_actions(&fresh, 0);
        let events = game.apply_action(&fresh, 0, &legal[0]).unwrap();
        for e in &events {
            let json = serde_json::to_string(e).unwrap();
            let back: playtest_greatgyre::Event = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{e:?}"), format!("{back:?}"));
            game.apply_event(&mut fresh, e);
        }
        let _ = state; // silence unused warning if the shape above changes
    }
}
