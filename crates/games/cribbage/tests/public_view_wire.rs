//! Wire-shape tests for `CribbageGame::PublicView` (Unit 2 of the
//! Phase 3 stdio-protocol / LLM-agent plan).
//!
//! These tests pin the JSON shape the stdio protocol and `LlmAgent`
//! will ship: every field serializes with its struct name (so snake_case
//! `pegging_stack`, `running_total`, etc.), the `Phase` enum uses
//! `snake_case` variant tags, and a full round-trip through
//! `serde_json` is lossless.

use playtest_cribbage::{Board, Card, Hand, Phase, PublicView, Rank, Suit};

fn sample_card(rank: Rank, suit: Suit) -> Card {
    Card::new(rank, suit)
}

/// Build a discard-phase view with an empty pegging stack and a plain
/// board. Baseline for the happy-path round-trip test.
fn discard_view() -> PublicView {
    PublicView {
        player: 0,
        own_hand: Hand::new(vec![
            sample_card(Rank::Ace, Suit::Clubs),
            sample_card(Rank::Five, Suit::Hearts),
            sample_card(Rank::Ten, Suit::Diamonds),
            sample_card(Rank::Jack, Suit::Spades),
            sample_card(Rank::Queen, Suit::Hearts),
            sample_card(Rank::King, Suit::Clubs),
        ]),
        crib_size: 0,
        starter: None,
        pegging_stack: Vec::new(),
        running_total: 0,
        board: Board::new(),
        phase: Phase::Discard,
        to_act: 1,
    }
}

/// Pegging-phase view: non-empty stack, a cut starter, advanced board.
/// Exercises the edge case called out in the Unit 2 test scenarios.
fn pegging_view() -> PublicView {
    let mut board = Board::new();
    board.advance(0, 7);
    board.advance(1, 4);
    PublicView {
        player: 1,
        own_hand: Hand::new(vec![
            sample_card(Rank::Seven, Suit::Clubs),
            sample_card(Rank::Eight, Suit::Diamonds),
            sample_card(Rank::Nine, Suit::Hearts),
        ]),
        crib_size: 4,
        starter: Some(sample_card(Rank::Five, Suit::Spades)),
        pegging_stack: vec![
            sample_card(Rank::Four, Suit::Hearts),
            sample_card(Rank::Three, Suit::Clubs),
            sample_card(Rank::Two, Suit::Spades),
        ],
        running_total: 9,
        board,
        phase: Phase::Pegging,
        to_act: 1,
    }
}

#[test]
fn public_view_round_trips_through_json() {
    let view = discard_view();
    let json = serde_json::to_string(&view).expect("serialize");
    let back: PublicView = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(view, back);
}

#[test]
fn pegging_phase_public_view_round_trips_with_non_empty_stack() {
    let view = pegging_view();
    let json = serde_json::to_string(&view).expect("serialize");
    let back: PublicView = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(view, back);
    // Sanity: the edge-case payload survives the round-trip.
    assert_eq!(back.pegging_stack.len(), 3);
    assert_eq!(back.running_total, 9);
    assert_eq!(back.starter, Some(sample_card(Rank::Five, Suit::Spades)));
}

#[test]
fn public_view_json_uses_snake_case_field_names() {
    let view = pegging_view();
    let value: serde_json::Value = serde_json::to_value(&view).expect("to_value");
    let obj = value.as_object().expect("PublicView is a JSON object");

    // Every field on PublicView should appear under its Rust snake_case
    // name. This locks the wire shape so the Python stdio client and
    // the LLM prompt template can depend on these keys.
    for key in [
        "player",
        "own_hand",
        "crib_size",
        "starter",
        "pegging_stack",
        "running_total",
        "board",
        "phase",
        "to_act",
    ] {
        assert!(obj.contains_key(key), "missing field {key} in {value}");
    }

    // `Phase` uses `#[serde(rename_all = "snake_case")]`, so the
    // pegging variant serializes as the string "pegging". Spot-check
    // that enum tagging is present and snake_case.
    assert_eq!(obj.get("phase"), Some(&serde_json::Value::from("pegging")));
}

#[test]
fn discard_phase_serializes_phase_as_snake_case_string() {
    let view = discard_view();
    let value: serde_json::Value = serde_json::to_value(&view).expect("to_value");
    assert_eq!(
        value.get("phase"),
        Some(&serde_json::Value::from("discard"))
    );
    // Starter is `None` → JSON `null` (serde default for `Option`).
    assert_eq!(value.get("starter"), Some(&serde_json::Value::Null));
}
