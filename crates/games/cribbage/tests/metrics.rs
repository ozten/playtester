//! Integration tests for [`CribbageMetrics`].
//!
//! Each test synthesizes a compact event stream that exhibits exactly
//! one fixture property (crib-win, pegging-win, mid-show non-dealer
//! win, nibs-on-cut, multiple lead changes) and asserts the metric
//! extractor produces the expected values. Synthesizing the stream in
//! code — rather than committing pre-baked JSONL — keeps the fixtures
//! readable, pins the intent of each scenario, and keeps the test
//! suite robust against unrelated event-schema tweaks.

use playtest_core::{EndReason, GameResult};
use playtest_cribbage::metrics::{game_shape, per_card, scoring as scoring_metrics};
use playtest_cribbage::scoring::ShowScore;
use playtest_cribbage::{Card, CribbageGame, CribbageMetrics, Event, PegReason, Rank, Suit};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    GameLog, MetricRegistry, MetricValue, MetricValueKind, validate_values_against_defs,
};
use uuid::Uuid;

fn header() -> LogHeader {
    LogHeader {
        schema: SCHEMA_VERSION,
        game: CribbageGame::NAME.into(),
        version: "0.0.0".into(),
        seed: 0,
        agents: vec!["random".into(), "random".into()],
        started_at: 0,
        config_hash: "0".repeat(64),
    }
}

fn c(r: Rank, s: Suit) -> Card {
    Card::new(r, s)
}

fn make_log(events: Vec<Event>, result: Option<GameResult>) -> GameLog<CribbageGame> {
    let mut records: Vec<Result<LogRecord<Event>, playtest_log::ReadError>> =
        vec![Ok(LogRecord::Header(header()))];
    for (tick, payload) in events.into_iter().enumerate() {
        records.push(Ok(LogRecord::Event {
            tick: tick as u64,
            payload,
        }));
    }
    if let Some(r) = result {
        records.push(Ok(LogRecord::Final {
            winner: r.winner,
            reason: r.reason,
            scores: r.scores,
            finished_at: 0,
        }));
    }
    GameLog::<CribbageGame>::from_records(records).expect("fixture log builds")
}

fn show_score_total(total: u8) -> ShowScore {
    ShowScore {
        fifteens: 0,
        pairs: 0,
        runs: 0,
        flush: 0,
        nobs: 0,
        total,
    }
}

/// Deal six arbitrary cards to each player, ensuring distinct ranks so
/// the kept/discard logic has unambiguous targets. Returns the 12
/// DealCard events in alternating order — non-dealer first, matching
/// the actual game loop. Use by fixtures that want a valid deal prefix
/// but don't care about the specific cards beyond the ranks tested.
fn deal_events(p0: [(Rank, Suit); 6], p1: [(Rank, Suit); 6], non_dealer: u8) -> Vec<Event> {
    let mut out = Vec::with_capacity(12);
    for i in 0..6 {
        out.push(Event::DealCard {
            player: non_dealer,
            card: c(p0[i].0, p0[i].1),
        });
        out.push(Event::DealCard {
            player: 1 - non_dealer,
            card: c(p1[i].0, p1[i].1),
        });
    }
    out
}

fn find<'a>(vs: &'a [MetricValue], name: &str, player: Option<u8>) -> &'a MetricValue {
    vs.iter()
        .find(|v| v.metric_name == name && v.player == player && v.tag.is_none())
        .unwrap_or_else(|| panic!("missing metric {name} for player {player:?}"))
}

fn find_count(vs: &[MetricValue], name: &str, player: Option<u8>) -> i64 {
    match find(vs, name, player).value {
        MetricValueKind::Count(n) => n,
        ref other => panic!("{name}: expected Count, got {other:?}"),
    }
}

fn find_tagged_count(vs: &[MetricValue], name: &str, player: Option<u8>, tag: &str) -> i64 {
    let mv = vs
        .iter()
        .find(|v| v.metric_name == name && v.player == player && v.tag.as_deref() == Some(tag))
        .unwrap_or_else(|| panic!("missing metric {name} player={player:?} tag={tag:?}"));
    match mv.value {
        MetricValueKind::Count(n) => n,
        ref other => panic!("{name}[tag={tag}]: expected Count, got {other:?}"),
    }
}

// ---------- Scenarios ---------------------------------------------------

/// (a) Crib-win game: dealer closes the game out while counting the
/// crib at showdown. End phase should be tagged as `show` (cribbage's
/// crib count happens inside the Show phase in the live state machine)
/// and the final score margin should be non-zero.
#[test]
fn fixture_a_crib_win_end_phase_and_winner_flags() {
    // Dealer is 0 — the live state machine starts with dealer=0 on
    // initial_state.
    let dealer: u8 = 0;
    let non_dealer: u8 = 1 - dealer;
    let mut events = deal_events(
        // Non-dealer dealt: 2..7 (so no jacks, no nibs)
        [
            (Rank::Two, Suit::Clubs),
            (Rank::Three, Suit::Clubs),
            (Rank::Four, Suit::Clubs),
            (Rank::Five, Suit::Clubs),
            (Rank::Six, Suit::Clubs),
            (Rank::Seven, Suit::Clubs),
        ],
        // Dealer dealt: 8..K (six distinct ranks, no jacks)
        [
            (Rank::Eight, Suit::Diamonds),
            (Rank::Nine, Suit::Diamonds),
            (Rank::Ten, Suit::Diamonds),
            (Rank::Queen, Suit::Diamonds),
            (Rank::King, Suit::Diamonds),
            (Rank::Ace, Suit::Diamonds),
        ],
        non_dealer,
    );
    // Discards: each player sends their two lowest ranks to the crib.
    events.push(Event::DiscardToCrib {
        player: non_dealer,
        cards: [c(Rank::Two, Suit::Clubs), c(Rank::Three, Suit::Clubs)],
    });
    events.push(Event::DiscardToCrib {
        player: dealer,
        cards: [c(Rank::Ace, Suit::Diamonds), c(Rank::Eight, Suit::Diamonds)],
    });
    events.push(Event::CutStarter {
        card: c(Rank::Four, Suit::Hearts),
    });
    // Non-dealer pegs modestly; dealer pegs a bit.
    events.push(Event::PegScored {
        player: non_dealer,
        reason: PegReason::Fifteen,
        points: 2,
    });
    events.push(Event::PegScored {
        player: dealer,
        reason: PegReason::Pair,
        points: 2,
    });
    events.push(Event::PeggingComplete);
    // Show: non-dealer hand (8), dealer hand (4), dealer crib (12) —
    // 8 + 4 + 12 = 24 dealer-side, 8 non-dealer-side. Combined with the
    // 2+2 pegging, dealer ends at 18, non-dealer at 10. We want the
    // crib count to flip the lead — so score pre-crib: ND=10, D=6,
    // then crib scores 12 to push D to 18. Let's wire those exact
    // totals.
    events.push(Event::ShowScored {
        player: non_dealer,
        is_crib: false,
        score: show_score_total(8),
    });
    events.push(Event::ShowScored {
        player: dealer,
        is_crib: false,
        score: show_score_total(4),
    });
    events.push(Event::ShowScored {
        player: dealer,
        is_crib: true,
        score: show_score_total(12),
    });
    events.push(Event::EndGame {
        winner: dealer,
        reason: EndReason::Victory,
    });

    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(dealer),
            reason: EndReason::Victory,
            scores: vec![0, 0],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);

    // End-phase tag: the live state machine ends the game inside Show
    // when the crib count crosses 121; our synthetic log's last phase-
    // updating event was the crib ShowScored, so the phase snapshot is
    // `Show`.
    match &find(&values, game_shape::GAME_ENDED_IN_PHASE, None).value {
        MetricValueKind::Tag(t) => assert_eq!(t, "show"),
        other => panic!("expected Tag, got {other:?}"),
    }

    // Winner was the dealer.
    match &find(&values, game_shape::GAME_WINNER_WAS_DEALER, None).value {
        MetricValueKind::Bool(b) => assert!(*b, "dealer should have won this fixture"),
        other => panic!("expected Bool, got {other:?}"),
    }

    // Margin is strictly positive (dealer ended ahead).
    let margin = find_count(&values, game_shape::FINAL_SCORE_MARGIN, None);
    assert!(margin >= 1, "expected a positive margin, got {margin}");
}

/// (b) Pegging-win game: the pin crosses 121 mid-pegging, so the end
/// phase is tagged `pegging` and no ShowScored event ever fires.
#[test]
fn fixture_b_pegging_win_phase_is_pegging() {
    let dealer: u8 = 0;
    let non_dealer: u8 = 1;
    let mut events = deal_events(
        [
            (Rank::Two, Suit::Clubs),
            (Rank::Three, Suit::Clubs),
            (Rank::Four, Suit::Clubs),
            (Rank::Five, Suit::Clubs),
            (Rank::Six, Suit::Clubs),
            (Rank::Seven, Suit::Clubs),
        ],
        [
            (Rank::Eight, Suit::Diamonds),
            (Rank::Nine, Suit::Diamonds),
            (Rank::Ten, Suit::Diamonds),
            (Rank::Queen, Suit::Diamonds),
            (Rank::King, Suit::Diamonds),
            (Rank::Ace, Suit::Diamonds),
        ],
        non_dealer,
    );
    events.push(Event::DiscardToCrib {
        player: non_dealer,
        cards: [c(Rank::Two, Suit::Clubs), c(Rank::Three, Suit::Clubs)],
    });
    events.push(Event::DiscardToCrib {
        player: dealer,
        cards: [c(Rank::Ace, Suit::Diamonds), c(Rank::Eight, Suit::Diamonds)],
    });
    events.push(Event::CutStarter {
        card: c(Rank::Four, Suit::Hearts),
    });
    // Cascade of pegging events that finally closes out the game.
    events.push(Event::PegPlayed {
        player: non_dealer,
        card: c(Rank::Seven, Suit::Clubs),
        running_total: 7,
    });
    events.push(Event::PegPlayed {
        player: dealer,
        card: c(Rank::Nine, Suit::Diamonds),
        running_total: 16,
    });
    events.push(Event::PegScored {
        player: non_dealer,
        reason: PegReason::Run(3),
        points: 3,
    });
    events.push(Event::EndGame {
        winner: non_dealer,
        reason: EndReason::Victory,
    });

    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(non_dealer),
            reason: EndReason::Victory,
            scores: vec![0, 3],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);

    match &find(&values, game_shape::GAME_ENDED_IN_PHASE, None).value {
        MetricValueKind::Tag(t) => assert_eq!(t, "pegging"),
        other => panic!("expected Tag, got {other:?}"),
    }

    // Scoring breakdown: non-dealer gets the pegging points, neither
    // player recorded any show points.
    assert_eq!(
        find_count(
            &values,
            scoring_metrics::PEGGING_SCORE_TOTAL,
            Some(non_dealer)
        ),
        3
    );
    assert_eq!(
        find_count(&values, scoring_metrics::HAND_SCORE_TOTAL, Some(non_dealer)),
        0
    );
    assert_eq!(
        find_count(&values, scoring_metrics::CRIB_SCORE_TOTAL, Some(dealer)),
        0
    );
}

/// (c) Mid-show win by non-dealer: the non-dealer pegs out while
/// counting their hand, so the dealer never counts their own hand or
/// crib. Dealer's `HAND_SCORE_TOTAL` and `CRIB_SCORE_TOTAL` stay at 0.
#[test]
fn fixture_c_mid_show_nondealer_win_dealer_scores_stay_zero() {
    let dealer: u8 = 0;
    let non_dealer: u8 = 1;
    let mut events = deal_events(
        [
            (Rank::Five, Suit::Clubs),
            (Rank::Five, Suit::Diamonds),
            (Rank::Five, Suit::Hearts),
            (Rank::Five, Suit::Spades),
            (Rank::Jack, Suit::Clubs),
            (Rank::Two, Suit::Clubs),
        ],
        [
            (Rank::Two, Suit::Diamonds),
            (Rank::Three, Suit::Diamonds),
            (Rank::Four, Suit::Diamonds),
            (Rank::Six, Suit::Diamonds),
            (Rank::Seven, Suit::Diamonds),
            (Rank::Eight, Suit::Diamonds),
        ],
        non_dealer,
    );
    events.push(Event::DiscardToCrib {
        player: non_dealer,
        cards: [c(Rank::Two, Suit::Clubs), c(Rank::Jack, Suit::Clubs)],
    });
    events.push(Event::DiscardToCrib {
        player: dealer,
        cards: [c(Rank::Two, Suit::Diamonds), c(Rank::Three, Suit::Diamonds)],
    });
    events.push(Event::CutStarter {
        card: c(Rank::Four, Suit::Hearts),
    });
    // Pegging: enough points distributed that both players are close to 121.
    events.push(Event::PegScored {
        player: non_dealer,
        reason: PegReason::Fifteen,
        points: 2,
    });
    events.push(Event::PegScored {
        player: dealer,
        reason: PegReason::Fifteen,
        points: 2,
    });
    events.push(Event::PeggingComplete);
    // Non-dealer counts their hand and pegs out — the big 29-hand
    // equivalent of 24 points. Dealer never gets to count.
    events.push(Event::ShowScored {
        player: non_dealer,
        is_crib: false,
        score: show_score_total(24),
    });
    events.push(Event::EndGame {
        winner: non_dealer,
        reason: EndReason::Victory,
    });

    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(non_dealer),
            reason: EndReason::Victory,
            scores: vec![2, 26],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);

    // Dealer never counted — their hand/crib scores stayed at 0.
    assert_eq!(
        find_count(&values, scoring_metrics::HAND_SCORE_TOTAL, Some(dealer)),
        0
    );
    assert_eq!(
        find_count(&values, scoring_metrics::CRIB_SCORE_TOTAL, Some(dealer)),
        0
    );
    // Non-dealer's hand score is the big show total.
    assert_eq!(
        find_count(&values, scoring_metrics::HAND_SCORE_TOTAL, Some(non_dealer)),
        24
    );
}

/// (d) Nibs on cut: starter is a jack, dealer gets +2 immediately.
/// Per-game `CUTS_PRODUCING_NIBS == 1` and `NIBS_CONTRIBUTION` for the
/// dealer is 2.
#[test]
fn fixture_d_nibs_on_cut_awards_dealer_two() {
    let dealer: u8 = 0;
    let non_dealer: u8 = 1;
    let mut events = deal_events(
        [
            (Rank::Two, Suit::Clubs),
            (Rank::Three, Suit::Clubs),
            (Rank::Four, Suit::Clubs),
            (Rank::Five, Suit::Clubs),
            (Rank::Six, Suit::Clubs),
            (Rank::Seven, Suit::Clubs),
        ],
        [
            (Rank::Eight, Suit::Diamonds),
            (Rank::Nine, Suit::Diamonds),
            (Rank::Ten, Suit::Diamonds),
            (Rank::Queen, Suit::Diamonds),
            (Rank::King, Suit::Diamonds),
            (Rank::Ace, Suit::Diamonds),
        ],
        non_dealer,
    );
    events.push(Event::DiscardToCrib {
        player: non_dealer,
        cards: [c(Rank::Two, Suit::Clubs), c(Rank::Three, Suit::Clubs)],
    });
    events.push(Event::DiscardToCrib {
        player: dealer,
        cards: [c(Rank::Ace, Suit::Diamonds), c(Rank::Eight, Suit::Diamonds)],
    });
    events.push(Event::CutStarter {
        card: c(Rank::Jack, Suit::Hearts),
    });
    events.push(Event::NibsScored {
        player: dealer,
        points: 2,
    });
    events.push(Event::EndGame {
        winner: dealer,
        reason: EndReason::Victory,
    });

    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(dealer),
            reason: EndReason::Victory,
            scores: vec![2, 0],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);

    assert_eq!(
        find_count(&values, game_shape::CUTS_PRODUCING_NIBS, None),
        1
    );
    assert_eq!(
        find_count(&values, scoring_metrics::NIBS_CONTRIBUTION, Some(dealer)),
        2
    );
    assert_eq!(
        find_count(
            &values,
            scoring_metrics::NIBS_CONTRIBUTION,
            Some(non_dealer)
        ),
        0
    );
}

/// (e) Multiple lead changes: cook up a game with a back-and-forth.
#[test]
fn fixture_e_multiple_lead_changes() {
    let dealer: u8 = 0;
    let non_dealer: u8 = 1;
    // Skip the deal events — lead changes only depend on scoring events.
    let events = vec![
        // P0 takes lead
        Event::PegScored {
            player: dealer,
            reason: PegReason::Fifteen,
            points: 5,
        },
        // P1 takes lead (flip 1)
        Event::PegScored {
            player: non_dealer,
            reason: PegReason::Run(3),
            points: 10,
        },
        // P0 takes lead back (flip 2)
        Event::PegScored {
            player: dealer,
            reason: PegReason::Pair,
            points: 10,
        },
        // P1 takes lead again (flip 3)
        Event::PegScored {
            player: non_dealer,
            reason: PegReason::Triple,
            points: 10,
        },
        Event::EndGame {
            winner: non_dealer,
            reason: EndReason::Victory,
        },
    ];
    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(non_dealer),
            reason: EndReason::Victory,
            scores: vec![15, 20],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);
    let flips = find_count(&values, game_shape::LEAD_CHANGES, None);
    assert!(flips >= 3, "expected >= 3 lead changes, got {flips}");
}

// ---------- Per-card (R1.5) -------------------------------------------

/// Per-card metrics emit a count for every (rank × player × metric)
/// triple. Here we synthesize a hand where a single card's fate is
/// unambiguous and assert each expected count.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "linear fixture: synthetic event stream + focused assertions on specific (rank, player) cells"
)]
fn per_card_counts_track_dealt_kept_discarded_and_winner_correlates() {
    let dealer: u8 = 0;
    let non_dealer: u8 = 1;
    let mut events = deal_events(
        // P0 (non-dealer here): has a five and a king among dealt cards.
        [
            (Rank::Five, Suit::Clubs),
            (Rank::King, Suit::Clubs),
            (Rank::Two, Suit::Clubs),
            (Rank::Three, Suit::Clubs),
            (Rank::Four, Suit::Clubs),
            (Rank::Six, Suit::Clubs),
        ],
        // P1 (dealer): has two queens and sends one to their own crib.
        [
            (Rank::Queen, Suit::Diamonds),
            (Rank::Queen, Suit::Hearts),
            (Rank::Seven, Suit::Diamonds),
            (Rank::Eight, Suit::Diamonds),
            (Rank::Nine, Suit::Diamonds),
            (Rank::Ten, Suit::Diamonds),
        ],
        non_dealer,
    );
    events.push(Event::DiscardToCrib {
        player: non_dealer,
        // P1 (non-dealer=1): sends 2♣ and 3♣ to dealer's crib.
        // Wait — `non_dealer` is `1` per `deal_events`'s convention
        // ("non-dealer first"). Actually deal_events puts `p0`'s slice
        // under `non_dealer` and `p1`'s under `dealer`. So the array
        // labeled "P0 (non-dealer here)" above really is non-dealer (1).
        cards: [c(Rank::Two, Suit::Clubs), c(Rank::Three, Suit::Clubs)],
    });
    events.push(Event::DiscardToCrib {
        player: dealer,
        cards: [
            c(Rank::Queen, Suit::Diamonds),
            c(Rank::Seven, Suit::Diamonds),
        ],
    });
    events.push(Event::CutStarter {
        card: c(Rank::Four, Suit::Hearts),
    });
    events.push(Event::ShowScored {
        player: non_dealer,
        is_crib: false,
        score: show_score_total(6),
    });
    events.push(Event::EndGame {
        winner: dealer,
        reason: EndReason::Victory,
    });

    let log = make_log(
        events,
        Some(GameResult {
            winner: Some(dealer),
            reason: EndReason::Victory,
            scores: vec![6, 0],
        }),
    );
    let values = CribbageMetrics.extract(Uuid::nil(), &log);

    // Non-dealer was dealt a Five — dealt count is 1, kept count is 1
    // (they kept the Five in their 4-card hand). Per-card values carry
    // the rank symbol as the `tag` on MetricValue.
    assert_eq!(
        find_tagged_count(&values, per_card::CARD_DEALT_COUNT, Some(non_dealer), "5"),
        1
    );
    assert_eq!(
        find_tagged_count(&values, per_card::CARD_KEPT_COUNT, Some(non_dealer), "5"),
        1
    );

    // Non-dealer discarded a 2 to opponent's crib — count is 1.
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::CARD_DISCARDED_TO_OPP_CRIB_COUNT,
            Some(non_dealer),
            "2",
        ),
        1
    );
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::CARD_DISCARDED_TO_OWN_CRIB_COUNT,
            Some(non_dealer),
            "2",
        ),
        0
    );

    // Dealer discarded a Queen (Q = rank 12) and a Seven to their own crib.
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::CARD_DISCARDED_TO_OWN_CRIB_COUNT,
            Some(dealer),
            "Q",
        ),
        1
    );
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::CARD_DISCARDED_TO_OWN_CRIB_COUNT,
            Some(dealer),
            "7",
        ),
        1
    );

    // Dealer won — win-when-card-in-crib for Q should be 1 for dealer,
    // 0 for non-dealer (crib belongs to dealer).
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::WIN_WHEN_CARD_IN_CRIB_COUNT,
            Some(dealer),
            "Q",
        ),
        1
    );
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::WIN_WHEN_CARD_IN_CRIB_COUNT,
            Some(non_dealer),
            "Q",
        ),
        0
    );

    // Non-dealer did NOT win — their WIN_WHEN_CARD_IN_HAND_COUNT for
    // any held rank should be 0 despite having held it.
    assert_eq!(
        find_tagged_count(
            &values,
            per_card::WIN_WHEN_CARD_IN_HAND_COUNT,
            Some(non_dealer),
            "5",
        ),
        0
    );
}

// ---------- Schema consistency ---------------------------------------

/// Every value emitted by the registry must match a declared metric
/// definition — catches forgotten updates when metrics are added.
#[test]
fn emitted_values_validate_against_declared_definitions() {
    // Tiny log: no events, a final record. Every metric still emits
    // (per-card zeros, scoring zeros, etc.), so this exercises the
    // whole definition set.
    let log = make_log(
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 98],
        }),
    );
    let defs = CribbageMetrics.metric_definitions();
    let values = CribbageMetrics.extract(Uuid::nil(), &log);
    validate_values_against_defs(&defs, &values).expect("values match their definitions");
}

/// At least 15 Cribbage-specific metric definitions registered, per the
/// plan's Verification checklist (game-shape + scoring + per-card with
/// multiple ranks easily exceeds this).
#[test]
fn at_least_fifteen_metric_definitions_registered() {
    let defs = CribbageMetrics.metric_definitions();
    assert!(
        defs.len() >= 15,
        "expected >= 15 metric definitions, got {}",
        defs.len()
    );
    // Sanity: definitions have unique names.
    playtest_metrics::validate_definitions(&defs).expect("no duplicate metric names");
}
