//! `GameLog<G>` — a fully-loaded event log, ready for metric extraction.
//!
//! The log format is JSONL (defined in `playtest-log`); this wrapper
//! reads a whole log into memory and splits it into header / events /
//! final-result for registries to walk. For huge logs this is heavier
//! than the streaming [`playtest_log::LogReader`], but metrics at
//! Phase 1 scale fit comfortably — a 10K-event Cribbage game is
//! ~1 MB of JSONL.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use playtest_core::{Game, GameResult};
use playtest_log::{LogHeader, LogReader, LogRecord, ReadError};
use playtest_ports::UnixMillis;
use serde::de::DeserializeOwned;

/// Errors raised while loading a [`GameLog`].
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("opening log {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("reading log: {0}")]
    Read(#[from] ReadError),

    #[error("log has no header line")]
    MissingHeader,

    #[error("log has more than one header line")]
    DuplicateHeader,

    #[error("log has more than one final record")]
    DuplicateFinal,
}

/// Fully-loaded log for one game. Extracted metrics are derived from
/// the header, the event stream, and the optional final record.
#[derive(Debug, Clone)]
pub struct GameLog<G: Game> {
    pub header: LogHeader,
    pub events: Vec<G::Event>,
    /// `None` when the log was cut off before the final record —
    /// useful for post-mortem on crashed games, but most registries
    /// will refuse to score a truncated game.
    pub final_result: Option<GameResult>,
    /// Wall-clock time the game ended, in Unix epoch ms. Lifted off the
    /// `Final` record (schema v2+); `None` when the final record is
    /// absent or when a v1 log — which had no `finished_at` — parsed
    /// through the `#[serde(default)]` path and came back as `0`.
    pub finished_at: Option<UnixMillis>,
}

impl<G: Game> GameLog<G>
where
    G::Event: DeserializeOwned,
{
    /// Read a full event log from disk.
    ///
    /// # Errors
    /// See [`LoadError`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let file = std::fs::File::open(path).map_err(|source| LoadError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = LogReader::<G::Event, _>::new(BufReader::new(file));
        Self::from_records(reader)
    }

    /// Build a `GameLog` from any iterator of parsed records. Handy
    /// for tests that want to synthesize a log without touching disk.
    ///
    /// # Errors
    /// Returns [`LoadError::MissingHeader`], [`LoadError::DuplicateHeader`],
    /// [`LoadError::DuplicateFinal`], or [`LoadError::Read`] if the
    /// stream is malformed.
    pub fn from_records<I>(records: I) -> Result<Self, LoadError>
    where
        I: IntoIterator<Item = Result<LogRecord<G::Event>, ReadError>>,
    {
        let mut header: Option<LogHeader> = None;
        let mut events: Vec<G::Event> = Vec::new();
        let mut final_result: Option<GameResult> = None;
        let mut finished_at: Option<UnixMillis> = None;

        for rec in records {
            match rec? {
                LogRecord::Header(h) => {
                    if header.is_some() {
                        return Err(LoadError::DuplicateHeader);
                    }
                    header = Some(h);
                }
                LogRecord::Event { payload, .. } => events.push(payload),
                LogRecord::Final {
                    winner,
                    reason,
                    scores,
                    finished_at: fa,
                } => {
                    if final_result.is_some() {
                        return Err(LoadError::DuplicateFinal);
                    }
                    final_result = Some(GameResult {
                        winner,
                        reason,
                        scores,
                    });
                    // `0` means "v1 log via serde default" — treat as
                    // absent so metrics can skip wall_clock_ms cleanly.
                    finished_at = if fa == 0 { None } else { Some(fa) };
                }
            }
        }

        Ok(Self {
            header: header.ok_or(LoadError::MissingHeader)?,
            events,
            final_result,
            finished_at,
        })
    }
}
