//! Fixture tests for [`ShipWreckMetrics`].
//!
//! Each test synthesizes a compact event stream that targets exactly
//! one metric property (winner on rescue points, rescue-point tie
//! broken by raft length, 4-way tie, event-card counts, per-player
//! starvation). Keeps the fixtures readable and decoupled from the
//! full game engine.

use playtest_core::{EndReason, GameResult};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    GameLog, MetricRegistry, MetricValue, MetricValueKind, validate_values_against_defs,
};
use playtest_shipwreck::{
    Event, PlayerScore, ShipWreckGame, ShipWreckMetrics, TieBreakerUsed,
    action::EventCardKind,
    action::EventTarget,
    raft::SlotId,
};
use uuid::Uuid;

fn header(agents: Vec<String>) -> LogHeader {
    LogHeader {
        schema: SCHEMA_VERSION,
        game: ShipWreckGame::NAME.into(),
        version: "0.0.0".into(),
        seed: 0,
        agents,
        started_at: 0,
        config_hash: "0".repeat(64),
    }
}

fn make_log(agents: Vec<String>, events: Vec<Event>, result: Option<GameResult>) -> GameLog<ShipWreckGame> {
    let mut records: Vec<Result<LogRecord<Event>, playtest_log::ReadError>> =
        vec![Ok(LogRecord::Header(header(agents)))];
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
    GameLog::<ShipWreckGame>::from_records(records).expect("well-formed synthetic log")
}

fn find<'a>(values: &'a [MetricValue], name: &str, player: Option<u8>) -> Option<&'a MetricValue> {
    values
        .iter()
        .find(|v| v.metric_name == name && v.player == player)
}

fn score(p: u8, rp: u16, raft: u16, inv: u16) -> PlayerScore {
    PlayerScore {
        player: p,
        rescue_points: rp,
        raft_length: raft,
        invention_count: inv,
    }
}

#[test]
fn winner_on_rescue_points_emits_all_game_shape_metrics() {
    // P0 wins 10-5 on rescue points alone — tie_breaker = "none".
    let scores = vec![score(0, 10, 3, 1), score(1, 5, 2, 0)];
    let events = vec![
        Event::EndTurn { player: 0 },
        Event::EndTurn { player: 1 },
        Event::EndTurn { player: 0 },
        Event::EventCardPlayed {
            player: 0,
            card: EventCardKind::Typhoon,
            target: EventTarget::None,
        },
        Event::EndGame {
            winner: Some(0),
            reason: EndReason::Other("deck_exhausted".into()),
            final_scores: scores.clone(),
            tie_breaker: TieBreakerUsed::None,
        },
    ];
    let log = make_log(
        vec!["random".into(), "random".into()],
        events,
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Other("deck_exhausted".into()),
            scores: scores.iter().map(|s| i32::from(s.rescue_points)).collect(),
        }),
    );

    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);
    validate_values_against_defs(&ShipWreckMetrics.metric_definitions(), &values)
        .expect("values must validate against defs");

    // game_length_turns = 3
    assert_eq!(
        find(&values, "game_length_turns", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(3)),
    );
    // winner_raft_length = 3 (winner = P0)
    assert_eq!(
        find(&values, "winner_raft_length", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(3)),
    );
    // winner_equipment_count = 1
    assert_eq!(
        find(&values, "winner_equipment_count", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(1)),
    );
    // tie_breaker = "none"
    assert_eq!(
        find(&values, "tie_breaker_used", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Tag("none".into())),
    );
    // event_cards_played = 1
    assert_eq!(
        find(&values, "event_cards_played", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(1)),
    );

    // Per-player rescue points.
    assert_eq!(
        find(&values, "player_rescue_points", Some(0)).map(|v| v.value.clone()),
        Some(MetricValueKind::Scalar(10.0)),
    );
    assert_eq!(
        find(&values, "player_rescue_points", Some(1)).map(|v| v.value.clone()),
        Some(MetricValueKind::Scalar(5.0)),
    );
}

#[test]
fn four_way_true_tie_omits_winner_metrics_and_flags_tie() {
    let scores = vec![
        score(0, 4, 2, 0),
        score(1, 4, 2, 0),
        score(2, 4, 2, 0),
        score(3, 4, 2, 0),
    ];
    let events = vec![
        Event::EndTurn { player: 0 },
        Event::EndGame {
            winner: None,
            reason: EndReason::Draw,
            final_scores: scores.clone(),
            tie_breaker: TieBreakerUsed::Tie,
        },
    ];
    let log = make_log(
        vec!["random".into(); 4],
        events,
        Some(GameResult {
            winner: None,
            reason: EndReason::Draw,
            scores: scores.iter().map(|s| i32::from(s.rescue_points)).collect(),
        }),
    );
    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);

    assert_eq!(
        find(&values, "tie_breaker_used", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Tag("tie".into())),
    );
    assert!(
        find(&values, "winner_raft_length", None).is_none(),
        "winner_raft_length must be absent on a tie"
    );
    assert!(
        find(&values, "winner_equipment_count", None).is_none(),
        "winner_equipment_count must be absent on a tie"
    );
}

#[test]
fn rescue_tie_broken_by_raft_length() {
    // Both seats 7 rescue points, P0 has longer raft.
    let scores = vec![score(0, 7, 3, 0), score(1, 7, 2, 0)];
    let events = vec![
        Event::EndGame {
            winner: Some(0),
            reason: EndReason::Other("deck_exhausted".into()),
            final_scores: scores.clone(),
            tie_breaker: TieBreakerUsed::RaftLength,
        },
    ];
    let log = make_log(
        vec!["random".into(), "random".into()],
        events,
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Other("deck_exhausted".into()),
            scores: scores.iter().map(|s| i32::from(s.rescue_points)).collect(),
        }),
    );

    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);
    assert_eq!(
        find(&values, "tie_breaker_used", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Tag("raft_length".into())),
    );
    // winner raft length is 3
    assert_eq!(
        find(&values, "winner_raft_length", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(3)),
    );
}

#[test]
fn event_cards_played_counts_played_not_resolved() {
    // 3 EventCardPlayed events + 2 EventResolved events — extractor
    // must report 3, not 5.
    use playtest_shipwreck::EventOutcome;
    let scores = vec![score(0, 5, 2, 0), score(1, 3, 2, 0)];
    let events = vec![
        Event::EventCardPlayed {
            player: 0,
            card: EventCardKind::FlyingFish,
            target: EventTarget::None,
        },
        Event::EventResolved {
            player: 0,
            outcome: EventOutcome::FlyingFishGranted { player: 0 },
        },
        Event::EventCardPlayed {
            player: 1,
            card: EventCardKind::Typhoon,
            target: EventTarget::None,
        },
        Event::EventResolved {
            player: 1,
            outcome: EventOutcome::TyphoonPass { player: 1 },
        },
        Event::EventCardPlayed {
            player: 0,
            card: EventCardKind::Shark,
            target: EventTarget::SingleSlot {
                player: 1,
                slot: SlotId::BaseLeft,
            },
        },
        Event::EndGame {
            winner: Some(0),
            reason: EndReason::Other("deck_exhausted".into()),
            final_scores: scores.clone(),
            tie_breaker: TieBreakerUsed::None,
        },
    ];
    let log = make_log(
        vec!["random".into(), "random".into()],
        events,
        None,
    );
    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);
    assert_eq!(
        find(&values, "event_cards_played", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Count(3)),
    );
}

#[test]
fn food_starvation_events_count_per_player() {
    // P0 starves twice, P1 starves once — extractor must attribute
    // to the right seat.
    let scores = vec![score(0, 3, 2, 0), score(1, 6, 2, 0)];
    let events = vec![
        Event::FoodConsumed {
            player: 0,
            slot: SlotId::BaseLeft,
            amount: 1,
            starved: true,
        },
        Event::FoodConsumed {
            player: 1,
            slot: SlotId::BaseRight,
            amount: 1,
            starved: false,
        },
        Event::FoodConsumed {
            player: 0,
            slot: SlotId::BaseRight,
            amount: 1,
            starved: true,
        },
        Event::FoodConsumed {
            player: 1,
            slot: SlotId::BaseLeft,
            amount: 1,
            starved: true,
        },
        Event::EndGame {
            winner: Some(1),
            reason: EndReason::Other("deck_exhausted".into()),
            final_scores: scores.clone(),
            tie_breaker: TieBreakerUsed::None,
        },
    ];
    let log = make_log(
        vec!["random".into(), "random".into()],
        events,
        None,
    );
    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);

    assert_eq!(
        find(&values, "player_food_starvation_events", Some(0))
            .map(|v| v.value.clone()),
        Some(MetricValueKind::Count(2)),
    );
    assert_eq!(
        find(&values, "player_food_starvation_events", Some(1))
            .map(|v| v.value.clone()),
        Some(MetricValueKind::Count(1)),
    );
}

#[test]
fn empty_game_produces_no_winner_metrics() {
    // Log with only an EndGame but no winner — everything winner-scoped
    // absent, game-scoped emits "tie".
    let events = vec![Event::EndGame {
        winner: None,
        reason: EndReason::Draw,
        final_scores: vec![],
        tie_breaker: TieBreakerUsed::Tie,
    }];
    let log = make_log(
        vec!["random".into(), "random".into()],
        events,
        Some(GameResult {
            winner: None,
            reason: EndReason::Draw,
            scores: vec![],
        }),
    );
    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);

    assert!(find(&values, "winner_raft_length", None).is_none());
    assert_eq!(
        find(&values, "tie_breaker_used", None).map(|v| v.value.clone()),
        Some(MetricValueKind::Tag("tie".into())),
    );
}

#[test]
fn metrics_round_trip_through_real_game_log() {
    // Run one real random-vs-random game through the engine, then run
    // metric extraction on the resulting log. Not an assertion of
    // specific values — just a "does the pipeline survive a real log?"
    // smoke.
    use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
    use playtest_agents::RandomAgent;
    use playtest_core::{Agent, Game, GameLoop};
    use playtest_log::{EventLogWriter, LogReader, compute_config_hash};
    use playtest_shipwreck::ShipWreckConfig;

    let seed = 42u64;
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let state = game.initial_state(seed, &cfg);
    let mut loop_ = GameLoop::new(&game, state);
    let mut rng = ProductionRng::from_seed(seed);
    let mut sink = StubGameEventSink::new();

    // Write a proper header through EventLogWriter so the sink lines
    // match the JSONL schema LogReader expects.
    {
        let hdr = LogHeader {
            schema: SCHEMA_VERSION,
            game: ShipWreckGame::NAME.into(),
            version: "0.0.0".into(),
            seed,
            agents: vec!["random".into(), "random".into()],
            started_at: 0,
            config_hash: compute_config_hash(&cfg).expect("config hash"),
        };
        let mut writer: EventLogWriter<Event> = EventLogWriter::new(&mut sink);
        writer.write_header(&hdr).expect("write header");
    }

    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = vec![
        Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(1))),
        Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(2))),
    ];

    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt")
        .block_on(loop_.run(agents.as_mut_slice(), &mut rng, &mut sink))
        .expect("game runs");

    // Emit a Final record so the ingestor treats this as a complete
    // game log.
    let final_line = serde_json::to_string(&LogRecord::<Event>::Final {
        winner: result.winner,
        reason: result.reason,
        scores: result.scores,
        finished_at: 0,
    })
    .expect("final serializes");
    {
        use playtest_ports::GameEventSink;
        sink.emit(&final_line).expect("emit final");
    }

    // Concatenate the sink lines into a buffer and parse with LogReader.
    let joined = sink.lines().join("\n");
    let reader = LogReader::<Event, _>::new(std::io::Cursor::new(joined.as_bytes()));
    let log = GameLog::<ShipWreckGame>::from_records(reader).expect("log parses");

    let values = ShipWreckMetrics.extract(Uuid::nil(), &log);
    validate_values_against_defs(&ShipWreckMetrics.metric_definitions(), &values)
        .expect("real-game values must validate against defs");

    // We should have at least *some* values emitted.
    assert!(
        !values.is_empty(),
        "real game should emit at least one metric value"
    );
}
