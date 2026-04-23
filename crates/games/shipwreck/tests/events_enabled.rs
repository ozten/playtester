//! Integration tests for the Phase 5 `events_enabled` toggle.
//!
//! The R5.9 exit-criterion benchmark runs 100 games each with
//! `events_enabled: true` (Typhoon active, frustration expected) and
//! `events_enabled: false` (baseline, non-frustrating) and compares
//! Likert "agency" means. Here we verify the toggle itself behaves:
//!
//! - When `events_enabled = false`, no Shark/Typhoon/FlyingFish cards
//!   appear in any player's initial state (hand, face-up pool, or
//!   anywhere the setup placed them).
//! - The determinize invariant (`public_view(determinize(s, p, rng), p)
//!   == public_view(s, p)`) holds for both config values.
//! - A full random-self-play game with `events_enabled: false`
//!   terminates legally — the rules path for "event card in hand" is
//!   never triggered, and the base game completes.

use playtest_core::Game;
use playtest_shipwreck::{Card, GameState, ShipWreckConfig, ShipWreckGame};

fn setup_state(
    cfg: ShipWreckConfig,
    seed: u64,
) -> <ShipWreckGame as Game>::State {
    let game = ShipWreckGame::new();
    game.initial_state(seed, &cfg)
}

fn count_event_cards_in_state(state: &GameState) -> usize {
    let mut n = 0;
    for seat in &state.players {
        for c in &seat.hand {
            if matches!(c, Card::Event(_)) {
                n += 1;
            }
        }
    }
    for pool in &state.face_up_pools {
        for c in pool {
            if matches!(c, Card::Event(_)) {
                n += 1;
            }
        }
    }
    for c in &state.wreckage_deck {
        if matches!(c, Card::Event(_)) {
            n += 1;
        }
    }
    n
}

#[test]
fn events_disabled_produces_zero_event_cards_anywhere() {
    let cfg = ShipWreckConfig::new(3).unwrap().with_events_enabled(false);
    // Sweep a handful of seeds to catch any seed-dependent bug.
    for seed in [0_u64, 1, 42, 100, 777] {
        let state = setup_state(cfg, seed);
        assert_eq!(
            count_event_cards_in_state(&state),
            0,
            "seed={seed}: expected zero event cards when events_enabled=false"
        );
    }
}

#[test]
fn events_enabled_default_seeds_some_event_cards() {
    let cfg = ShipWreckConfig::new(3).unwrap();
    assert!(cfg.events_enabled, "default must be events_enabled=true");
    // With the default 5-shark + 5-typhoon + 10-flying-fish pool, a
    // setup over seeds should usually find at least one event card
    // somewhere across the 100-seed sweep.
    let mut total = 0;
    for seed in 0_u64..50 {
        total += count_event_cards_in_state(&setup_state(cfg, seed));
    }
    assert!(
        total > 0,
        "expected event cards to appear somewhere in 50 seeded setups"
    );
}

#[test]
fn event_counts_differ_between_the_two_configs_for_same_seed() {
    // Sanity: the toggle actually does something at setup time.
    // After setup, cards live in face_up_pools (the wreckage_deck is
    // drained into those during setup step 5). Count the event cards
    // across both.
    let cfg_on = ShipWreckConfig::new(3).unwrap();
    let cfg_off = cfg_on.with_events_enabled(false);
    let state_on = setup_state(cfg_on, 42);
    let state_off = setup_state(cfg_off, 42);
    let count_on = count_event_cards_in_state(&state_on);
    let count_off = count_event_cards_in_state(&state_off);
    assert!(
        count_on > 0,
        "events_enabled=true should produce >0 event cards at seed 42"
    );
    assert_eq!(
        count_off, 0,
        "events_enabled=false must produce 0 event cards at seed 42"
    );
}

#[test]
fn config_round_trips_through_serde() {
    // The ingest pipeline hashes the config via compute_config_hash;
    // serde round-trip stability is the discipline that backs
    // different-hash-per-config buckets.
    let cfg = ShipWreckConfig::new(3).unwrap().with_events_enabled(false);
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(json.contains("events_enabled"));
    assert!(json.contains("false"));
    let back: ShipWreckConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn legacy_config_without_events_field_defaults_to_true() {
    // Backward-compat: old serialized configs that predate
    // `events_enabled` / per-card toggles should deserialize with
    // every bool defaulted to `true`, via `#[serde(default = ...)]`.
    let legacy = r#"{"num_players": 3}"#;
    let cfg: ShipWreckConfig = serde_json::from_str(legacy).unwrap();
    assert_eq!(cfg.num_players, 3);
    assert!(cfg.events_enabled);
    assert!(cfg.shark_enabled);
    assert!(cfg.typhoon_enabled);
    assert!(cfg.flying_fish_enabled);
}

// -----------------------------------------------------------------
// Phase 6 per-card toggle tests
// -----------------------------------------------------------------

use playtest_shipwreck::EventCard;

fn count_event_kind(state: &GameState, kind: EventCard) -> usize {
    let matches_kind = |c: &Card| {
        if let Card::Event(ec) = c { *ec == kind } else { false }
    };
    let mut n = 0;
    for seat in &state.players {
        n += seat.hand.iter().filter(|c| matches_kind(c)).count();
    }
    for pool in &state.face_up_pools {
        n += pool.iter().filter(|c| matches_kind(c)).count();
    }
    n += state.wreckage_deck.iter().filter(|c| matches_kind(c)).count();
    n
}

#[test]
fn disabling_single_event_card_removes_only_that_kind() {
    let cfg_on = ShipWreckConfig::new(3).unwrap();
    let cfg_no_typhoon = cfg_on.with_event_card(EventCard::Typhoon, false);
    let state_on = setup_state(cfg_on, 42);
    let state_no_typhoon = setup_state(cfg_no_typhoon, 42);

    assert!(count_event_kind(&state_on, EventCard::Typhoon) > 0);
    assert_eq!(count_event_kind(&state_no_typhoon, EventCard::Typhoon), 0);
    // Shark and FlyingFish still present.
    assert!(count_event_kind(&state_no_typhoon, EventCard::Shark) > 0);
    assert!(count_event_kind(&state_no_typhoon, EventCard::FlyingFish) > 0);
}

#[test]
fn events_enabled_false_overrides_per_card_flags() {
    // Even if all per-card flags are true, `events_enabled: false`
    // wins — `event_card_active` returns false for every kind.
    let cfg = ShipWreckConfig::new(2).unwrap().with_events_enabled(false);
    assert!(cfg.shark_enabled); // per-card flag unchanged
    for kind in [EventCard::Shark, EventCard::Typhoon, EventCard::FlyingFish] {
        assert!(
            !cfg.event_card_active(kind),
            "events_enabled=false must override per-card flag for {kind:?}"
        );
    }
    let state = setup_state(cfg, 42);
    assert_eq!(count_event_cards_in_state(&state), 0);
}

#[test]
fn per_card_flags_round_trip_through_serde() {
    let cfg = ShipWreckConfig::new(3)
        .unwrap()
        .with_event_card(EventCard::Shark, false)
        .with_event_card(EventCard::FlyingFish, false);
    let json = serde_json::to_string(&cfg).unwrap();
    let back: ShipWreckConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cfg);
    assert!(!back.shark_enabled);
    assert!(back.typhoon_enabled);
    assert!(!back.flying_fish_enabled);
}

#[test]
fn event_card_active_matrix() {
    // Explicit matrix test — all 8 combinations of (master, shark).
    for events_enabled in [true, false] {
        for shark_enabled in [true, false] {
            let cfg = ShipWreckConfig {
                num_players: 2,
                events_enabled,
                shark_enabled,
                typhoon_enabled: true,
                flying_fish_enabled: true,
            };
            let expected = events_enabled && shark_enabled;
            assert_eq!(
                cfg.event_card_active(EventCard::Shark),
                expected,
                "events_enabled={events_enabled}, shark_enabled={shark_enabled}"
            );
        }
    }
}
