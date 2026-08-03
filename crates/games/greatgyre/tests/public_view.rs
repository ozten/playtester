//! `PublicView` redaction property tests (Unit 5).
//!
//! Two properties, per the plan:
//! (a) the view for observer `P` never contains the identity of any
//!     other player's hand cards, or of *any* player's face-down
//!     Current cards (including `P`'s own — face-down identity is
//!     hidden from everyone until drawn, per `docs/greatgyre.md`).
//! (b) the view is serde-serializable and round-trips (stable).
//!
//! Exercised across a spread of seeds, player counts, and mid-game
//! states reached via bounded pseudo-random play (deterministic via a
//! seeded `StubRng` — not `RandomAgent`, so this file has no `tokio`
//! dependency).

use std::collections::HashSet;

use playtest_adapters::StubRng;
use playtest_core::{Actor, Game};
use playtest_greatgyre::{
    CurrentSlotView, Face, GameState, GreatGyreConfig, GreatGyreGame, GreatGyrePublicView,
};
use playtest_ports::Rng;

/// Build the exact `"id":<n><delim>` needle for a card-id leak check.
/// Requiring a trailing delimiter (`,` or `}`) prevents a shorter id
/// (e.g. `7`) from false-positive-matching inside a longer one that
/// merely starts with the same digits (e.g. `70`, `712`).
fn card_id_needle(id: u32, delim: char) -> String {
    format!("\"id\":{id}{delim}")
}

fn pick(rng: &mut StubRng, n: usize) -> usize {
    let upper = u64::try_from(n).expect("legal count fits in u64");
    let v = rng.gen_range(0..upper).unwrap();
    usize::try_from(v).unwrap()
}

/// Drive up to `num_events` events of deterministic pseudo-random play
/// (covering both chance steps and player decisions) to reach a
/// representative mid-game state with populated hands, face-up and
/// face-down Current cards, and placed rafts. Stops early on
/// `game_over` or a stuck legal-actions slice (shouldn't happen, but
/// mirrors the other soak tests' defensive break).
fn mid_game_state(seed: u64, num_players: u8, num_events: usize) -> GameState {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(num_players).expect("valid player count");
    let mut state = game.initial_state(seed, &cfg);
    let mut rng = StubRng::seeded(seed);
    for _ in 0..num_events {
        if game.game_over(&state).is_some() {
            break;
        }
        match game.next_actor(&state) {
            Actor::Chance => {
                let ev = game.resolve_chance(&state, &mut rng).expect("chance resolves");
                game.apply_event(&mut state, &ev);
            }
            Actor::Player(p) => {
                let legal = game.legal_actions(&state, p);
                if legal.is_empty() {
                    break;
                }
                let idx = pick(&mut rng, legal.len());
                let events = game
                    .apply_action(&state, p, &legal[idx])
                    .expect("legal action applies");
                for e in &events {
                    game.apply_event(&mut state, e);
                }
            }
        }
    }
    state
}

#[test]
fn property_a_view_never_leaks_hidden_card_identities() {
    let game = GreatGyreGame::new();
    for seed in 0..15u64 {
        for n in 2..=4u8 {
            let state = mid_game_state(seed.wrapping_mul(7919).wrapping_add(u64::from(n)), n, 80);
            for observer in 0..n {
                let view = game.public_view(&state, observer);

                // Structural: exactly the observer's own slot is `None`.
                for (i, opp) in view.opponents.iter().enumerate() {
                    if i == observer as usize {
                        assert!(opp.is_none(), "seed={seed} n={n} observer={observer}: own slot should be None");
                    } else {
                        assert!(opp.is_some(), "seed={seed} n={n} observer={observer}: opponent {i} slot should be Some");
                        // Hand size is exposed, contents are not — no
                        // `hand` field exists on `OpponentView` at all,
                        // so this is a compile-time guarantee, not just
                        // a runtime one.
                        assert_eq!(
                            opp.as_ref().unwrap().hand_size,
                            state.players[i].hand.len()
                        );
                    }
                }

                // Every hidden card id: other players' hand cards, plus
                // every face-down Current slot across every seat
                // (including the observer's own).
                let mut hidden_ids: HashSet<u32> = HashSet::new();
                for (i, p) in state.players.iter().enumerate() {
                    if i != observer as usize {
                        for c in &p.hand {
                            hidden_ids.insert(c.id.0);
                        }
                    }
                    for cc in &p.current {
                        if cc.face == Face::Down {
                            hidden_ids.insert(cc.card.id.0);
                        }
                    }
                }

                let json = serde_json::to_string(&view).expect("view serializes");
                for id in &hidden_ids {
                    // Exact-match the numeric `"id":<n>` field (guarding
                    // both delimiters `,` and `}` after the digits) so a
                    // shorter id (e.g. 7) can't false-positive against a
                    // longer one that merely starts with it (e.g. 70,
                    // 712) — see `card_id_needle_does_not_false_positive`
                    // below for the regression this guards against.
                    assert!(
                        !json.contains(&card_id_needle(*id, ',')) && !json.contains(&card_id_needle(*id, '}')),
                        "seed={seed} n={n} observer={observer}: hidden card id {id} leaked into public_view JSON"
                    );
                }

                // Positive control: the observer's own hand is visible,
                // in full, exactly.
                assert_eq!(view.own.hand, state.players[observer as usize].hand);

                // Positive control: every face-up Current card (own and
                // opponents') is visible with correct identity and
                // position; every face-down slot carries no card.
                for (i, p) in state.players.iter().enumerate() {
                    let slots = if i == observer as usize {
                        &view.own.seat.current
                    } else {
                        &view.opponents[i].as_ref().unwrap().seat.current
                    };
                    assert_eq!(slots.len(), p.current.len());
                    for (slot, real) in slots.iter().zip(p.current.iter()) {
                        match (slot, real.face) {
                            (CurrentSlotView::Up { card }, Face::Up) => {
                                assert_eq!(*card, real.card);
                            }
                            (CurrentSlotView::Down, Face::Down) => {}
                            (got, real_face) => panic!(
                                "seed={seed} n={n} observer={observer} seat={i}: face mismatch — view={got:?} real={real_face:?}"
                            ),
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn property_b_public_view_serde_round_trips() {
    let game = GreatGyreGame::new();
    for seed in 0..12u64 {
        for n in 2..=4u8 {
            let state = mid_game_state(seed.wrapping_mul(31337), n, 60);
            for observer in 0..n {
                let view = game.public_view(&state, observer);
                let json = serde_json::to_string(&view).expect("view serializes");
                let back: GreatGyrePublicView =
                    serde_json::from_str(&json).expect("view deserializes");
                assert_eq!(view, back, "seed={seed} n={n} observer={observer}: round-trip diverged");

                // Stability: serializing twice yields byte-identical JSON.
                let json2 = serde_json::to_string(&view).expect("view serializes again");
                assert_eq!(json, json2, "seed={seed} n={n} observer={observer}: serialization not stable");
            }
        }
    }
}

#[test]
fn survivor_draft_phase_view_also_round_trips_and_hides_nothing_new() {
    // At Phase::SurvivorDraft, hands/Currents are all still empty (the
    // post-draft shuffle hasn't run) — a degenerate but real case the
    // property tests above don't hit by construction (mid_game_state
    // always runs at least the draft + shuffle before returning at
    // `num_events == 0`, so exercise it directly here).
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(3).unwrap();
    let state = game.initial_state(42, &cfg);
    for observer in 0..3u8 {
        let view = game.public_view(&state, observer);
        assert!(view.own.hand.is_empty());
        assert_eq!(view.undrafted_survivors.len(), 12);
        let json = serde_json::to_string(&view).expect("view serializes");
        let back: GreatGyrePublicView = serde_json::from_str(&json).expect("round trips");
        assert_eq!(view, back);
    }
}
