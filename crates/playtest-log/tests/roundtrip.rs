//! End-to-end log round-trip tests: writer -> file -> reader -> replay.

use core::ops::Range;
use std::io::{Cursor, Write as _};
use std::path::Path;

use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_core::{Actor, EndReason, Game, GameError, GameResult, PlayerId};
use playtest_log::{
    EventLogWriter, LogHeader, LogReader, LogRecord, ReplayError, SCHEMA_VERSION,
    compute_config_hash, replay,
};
use playtest_ports::{GameEventSink, Rng, RngError};
use serde::{Deserialize, Serialize};
use tempfile::tempdir;

// ---------- Minimal game used across tests ------------------------------

#[derive(Default, Clone, PartialEq, Eq, Debug)]
struct TallyState {
    scores: [u32; 2],
    next_player: PlayerId,
}

#[derive(Clone, PartialEq, Eq)]
struct Add(u8);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Scored {
    player: PlayerId,
    amount: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TallyConfig {
    target: u32,
}

#[derive(Debug)]
struct TallyGame;

impl Game for TallyGame {
    type State = TallyState;
    type Action = Add;
    type Event = Scored;
    type PublicView = TallyState;
    type Config = TallyConfig;

    fn initial_state(&self, _seed: u64, _cfg: &TallyConfig) -> TallyState {
        TallyState::default()
    }
    fn next_actor(&self, s: &TallyState) -> Actor {
        Actor::Player(s.next_player)
    }
    fn legal_actions(&self, _s: &TallyState, _p: PlayerId) -> Vec<Add> {
        vec![Add(1), Add(2), Add(3)]
    }
    fn apply_action(
        &self,
        _s: &TallyState,
        player: PlayerId,
        a: &Add,
    ) -> Result<Vec<Scored>, GameError> {
        Ok(vec![Scored {
            player,
            amount: a.0,
        }])
    }
    fn resolve_chance(&self, _s: &TallyState, _rng: &mut dyn Rng) -> Result<Scored, GameError> {
        unreachable!("TallyGame has no chance events")
    }
    fn apply_event(&self, s: &mut TallyState, e: &Scored) {
        s.scores[e.player as usize] += u32::from(e.amount);
        s.next_player = 1 - e.player;
    }
    fn public_view(&self, s: &TallyState, _p: PlayerId) -> TallyState {
        s.clone()
    }
    fn game_over(&self, s: &TallyState) -> Option<GameResult> {
        let cfg_target: u32 = 10;
        if s.scores[0] >= cfg_target || s.scores[1] >= cfg_target {
            let winner = u8::from(s.scores[1] >= cfg_target);
            Some(GameResult {
                winner: Some(winner),
                reason: EndReason::Victory,
                scores: s
                    .scores
                    .iter()
                    .map(|&v| i32::try_from(v).unwrap_or(i32::MAX))
                    .collect(),
            })
        } else {
            None
        }
    }
}

// ---------- Sinks used by the tests -------------------------------------

struct UnusedRng;
impl Rng for UnusedRng {
    fn next_u64(&mut self) -> u64 {
        unreachable!()
    }
    fn gen_range(&mut self, _range: Range<u64>) -> Result<u64, RngError> {
        unreachable!()
    }
}

fn make_header(cfg: &TallyConfig) -> LogHeader {
    LogHeader {
        schema: SCHEMA_VERSION,
        game: "tally".into(),
        version: "0.0.0".into(),
        seed: 1,
        agents: vec!["scripted".into(), "scripted".into()],
        started_at: 1_700_000_000_000,
        config_hash: compute_config_hash(cfg).unwrap(),
    }
}

fn write_sample_log(path: &Path, num_events: u64, include_final: bool, cfg: &TallyConfig) {
    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, path);
    {
        let mut writer: EventLogWriter<Scored> = EventLogWriter::new(&mut sink);
        writer.write_header(&make_header(cfg)).unwrap();
        for t in 0..num_events {
            let player = u8::try_from(t % 2).unwrap();
            writer
                .write_event(t, &Scored { player, amount: 1 })
                .unwrap();
        }
        if include_final {
            writer
                .finish(&GameResult {
                    winner: None,
                    reason: EndReason::Draw,
                    scores: vec![0, 0],
                })
                .unwrap();
        }
    }
}

// ---------- Scenarios ---------------------------------------------------

#[test]
fn hundred_events_write_read_count_matches() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("hundred.jsonl");
    let cfg = TallyConfig { target: 10 };
    write_sample_log(&path, 100, true, &cfg);

    let file = std::fs::File::open(&path).unwrap();
    let reader = LogReader::<Scored, _>::new(std::io::BufReader::new(file));
    let records: Vec<_> = reader.collect::<Result<_, _>>().unwrap();

    // 1 header + 100 events + 1 final = 102 records.
    assert_eq!(records.len(), 102);
    assert!(matches!(records[0], LogRecord::Header(_)));
    assert!(matches!(records[101], LogRecord::Final { .. }));
    for (i, r) in records.iter().skip(1).take(100).enumerate() {
        match r {
            LogRecord::Event { tick, .. } => {
                assert_eq!(*tick, i as u64, "tick ordering broken at index {i}");
            }
            other => panic!("expected Event at index {i}, got {other:?}"),
        }
    }
}

#[test]
fn zero_event_log_with_header_and_final_is_valid() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("empty.jsonl");
    let cfg = TallyConfig { target: 10 };
    write_sample_log(&path, 0, true, &cfg);

    let file = std::fs::File::open(&path).unwrap();
    let reader = LogReader::<Scored, _>::new(std::io::BufReader::new(file));
    let records: Vec<_> = reader.collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn malformed_line_surfaces_with_accurate_line_number() {
    let mut buf: Vec<u8> = Vec::new();
    let cfg = TallyConfig { target: 10 };
    {
        let header_line =
            serde_json::to_string(&LogRecord::<Scored>::Header(make_header(&cfg))).unwrap();
        writeln!(buf, "{header_line}").unwrap();
    }
    for t in 0..5u64 {
        let rec: LogRecord<Scored> = LogRecord::Event {
            tick: t,
            payload: Scored {
                player: 0,
                amount: 1,
            },
        };
        writeln!(buf, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
    }
    writeln!(buf, "{{not valid json").unwrap(); // line 7
    for t in 5..10u64 {
        let rec: LogRecord<Scored> = LogRecord::Event {
            tick: t,
            payload: Scored {
                player: 1,
                amount: 1,
            },
        };
        writeln!(buf, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
    }

    let mut reader = LogReader::<Scored, _>::new(Cursor::new(buf));
    // 6 valid records first (header + 5 events), then the bad line.
    for _ in 0..6 {
        reader.next().unwrap().unwrap();
    }
    let err = reader.next().unwrap().unwrap_err();
    match err {
        playtest_log::ReadError::Malformed { line, .. } => assert_eq!(line, 7),
        other => panic!("expected Malformed on line 7, got {other:?}"),
    }
}

#[test]
fn schema_mismatch_during_replay_is_surfaced() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("badschema.jsonl");
    // Write a bogus header with schema = 99 by hand so we don't need to
    // fabricate a writer path for this.
    let cfg = TallyConfig { target: 10 };
    let mut bad_header = make_header(&cfg);
    bad_header.schema = 99;
    let line = serde_json::to_string(&LogRecord::<Scored>::Header(bad_header)).unwrap();
    std::fs::write(&path, format!("{line}\n")).unwrap();

    let err = replay::<TallyGame>(&TallyGame, "tally", &cfg, &path).unwrap_err();
    assert!(
        matches!(
            err,
            ReplayError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                actual: 99
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn config_hash_mismatch_during_replay_is_surfaced() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("cfg_mismatch.jsonl");

    // Write a log with config_hash for target=10.
    let written_cfg = TallyConfig { target: 10 };
    write_sample_log(&path, 3, true, &written_cfg);

    // Try to replay with a different config.
    let other_cfg = TallyConfig { target: 20 };
    let err = replay::<TallyGame>(&TallyGame, "tally", &other_cfg, &path).unwrap_err();
    assert!(
        matches!(err, ReplayError::ConfigMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn replay_reconstructs_snapshots_per_event() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snap.jsonl");
    let cfg = TallyConfig { target: 10 };
    write_sample_log(&path, 6, true, &cfg);

    let result = replay::<TallyGame>(&TallyGame, "tally", &cfg, &path).unwrap();
    assert_eq!(result.snapshots.len(), 6);
    assert_eq!(result.header.seed, 1);
    // Each event adds 1 point, alternating players. After 6 events both
    // players have scored 3 times each.
    assert_eq!(result.final_state.scores, [3, 3]);
    assert!(result.result.is_some());
}

#[test]
fn game_loop_log_round_trips_through_replay() {
    use playtest_agents::ScriptedAgent;
    use playtest_core::{Agent, GameLoop};

    let dir = tempdir().unwrap();
    let path = dir.path().join("live.jsonl");
    let cfg = TallyConfig { target: 10 };

    // Live pass: GameLoop writes events through a production sink; we
    // sandwich them with header + final via an EventLogWriter.
    let (live_final_state, live_result) = {
        let fs = ProductionFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, &path);

        // Header first.
        {
            let mut writer: EventLogWriter<Scored> = EventLogWriter::new(&mut sink);
            writer.write_header(&make_header(&cfg)).unwrap();
        }

        // Now drive the loop. The loop's own emit wire shape matches
        // `LogRecord::Event`, so its writes slot in between our header
        // and final without re-serialization.
        let game = TallyGame;
        let mut loop_ = GameLoop::new(&game, game.initial_state(1, &cfg));
        let mut agents: Vec<Box<dyn Agent<TallyGame>>> = vec![
            Box::new(ScriptedAgent::new(|_v: &TallyState, a: &Add| {
                i32::from(a.0)
            })),
            Box::new(ScriptedAgent::new(|_v: &TallyState, a: &Add| {
                i32::from(a.0)
            })),
        ];
        let mut rng = UnusedRng;

        let result = {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            runtime.block_on(async {
                loop_
                    .run(agents.as_mut_slice(), &mut rng, &mut sink)
                    .await
                    .unwrap()
            })
        };
        let final_state = loop_.into_state();

        // Final + flush. `EventLogWriter`'s state machine expects a
        // header before anything else, and we already wrote one earlier
        // through a different writer instance tied to the same sink.
        // Emit the final record directly and flush — this is the
        // pragmatic seam where two writer lifecycles share one sink.
        {
            let final_line = serde_json::to_string(&LogRecord::<Scored>::Final {
                winner: result.winner,
                reason: result.reason.clone(),
                scores: result.scores.clone(),
            })
            .unwrap();
            sink.emit(&final_line).unwrap();
            sink.flush().unwrap();
        }

        (final_state, result)
    };

    // Replay the log and assert we reproduce the same final state.
    let replayed = replay::<TallyGame>(&TallyGame, "tally", &cfg, &path).unwrap();
    assert_eq!(replayed.final_state, live_final_state);
    assert_eq!(replayed.result.unwrap(), live_result);
    assert!(
        !replayed.snapshots.is_empty(),
        "game had at least one event"
    );
}

#[test]
fn log_file_is_eyeball_readable_and_greppable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("grep.jsonl");
    let cfg = TallyConfig { target: 10 };
    write_sample_log(&path, 4, true, &cfg);

    let contents = std::fs::read_to_string(&path).unwrap();
    // Every line parses standalone as JSON (the "one line = one record"
    // contract the Phase 1 metrics pass relies on).
    for line in contents.lines() {
        let _: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
    }
    // `grep '"kind":"event"'` finds exactly 4 lines.
    let event_lines = contents
        .lines()
        .filter(|l| l.contains("\"kind\":\"event\""))
        .count();
    assert_eq!(event_lines, 4);
}
