//! Phase 6 compare module.
//!
//! Given two independent `Connection` handles — one per log dir
//! ingested into an in-memory SQLite — this module produces the
//! paired sample vectors that [`crate::stats`] runs tests over, and
//! the [`engine`] submodule ties those samples into significance-
//! corrected findings. Each query binds to exactly one connection;
//! there are no cross-DB SQL joins.
//!
//! Three sample families populate the compare report:
//!
//! - **Numeric game metrics** — one sample per game per
//!   `(metric_name, player, tag)` tuple. Welch's t-test consumes
//!   these.
//! - **Per-agent outcomes** — `(wins, games)` pairs keyed by agent
//!   name. Two-proportion z-test consumes these.
//! - **Phase 5 critique signals** — per-question Likert samples
//!   (Welch) and coded-tag frequency counts (z-test).

pub mod engine;

pub use engine::{
    CompareOpts, CompareResult, Correction, CritiqueAvailability, Finding, FindingKind,
    run_compare,
};

use rusqlite::{Connection, Error as SqliteError, params};

use crate::query::{AgentSummary, agent_summaries};

/// Normalized identifier for a numeric metric row. `player = None`
/// encodes the game-scoped sentinel (`-1` in SQLite); `tag = None`
/// encodes the untagged sentinel (empty string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MetricKey {
    pub name: String,
    pub player: Option<u8>,
    pub tag: Option<String>,
}

impl MetricKey {
    /// Build a [`MetricKey`] from a database row's raw values.
    #[must_use]
    pub fn from_row(name: String, player_sentinel: i64, tag_sentinel: String) -> Self {
        let player = if player_sentinel < 0 {
            None
        } else {
            u8::try_from(player_sentinel).ok()
        };
        let tag = if tag_sentinel.is_empty() {
            None
        } else {
            Some(tag_sentinel)
        };
        Self { name, player, tag }
    }

    /// Convert back to the sentinel shape used in SQL parameter binds.
    #[must_use]
    pub fn to_sentinels(&self) -> (String, i64, String) {
        let player_val = self.player.map_or(-1_i64, i64::from);
        let tag_val = self.tag.clone().unwrap_or_default();
        (self.name.clone(), player_val, tag_val)
    }

    /// Short display label for the report — "name", "name@player",
    /// "name:tag", or "name@player:tag".
    #[must_use]
    pub fn label(&self) -> String {
        use core::fmt::Write as _;
        let mut s = self.name.clone();
        if let Some(p) = self.player {
            let _ = write!(s, "@p{p}");
        }
        if let Some(tag) = &self.tag {
            s.push(':');
            s.push_str(tag);
        }
        s
    }
}

/// The triage result of enumerating metrics across two DBs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PairedMetrics {
    /// Metrics present in both baseline and variant.
    pub paired: Vec<MetricKey>,
    /// Metrics present in baseline but absent from variant.
    pub only_baseline: Vec<MetricKey>,
    /// Metrics present in variant but absent from baseline.
    pub only_variant: Vec<MetricKey>,
}

/// Enumerate every distinct `(metric_name, player, tag)` seen in each
/// DB's `game_metrics`, sort into paired / only-one-side buckets.
///
/// # Errors
/// Propagates `rusqlite::Error` from the underlying queries.
pub fn enumerate_paired_metrics(
    baseline: &Connection,
    variant: &Connection,
) -> Result<PairedMetrics, SqliteError> {
    let a = distinct_metric_keys(baseline)?;
    let b = distinct_metric_keys(variant)?;
    let a_set: std::collections::BTreeSet<&MetricKey> = a.iter().collect();
    let b_set: std::collections::BTreeSet<&MetricKey> = b.iter().collect();

    let paired: Vec<MetricKey> = a.iter().filter(|k| b_set.contains(*k)).cloned().collect();
    let only_baseline: Vec<MetricKey> = a
        .iter()
        .filter(|k| !b_set.contains(*k))
        .cloned()
        .collect();
    let only_variant: Vec<MetricKey> = b.iter().filter(|k| !a_set.contains(*k)).cloned().collect();

    Ok(PairedMetrics {
        paired,
        only_baseline,
        only_variant,
    })
}

fn distinct_metric_keys(conn: &Connection) -> Result<Vec<MetricKey>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT metric_name, player, tag FROM game_metrics \
         WHERE value_kind IN ('scalar', 'count', 'bool') \
         ORDER BY metric_name, player, tag",
    )?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let player: i64 = row.get(1)?;
        let tag: String = row.get(2)?;
        Ok(MetricKey::from_row(name, player, tag))
    })?;
    rows.collect()
}

/// Pull every per-game numeric sample for `key`. One row per matching
/// `game_metrics` row; output order is `ORDER BY game_id` for
/// determinism.
///
/// Returns an empty vector if the key isn't present in `conn`.
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_numeric_samples(
    conn: &Connection,
    key: &MetricKey,
) -> Result<Vec<f64>, SqliteError> {
    let (name, player_val, tag_val) = key.to_sentinels();
    let mut stmt = conn.prepare(
        "SELECT value_numeric FROM game_metrics \
         WHERE metric_name = ?1 AND player = ?2 AND tag = ?3 \
               AND value_kind IN ('scalar', 'count', 'bool') \
               AND value_numeric IS NOT NULL \
         ORDER BY game_id",
    )?;
    let rows = stmt.query_map(params![name, player_val, tag_val], |row| {
        row.get::<_, f64>(0)
    })?;
    rows.collect()
}

/// `(agent_name, wins, games)` per agent. Wraps [`agent_summaries`]
/// with a flattening that discards `avg_score` (compare doesn't use
/// it — that's reporter-side, this side is sample-vector-land).
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_agent_outcomes(conn: &Connection) -> Result<Vec<(String, u64, u64)>, SqliteError> {
    let summaries: Vec<AgentSummary> = agent_summaries(conn)?;
    Ok(summaries
        .into_iter()
        .map(|s| (s.agent_name, s.wins, s.games_played))
        .collect())
}

/// Pull every Likert score for `question` across all critiqued games
/// in `conn`. One element per `(game_id, seat)` pair that answered.
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_likert_samples(
    conn: &Connection,
    question: &str,
) -> Result<Vec<f64>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT score FROM critique_likert \
         WHERE question = ?1 \
         ORDER BY game_id, seat",
    )?;
    let rows = stmt.query_map(params![question], |row| {
        let s: i64 = row.get(0)?;
        #[allow(clippy::cast_precision_loss)]
        Ok(s as f64)
    })?;
    rows.collect()
}

/// Distinct Likert question names present in the DB. Used by the
/// compare engine to enumerate which questions to Welch-test.
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_likert_questions(conn: &Connection) -> Result<Vec<String>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT question FROM critique_likert ORDER BY question",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Total number of `(game_id, seat)` pairs that produced a critique
/// response — the denominator for two-proportion z-tests on coded
/// tags. Approximated as `COUNT(DISTINCT game_id || '|' || seat)`
/// from `critique_likert` (every critiqued seat emits at least one
/// Likert row by spec).
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_total_critique_responses(conn: &Connection) -> Result<u64, SqliteError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM \
            (SELECT DISTINCT game_id, seat FROM critique_likert)",
        [],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Tag-count row: total mentions of a given tag across the entire DB.
/// The denominator for two-proportion z-tests is
/// `fetch_total_critique_responses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CritiqueTagTotal {
    pub tag: String,
    pub count: u64,
}

/// Every distinct tag + its overall mention count, sorted by tag
/// ascending.
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_tag_totals(conn: &Connection) -> Result<Vec<CritiqueTagTotal>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT tag, COUNT(*) FROM critique_tags \
         GROUP BY tag ORDER BY tag",
    )?;
    let rows = stmt.query_map([], |row| {
        let tag: String = row.get(0)?;
        let n_i: i64 = row.get(1)?;
        Ok(CritiqueTagTotal {
            tag,
            count: u64::try_from(n_i).unwrap_or(0),
        })
    })?;
    rows.collect()
}

/// Total number of games ingested into the DB — the n for the
/// report's sample-size line.
///
/// # Errors
/// Propagates `rusqlite::Error`.
pub fn fetch_games_count(conn: &Connection) -> Result<u64, SqliteError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}
