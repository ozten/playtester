//! Load a directory of JSONL event logs into a SQLite database.
//!
//! Idempotency is the load-bearing property: re-ingesting the same
//! directory rewrites existing rows rather than duplicating them. The
//! mechanism is a deterministic `game_id` derived from the log
//! header's stable fields (`game`, `seed`, `started_at`,
//! `config_hash`) and `INSERT OR REPLACE` on every row. Crash-resume
//! and repeated `playtest report` runs just work.
//!
//! Tolerates malformed files: a parse error on one file is recorded in
//! the [`IngestReport`] and the batch continues. The Unit 15 reporter
//! surfaces the summary so an operator notices.
//!
//! Performance: the whole batch runs inside one transaction with
//! prepared statements and `PRAGMA synchronous=OFF` / `journal_mode=
//! MEMORY`. That's what the Unit 14 risks table calls for to keep the
//! 30s report budget achievable on 10K games.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use playtest_core::Game;
use playtest_log::{LogReader, SCHEMA_VERSION};
use rusqlite::{Connection, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::builtin::BuiltInMetrics;
use crate::log::{GameLog, LoadError};
use crate::registry::MetricRegistry;
use crate::value::{MetricValue, MetricValueKind};

/// Errors raised during ingestion setup (not per-file).
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("reading directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One-per-file failure report. Stored in [`IngestReport::errors`] so
/// the caller can surface every problem rather than bailing on the
/// first bad log.
#[derive(Debug, Clone)]
pub struct FileError {
    pub path: PathBuf,
    pub reason: String,
}

/// Summary of what ingestion did.
#[derive(Debug, Clone, Default)]
pub struct IngestReport {
    /// Number of JSONL files that produced a row in `games`.
    pub games_ingested: u64,
    /// Files skipped because their `schema` did not match [`SCHEMA_VERSION`].
    pub games_skipped_schema_mismatch: u64,
    /// Files skipped because their `game` name did not match the
    /// `Game` impl passed to [`ingest_directory`].
    pub games_skipped_wrong_game: u64,
    /// Total MetricValue rows written to `game_metrics`.
    pub metrics_written: u64,
    /// Total rows written to `agent_stats`.
    pub agent_rows_written: u64,
    /// Per-file failures.
    pub errors: Vec<FileError>,
}

impl IngestReport {
    /// One-line summary for CLI output.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "ingested {} games, {} metric rows, {} agent rows; \
             skipped {} (schema mismatch) + {} (wrong game); errors: {}",
            self.games_ingested,
            self.metrics_written,
            self.agent_rows_written,
            self.games_skipped_schema_mismatch,
            self.games_skipped_wrong_game,
            self.errors.len(),
        )
    }
}

/// Apply the schema to an empty (or existing) database. Safe to call
/// repeatedly — all statements are `IF NOT EXISTS`.
///
/// # Errors
/// Returns [`IngestError::Sqlite`] if the schema statements fail.
pub fn init_schema(conn: &Connection) -> Result<(), IngestError> {
    conn.execute_batch(include_str!("schema.sql"))?;
    Ok(())
}

/// Walk every `*.jsonl` file in `dir` and ingest each.
///
/// `game` identifies the `Game` impl the logs are expected to belong
/// to; files whose header's `game` field doesn't match are skipped.
/// `registry` is the game-specific `MetricRegistry`; `BuiltInMetrics`
/// is always applied automatically on top.
///
/// # Errors
/// Returns [`IngestError`] on directory-walk or transaction-setup
/// failures. Per-file parse errors are accumulated into the returned
/// [`IngestReport`], not bubbled up.
pub fn ingest_directory<G, R>(
    conn: &mut Connection,
    dir: &Path,
    game_name: &str,
    registry: &R,
) -> Result<IngestReport, IngestError>
where
    G: Game,
    G::Event: DeserializeOwned,
    G::Config: Serialize,
    R: MetricRegistry<G>,
{
    init_schema(conn)?;

    // Performance knobs, per the Unit 14 risks table.
    conn.execute_batch(
        "PRAGMA synchronous = OFF; \
         PRAGMA journal_mode = MEMORY; \
         PRAGMA temp_store = MEMORY;",
    )?;

    let mut report = IngestReport::default();
    let entries = std::fs::read_dir(dir).map_err(|source| IngestError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    // Deterministic ingest order so ingest reports are reproducible.
    paths.sort();

    let tx = conn.transaction()?;
    for path in &paths {
        if let Err(e) = ingest_one_file::<G, R>(&tx, path, game_name, registry, &mut report) {
            report.errors.push(FileError {
                path: path.clone(),
                reason: e.to_string(),
            });
        }
    }
    tx.commit()?;

    Ok(report)
}

/// Ingest a single log file inside an open transaction. Caller handles
/// schema-mismatch / wrong-game counting so the top-level loop can
/// keep going after any kind of per-file problem.
fn ingest_one_file<G, R>(
    tx: &Transaction<'_>,
    path: &Path,
    game_name: &str,
    registry: &R,
    report: &mut IngestReport,
) -> Result<(), IngestError>
where
    G: Game,
    G::Event: DeserializeOwned,
    R: MetricRegistry<G>,
{
    let log = match load_log::<G>(path) {
        Ok(l) => l,
        Err(LoadError::MissingHeader | LoadError::DuplicateHeader | LoadError::DuplicateFinal) => {
            report.errors.push(FileError {
                path: path.to_path_buf(),
                reason: "malformed log (header problem)".into(),
            });
            return Ok(());
        }
        Err(LoadError::Read(e)) => {
            report.errors.push(FileError {
                path: path.to_path_buf(),
                reason: format!("malformed JSON: {e}"),
            });
            return Ok(());
        }
        Err(LoadError::Open { source, .. }) => {
            report.errors.push(FileError {
                path: path.to_path_buf(),
                reason: format!("open failed: {source}"),
            });
            return Ok(());
        }
    };

    if log.header.schema != SCHEMA_VERSION {
        report.games_skipped_schema_mismatch += 1;
        return Ok(());
    }
    if log.header.game != game_name {
        report.games_skipped_wrong_game += 1;
        return Ok(());
    }

    let game_id = derive_game_id(&log.header);

    upsert_game_row(tx, game_id, &log, path)?;
    let agent_rows = upsert_agent_stats(tx, game_id, &log)?;
    report.agent_rows_written += agent_rows;

    // Delete any pre-existing metric rows for this game so re-ingest is
    // idempotent even when the registry changes between runs.
    tx.execute(
        "DELETE FROM game_metrics WHERE game_id = ?1",
        params![game_id.to_string()],
    )?;

    let mut values = BuiltInMetrics.extract(game_id, &log);
    values.extend(registry.extract(game_id, &log));
    let written = insert_metric_rows(tx, &values)?;

    report.games_ingested += 1;
    report.metrics_written += written;
    Ok(())
}

fn load_log<G>(path: &Path) -> Result<GameLog<G>, LoadError>
where
    G: Game,
    G::Event: DeserializeOwned,
{
    let file = File::open(path).map_err(|source| LoadError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = LogReader::<G::Event, _>::new(BufReader::new(file));
    GameLog::<G>::from_records(reader)
}

/// Compute a stable UUID for the game from log-header fields.
///
/// SHA-256 of `game|seed|started_at|config_hash`, truncated to 16 bytes.
/// Same inputs → same UUID, which is what makes re-ingest idempotent.
/// Collisions within realistic usage are vanishingly unlikely — SHA-256
/// with even a 128-bit truncation is well beyond the birthday bound.
fn derive_game_id(header: &playtest_log::LogHeader) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(header.game.as_bytes());
    hasher.update(b"|");
    hasher.update(header.seed.to_le_bytes());
    hasher.update(b"|");
    hasher.update(header.started_at.to_le_bytes());
    hasher.update(b"|");
    hasher.update(header.config_hash.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

fn upsert_game_row<G: Game>(
    tx: &Transaction<'_>,
    game_id: Uuid,
    log: &GameLog<G>,
    path: &Path,
) -> Result<(), IngestError> {
    let (winner, end_reason) = match &log.final_result {
        Some(r) => {
            let reason = crate::builtin::reason_tag(&r.reason);
            let winner = r.winner.map(i64::from);
            (winner, reason)
        }
        None => (None, "unfinished".to_owned()),
    };
    let finished_at: Option<i64> = log.finished_at.and_then(|v| i64::try_from(v).ok());
    let event_count = i64::try_from(log.events.len()).unwrap_or(i64::MAX);

    tx.execute(
        "INSERT OR REPLACE INTO games \
         (id, game, version, seed, started_at, finished_at, winner, end_reason, config_hash, event_count, source_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            game_id.to_string(),
            log.header.game,
            log.header.version,
            i64::try_from(log.header.seed).unwrap_or(i64::MAX),
            i64::try_from(log.header.started_at).unwrap_or(i64::MAX),
            finished_at,
            winner,
            end_reason,
            log.header.config_hash,
            event_count,
            path.to_string_lossy().into_owned(),
        ],
    )?;
    Ok(())
}

fn upsert_agent_stats<G: Game>(
    tx: &Transaction<'_>,
    game_id: Uuid,
    log: &GameLog<G>,
) -> Result<u64, IngestError> {
    let mut written = 0u64;
    let winner = log.final_result.as_ref().and_then(|r| r.winner);
    let scores = log
        .final_result
        .as_ref()
        .map_or(&[][..], |r| r.scores.as_slice());

    for (idx, agent_name) in log.header.agents.iter().enumerate() {
        let player = i64::try_from(idx).unwrap_or(i64::MAX);
        let won = i64::from(u8::from(winner == u8::try_from(idx).ok()));
        let score = scores.get(idx).copied().map_or(0i64, i64::from);
        tx.execute(
            "INSERT OR REPLACE INTO agent_stats (game_id, player, agent_name, won, score) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![game_id.to_string(), player, agent_name, won, score],
        )?;
        written += 1;
    }
    Ok(written)
}

fn insert_metric_rows(tx: &Transaction<'_>, values: &[MetricValue]) -> Result<u64, IngestError> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO game_metrics \
         (game_id, metric_name, player, tag, value_kind, value_numeric, value_text) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut written = 0u64;
    for v in values {
        let (kind_str, num, text): (&str, Option<f64>, Option<&str>) = match &v.value {
            MetricValueKind::Scalar(f) => ("scalar", Some(*f), None),
            #[allow(
                clippy::cast_precision_loss,
                reason = "metric counts store fine in f64 for any Phase 1 magnitude"
            )]
            MetricValueKind::Count(n) => ("count", Some(*n as f64), None),
            MetricValueKind::Tag(s) => ("tag", None, Some(s.as_str())),
            MetricValueKind::Bool(b) => ("bool", Some(f64::from(u8::from(*b))), None),
        };
        // -1 is the sentinel for game-scoped metrics (see schema.sql).
        let player: i64 = v.player.map_or(-1, i64::from);
        let tag = v.tag.as_deref().unwrap_or("");
        stmt.execute(params![
            v.game_id.to_string(),
            v.metric_name,
            player,
            tag,
            kind_str,
            num,
            text,
        ])?;
        written += 1;
    }
    Ok(written)
}
