//! Integration tests for Unit 21 — ShipWreckConfig, state shape, and
//! the deterministic setup deal.

use playtest_adapters::ProductionRng;
use playtest_shipwreck::{
    Card, ConfigError, Event, MAX_PLAYERS, MIN_PLAYERS, Phase, ShipWreckConfig,
    all_equipment, all_wreckage_cards,
    setup::{Setup, WRECKAGE_HAND_SIZE, build_initial_state},
};

/// Pool size of the wreckage deck *before* the leftover player cards
/// are mixed in. `all_wreckage_cards()` already includes all 40
/// extensions + 150 items + 13 equipment + 18 events = 221.
fn wreckage_pool_size() -> usize {
    all_wreckage_cards().len()
}

/// Total cards in circulation across every pile during a full game,
/// after setup. Equals 7 player cards + wreckage pool. Used by the
/// face-up distribution tests to derive the per-seat count without
/// hardcoding a number that drifts if pool sizes ever change.
fn total_cards() -> usize {
    7 + wreckage_pool_size()
}

fn run_setup(seed: u64, n: u8) -> Setup {
    let cfg = ShipWreckConfig::new(n).expect("valid count");
    let mut rng = ProductionRng::from_seed(seed);
    build_initial_state(seed, &cfg, &mut rng)
}

// ---------- Config ------------------------------------------------------

mod config {
    use super::*;

    #[test]
    fn default_has_two_players() {
        assert_eq!(ShipWreckConfig::default().num_players, 2);
    }

    #[test]
    fn new_rejects_out_of_range() {
        assert_eq!(
            ShipWreckConfig::new(5),
            Err(ConfigError::InvalidPlayerCount { got: 5 })
        );
        assert_eq!(
            ShipWreckConfig::new(1),
            Err(ConfigError::InvalidPlayerCount { got: 1 })
        );
    }

    #[test]
    fn new_accepts_two_three_four() {
        for n in MIN_PLAYERS..=MAX_PLAYERS {
            assert!(ShipWreckConfig::new(n).is_ok(), "should accept {n}");
        }
    }
}

// ---------- Hand sizes --------------------------------------------------

mod hands {
    use super::*;

    fn assert_hand_shape(n: u8) {
        let setup = run_setup(42, n);
        let state = &setup.state;
        assert_eq!(
            state.players.len(),
            usize::from(n),
            "players vec length == num_players"
        );
        for seat in 0..usize::from(n) {
            let hand = &state.players[seat].hand;
            // 1 player card dealt + 6 wreckage cards = 7 items total.
            // A leftover player card (mixed into the wreckage deck per
            // the spec) *may* also be dealt back as a wreckage card, so
            // the count of `Card::Player` in-hand is ≥ 1, not exactly 1.
            assert_eq!(hand.len(), 1 + WRECKAGE_HAND_SIZE, "seat {seat} hand size");
            let player_card_count = hand
                .iter()
                .filter(|c| matches!(c, Card::Player(_)))
                .count();
            assert!(
                player_card_count >= 1,
                "seat {seat} must hold at least the initially-dealt player card"
            );
        }
    }

    #[test]
    fn two_player_hands_are_seven_cards_with_one_player_card() {
        assert_hand_shape(2);
    }

    #[test]
    fn three_player_hands_are_seven_cards_with_one_player_card() {
        assert_hand_shape(3);
    }

    #[test]
    fn four_player_hands_are_seven_cards_with_one_player_card() {
        assert_hand_shape(4);
    }
}

// ---------- Face-up pool distribution ----------------------------------

mod face_up {
    use super::*;

    /// Expected total face-up cards = total_cards - n*(1 + 6).
    fn expected_face_up_total(n: usize) -> usize {
        total_cards() - n * (1 + WRECKAGE_HAND_SIZE)
    }

    fn assert_face_up_distribution(n: u8) {
        let setup = run_setup(7, n);
        let nu = usize::from(n);
        let expected_total = expected_face_up_total(nu);
        let actual_total: usize =
            setup.state.face_up_pools.iter().map(Vec::len).sum();
        assert_eq!(
            actual_total, expected_total,
            "{n}-player face-up total mismatch"
        );

        // Round-robin: seat 0 gets the first card of each pass, so
        // lower seats have ≥ higher seats. All lengths are in
        // {floor(total/n), floor(total/n) + 1}.
        let base = expected_total / nu;
        let remainder = expected_total % nu;
        for (seat, pool) in setup.state.face_up_pools.iter().enumerate() {
            let expected = if seat < remainder { base + 1 } else { base };
            assert_eq!(
                pool.len(),
                expected,
                "{n}-player seat {seat} face-up size"
            );
        }
    }

    #[test]
    fn two_player_face_up_pools_are_even() {
        assert_face_up_distribution(2);
    }

    #[test]
    fn three_player_face_up_pools_are_even_when_divisible() {
        assert_face_up_distribution(3);
    }

    #[test]
    fn four_player_face_up_pools_are_even_when_divisible() {
        assert_face_up_distribution(4);
    }

    #[test]
    fn no_duplicate_cards_across_all_piles() {
        // Sanity: total cards in every pile = 7 (player cards) +
        // len(all_wreckage_cards()).
        let setup = run_setup(13, 3);
        let mut all: Vec<Card> = Vec::new();
        for p in &setup.state.players {
            all.extend(p.hand.iter().copied());
        }
        for pool in &setup.state.face_up_pools {
            all.extend(pool.iter().copied());
        }
        all.extend(setup.state.wreckage_deck.iter().copied());
        assert_eq!(all.len(), total_cards(), "card conservation");
    }
}

// ---------- Determinism + variability -----------------------------------

mod determinism {
    use super::*;

    #[test]
    fn same_seed_same_cfg_yields_identical_state() {
        let a = run_setup(99, 2);
        let b = run_setup(99, 2);
        assert_eq!(a.state, b.state);
        assert_eq!(a.events, b.events);
    }

    #[test]
    fn different_seeds_produce_different_face_ups_somewhere() {
        let mut pools_per_seed: Vec<Vec<Card>> = Vec::new();
        for seed in 0..10 {
            let setup = run_setup(seed, 2);
            // Use seat 0's face-up pool as the fingerprint.
            pools_per_seed.push(setup.state.face_up_pools[0].clone());
        }
        // Verify at least one pair differs.
        let any_diff = (0..pools_per_seed.len())
            .any(|i| (i + 1..pools_per_seed.len()).any(|j| pools_per_seed[i] != pools_per_seed[j]));
        assert!(
            any_diff,
            "10 different seeds produced identical seat-0 pools — something isn't using the RNG"
        );
    }
}

// ---------- Initial-state invariants ------------------------------------

mod initial_invariants {
    use super::*;

    #[test]
    fn phase_is_play_after_setup() {
        let setup = run_setup(1, 2);
        assert_eq!(setup.state.phase, Phase::Play);
    }

    #[test]
    fn current_player_is_seat_zero() {
        let setup = run_setup(1, 3);
        assert_eq!(setup.state.current_player, 0);
    }

    #[test]
    fn event_resolution_stack_empty_after_setup() {
        let setup = run_setup(1, 4);
        assert!(setup.state.event_resolution_stack.is_empty());
    }

    #[test]
    fn wreckage_deck_empty_after_setup() {
        let setup = run_setup(1, 2);
        assert!(
            setup.state.wreckage_deck.is_empty(),
            "wreckage deck should be fully distributed"
        );
    }

    #[test]
    fn all_player_inventories_start_zero() {
        let setup = run_setup(1, 4);
        for p in &setup.state.players {
            assert_eq!(p.inventory, [0; 5]);
            // Unit 22 seeds food with `STARTING_FOOD_COUNTER` so
            // Random-vs-Random self-play doesn't collapse via starvation
            // the turn any player card is placed. Asserting positive
            // reserves here pins the current tuning so a silent drop
            // back to zero is caught.
            assert!(p.food_counter > 0, "starting food counter");
            assert!(p.played_players.is_empty());
        }
    }

    #[test]
    fn each_raft_starts_at_length_two() {
        let setup = run_setup(1, 4);
        for p in &setup.state.players {
            assert_eq!(p.raft.length(), 2);
            assert_eq!(p.raft.invention_count(), 0);
        }
    }
}

// ---------- Setup events ------------------------------------------------

mod setup_events {
    use super::*;

    #[test]
    fn event_stream_matches_expected_counts_and_order() {
        let n: u8 = 3;
        let setup = run_setup(5, n);
        let nu = usize::from(n);
        let face_up_total: usize =
            setup.state.face_up_pools.iter().map(Vec::len).sum();

        // Expected: N DealPlayerCard, N DealWreckageHand, face_up_total
        // DealWreckageFaceUp, in that order.
        let expected_len = nu + nu + face_up_total;
        assert_eq!(setup.events.len(), expected_len);

        // First N events: DealPlayerCard for seats 0..n in order.
        for (seat, ev) in setup.events[0..nu].iter().enumerate() {
            match ev {
                Event::DealPlayerCard { player, .. } => {
                    assert_eq!(usize::from(*player), seat, "DealPlayerCard seat order");
                }
                other => panic!("expected DealPlayerCard at index {seat}, got {other:?}"),
            }
        }

        // Next N events: DealWreckageHand for seats 0..n in order.
        for (seat, ev) in setup.events[nu..2 * nu].iter().enumerate() {
            match ev {
                Event::DealWreckageHand { player, cards } => {
                    assert_eq!(usize::from(*player), seat, "DealWreckageHand seat order");
                    assert_eq!(cards.len(), WRECKAGE_HAND_SIZE);
                }
                other => panic!("expected DealWreckageHand at index {seat}, got {other:?}"),
            }
        }

        // Remaining events are DealWreckageFaceUp.
        for ev in &setup.events[2 * nu..] {
            assert!(
                matches!(ev, Event::DealWreckageFaceUp { .. }),
                "expected DealWreckageFaceUp, got {ev:?}"
            );
        }
    }

    #[test]
    fn deal_wreckage_hand_event_has_exactly_six_cards_and_matches_hand_tail() {
        let n: u8 = 2;
        let setup = run_setup(17, n);
        let nu = usize::from(n);
        // For each seat, the DealWreckageHand.cards should exactly
        // match the last 6 cards of the player's hand (the setup
        // order is "dealt player card first, then 6 wreckage cards").
        for seat in 0..nu {
            let ev = &setup.events[nu + seat];
            let cards = match ev {
                Event::DealWreckageHand { cards, .. } => cards,
                other => panic!("expected DealWreckageHand at {nu}+{seat}, got {other:?}"),
            };
            assert_eq!(
                cards.len(),
                WRECKAGE_HAND_SIZE,
                "DealWreckageHand must carry 6 cards"
            );
            let hand = &setup.state.players[seat].hand;
            assert_eq!(hand.len(), 1 + WRECKAGE_HAND_SIZE);
            let hand_tail: Vec<Card> = hand[1..].to_vec();
            assert_eq!(
                &hand_tail, cards,
                "seat {seat}: hand tail matches event record"
            );
        }
    }

    #[test]
    fn equipment_card_count_in_pool_matches_constructor() {
        // Unit-20 invariant double-check: ensure we're not silently
        // missing equipment cards during setup distribution.
        let setup = run_setup(3, 4);
        let mut all: Vec<Card> = Vec::new();
        for p in &setup.state.players {
            all.extend(p.hand.iter().copied());
        }
        for pool in &setup.state.face_up_pools {
            all.extend(pool.iter().copied());
        }
        let equip_count = all
            .iter()
            .filter(|c| matches!(c, Card::Equipment(_)))
            .count();
        assert_eq!(equip_count, all_equipment().len());
    }
}
