//! Replay: reconstruct every tick's state from a log file.
//!
//! The caller provides the `Game` impl and the `Config` used to start
//! the original game. Replay validates the header (schema + config
//! hash) then folds every event into the initial state via
//! `Game::apply_event`, capturing a snapshot after each event.
//!
//! Snapshots are `G::State: Clone`-bound only for the full-snapshot
//! form. Callers that only want to verify the log is valid can use
//! [`replay_final`] instead, which does not clone.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use playtest_core::{Game, GameResult};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::header::{LogHeader, SCHEMA_VERSION, compute_config_hash};
use crate::reader::{LogReader, ReadError};
use crate::record::LogRecord;

/// Errors produced by [`replay`] / [`replay_final`].
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("opening log {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("reading log: {0}")]
    Read(#[from] ReadError),

    #[error("log is missing its header line")]
    MissingHeader,

    #[error(
        "schema mismatch: log was written against version {actual} but replay only understands version {expected}"
    )]
    SchemaMismatch { expected: u32, actual: u32 },

    #[error(
        "config mismatch: log config_hash {actual} does not match the config passed to replay ({expected})"
    )]
    ConfigMismatch { expected: String, actual: String },

    #[error("game name mismatch: log was written for `{actual}` but replay is for `{expected}`")]
    GameMismatch { expected: String, actual: String },

    #[error("event appeared before header")]
    EventBeforeHeader,

    #[error("tick {expected} expected, got {actual}")]
    TickMismatch { expected: u64, actual: u64 },

    #[error("config hash computation failed: {0}")]
    ConfigHash(#[from] serde_json::Error),
}

/// The outcome of a successful full replay.
#[derive(Debug)]
pub struct Replay<G: Game> {
    pub header: LogHeader,
    /// One entry per event: the state *after* that event was applied.
    /// Index `N` in this vec is the state at tick `N+1`.
    pub snapshots: Vec<G::State>,
    /// The final state at the end of replay.
    pub final_state: G::State,
    /// The `Final` record, if present. `None` when the log was cut off
    /// (e.g., crash mid-play) — the final snapshot is still valid up
    /// to the last recorded event.
    pub result: Option<GameResult>,
}

/// Replay a log file, capturing every intermediate state.
///
/// Requires `G::State: Clone`. For huge games where cloning every
/// state is wasteful, use [`replay_final`] instead.
///
/// # Errors
/// See [`ReplayError`].
pub fn replay<G>(
    game: &G,
    game_name: &str,
    cfg: &G::Config,
    log_path: &Path,
) -> Result<Replay<G>, ReplayError>
where
    G: Game,
    G::State: Clone,
    G::Event: DeserializeOwned,
    G::Config: Serialize,
{
    let mut r = Runner::<G>::open(game, game_name, cfg, log_path)?;
    let mut snapshots = Vec::new();
    while let Some(record) = r.reader.next() {
        let record = record?;
        match record {
            LogRecord::Header(_) => {
                // A second header is treated as malformed — bail rather than
                // silently accepting duplicate headers.
                return Err(ReplayError::EventBeforeHeader);
            }
            LogRecord::Event { tick, payload } => {
                r.apply_event(tick, &payload)?;
                snapshots.push(r.state.clone());
            }
            LogRecord::Final {
                winner,
                reason,
                scores,
                ..
            } => {
                r.result = Some(GameResult {
                    winner,
                    reason,
                    scores,
                });
                break;
            }
        }
    }
    Ok(Replay {
        header: r.header,
        final_state: r.state,
        result: r.result,
        snapshots,
    })
}

/// Replay a log for validation only, returning the final state and
/// result without cloning intermediate snapshots.
///
/// # Errors
/// See [`ReplayError`].
pub fn replay_final<G>(
    game: &G,
    game_name: &str,
    cfg: &G::Config,
    log_path: &Path,
) -> Result<(LogHeader, G::State, Option<GameResult>), ReplayError>
where
    G: Game,
    G::Event: DeserializeOwned,
    G::Config: Serialize,
{
    let mut r = Runner::<G>::open(game, game_name, cfg, log_path)?;
    while let Some(record) = r.reader.next() {
        let record = record?;
        match record {
            LogRecord::Header(_) => return Err(ReplayError::EventBeforeHeader),
            LogRecord::Event { tick, payload } => {
                r.apply_event(tick, &payload)?;
            }
            LogRecord::Final {
                winner,
                reason,
                scores,
                ..
            } => {
                r.result = Some(GameResult {
                    winner,
                    reason,
                    scores,
                });
                break;
            }
        }
    }
    Ok((r.header, r.state, r.result))
}

/// Shared setup + per-event state-folding logic for the two replay
/// flavors above. Not exposed — the public API is `replay` and
/// `replay_final`.
struct Runner<'a, G: Game> {
    game: &'a G,
    state: G::State,
    header: LogHeader,
    reader: LogReader<G::Event, BufReader<File>>,
    next_tick: u64,
    result: Option<GameResult>,
}

impl<'a, G> Runner<'a, G>
where
    G: Game,
    G::Event: DeserializeOwned,
    G::Config: Serialize,
{
    fn open(
        game: &'a G,
        game_name: &str,
        cfg: &G::Config,
        log_path: &Path,
    ) -> Result<Self, ReplayError> {
        let file = File::open(log_path).map_err(|source| ReplayError::Open {
            path: log_path.to_path_buf(),
            source,
        })?;
        let mut reader = LogReader::<G::Event, _>::new(BufReader::new(file));

        let header = match reader.next() {
            Some(Ok(LogRecord::Header(h))) => h,
            Some(Ok(_)) => return Err(ReplayError::EventBeforeHeader),
            Some(Err(e)) => return Err(ReplayError::Read(e)),
            None => return Err(ReplayError::MissingHeader),
        };

        if header.schema != SCHEMA_VERSION {
            return Err(ReplayError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                actual: header.schema,
            });
        }
        if header.game != game_name {
            return Err(ReplayError::GameMismatch {
                expected: game_name.to_owned(),
                actual: header.game.clone(),
            });
        }
        let expected_hash = compute_config_hash(cfg)?;
        if header.config_hash != expected_hash {
            return Err(ReplayError::ConfigMismatch {
                expected: expected_hash,
                actual: header.config_hash.clone(),
            });
        }

        let state = game.initial_state(header.seed, cfg);
        Ok(Self {
            game,
            state,
            header,
            reader,
            next_tick: 0,
            result: None,
        })
    }

    fn apply_event(&mut self, tick: u64, payload: &G::Event) -> Result<(), ReplayError> {
        if tick != self.next_tick {
            return Err(ReplayError::TickMismatch {
                expected: self.next_tick,
                actual: tick,
            });
        }
        self.game.apply_event(&mut self.state, payload);
        self.next_tick += 1;
        Ok(())
    }
}
