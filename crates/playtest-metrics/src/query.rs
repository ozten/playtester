//! Canned queries the Unit 15 reporter composes into a markdown report.
//!
//! Kept narrow by design: each function returns a small, report-ready
//! struct rather than a row cursor. That keeps the reporter free of
//! SQL and the SQL here free of presentation logic. Functions that
//! join across `games` + `agent_stats` + `game_metrics` live here;
//! anything more ad-hoc is left to the reporter to build from
//! primitives.

use rusqlite::{Connection, Error as SqliteError, params};

/// Total row count in `games`. Useful for the reporter's header line
/// and for the ingestion-smoke tests.
///
/// # Errors
/// Returns a `rusqlite::Error` if the query fails.
pub fn games_count(conn: &Connection) -> Result<u64, SqliteError> {
    conn.query_row("SELECT COUNT(*) FROM games", [], |row| {
        let n: i64 = row.get(0)?;
        Ok(u64::try_from(n).unwrap_or(0))
    })
}

/// Win-rate / outcome summary, one row per distinct `agent_name`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSummary {
    pub agent_name: String,
    pub games_played: u64,
    pub wins: u64,
    pub avg_score: f64,
}

impl AgentSummary {
    #[must_use]
    pub fn win_rate(&self) -> f64 {
        if self.games_played == 0 {
            0.0
        } else {
            #[allow(
                clippy::cast_precision_loss,
                reason = "sample sizes in Phase 1 are well below f64 exact-integer range"
            )]
            let num = self.wins as f64;
            #[allow(
                clippy::cast_precision_loss,
                reason = "sample sizes in Phase 1 are well below f64 exact-integer range"
            )]
            let den = self.games_played as f64;
            num / den
        }
    }
}

/// One row per registered agent name, summarising play counts, wins,
/// and mean score. Ordered alphabetically by agent name for stable
/// reports.
///
/// # Errors
/// Returns a `rusqlite::Error` if the query or row-mapping fails.
pub fn agent_summaries(conn: &Connection) -> Result<Vec<AgentSummary>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT agent_name, \
                COUNT(*)         AS games_played, \
                SUM(won)         AS wins, \
                AVG(CAST(score AS REAL)) AS avg_score \
         FROM agent_stats \
         GROUP BY agent_name \
         ORDER BY agent_name",
    )?;
    let rows = stmt.query_map([], |row| {
        let agent_name: String = row.get(0)?;
        let games_played: i64 = row.get(1)?;
        let wins: i64 = row.get(2)?;
        let avg_score: Option<f64> = row.get(3)?;
        Ok(AgentSummary {
            agent_name,
            games_played: u64::try_from(games_played).unwrap_or(0),
            wins: u64::try_from(wins).unwrap_or(0),
            avg_score: avg_score.unwrap_or(0.0),
        })
    })?;
    rows.collect()
}

/// Winner distribution: one row per winner slot (player id or "draw"),
/// counting how many games ended with that outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinnerBreakdown {
    pub winner: Option<i64>,
    pub games: u64,
}

/// # Errors
/// Returns a `rusqlite::Error` if the query or row-mapping fails.
pub fn winner_breakdown(conn: &Connection) -> Result<Vec<WinnerBreakdown>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT winner, COUNT(*) AS games \
         FROM games GROUP BY winner ORDER BY winner IS NULL, winner",
    )?;
    let rows = stmt.query_map([], |row| {
        let winner: Option<i64> = row.get(0)?;
        let games: i64 = row.get(1)?;
        Ok(WinnerBreakdown {
            winner,
            games: u64::try_from(games).unwrap_or(0),
        })
    })?;
    rows.collect()
}

/// End-reason distribution: one row per distinct `end_reason` tag,
/// counting how many games ended that way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndReasonBreakdown {
    pub end_reason: String,
    pub games: u64,
}

/// # Errors
/// Returns a `rusqlite::Error` if the query or row-mapping fails.
pub fn end_reason_breakdown(conn: &Connection) -> Result<Vec<EndReasonBreakdown>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT end_reason, COUNT(*) AS games \
         FROM games GROUP BY end_reason ORDER BY end_reason",
    )?;
    let rows = stmt.query_map([], |row| {
        let end_reason: String = row.get(0)?;
        let games: i64 = row.get(1)?;
        Ok(EndReasonBreakdown {
            end_reason,
            games: u64::try_from(games).unwrap_or(0),
        })
    })?;
    rows.collect()
}

/// Mean of a numeric (Scalar/Count/Bool) metric across all games,
/// optionally filtered by player and/or tag. Used by the reporter for
/// "average lead changes per game", "average hand score per player",
/// "kept rate per rank", etc.
///
/// `player = None` restricts to game-scoped metrics (rows with
/// `player IS NULL`). `player = Some(p)` restricts to the specific
/// player. `tag = None` restricts to untagged rows; `tag = Some(...)`
/// restricts to that tag.
///
/// Returns `None` when zero matching rows exist (avoids reporting
/// "average = 0" on empty samples, which would mislead the R1.5 table).
///
/// # Errors
/// Returns a `rusqlite::Error` if the query fails.
pub fn avg_numeric_metric(
    conn: &Connection,
    metric_name: &str,
    player: Option<u8>,
    tag: Option<&str>,
) -> Result<Option<f64>, SqliteError> {
    // -1 is the sentinel for game-scoped rows (see schema.sql).
    let player_val: i64 = player.map_or(-1, i64::from);
    let tag_val = tag.unwrap_or("");
    conn.query_row(
        "SELECT AVG(value_numeric), COUNT(*) FROM game_metrics \
         WHERE metric_name = ?1 AND player = ?2 AND tag = ?3 \
               AND value_numeric IS NOT NULL",
        params![metric_name, player_val, tag_val],
        |row| {
            let avg: Option<f64> = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok(if count == 0 { None } else { avg })
        },
    )
}

/// Sum of a `Count` metric across all games, with the same filtering
/// semantics as [`avg_numeric_metric`]. Used for computing rates that
/// need an explicit numerator (e.g., `kept_count / dealt_count`).
///
/// # Errors
/// Returns a `rusqlite::Error` if the query fails.
pub fn sum_count_metric(
    conn: &Connection,
    metric_name: &str,
    player: Option<u8>,
    tag: Option<&str>,
) -> Result<i64, SqliteError> {
    let player_val: i64 = player.map_or(-1, i64::from);
    let tag_val = tag.unwrap_or("");
    conn.query_row(
        "SELECT COALESCE(SUM(value_numeric), 0) FROM game_metrics \
         WHERE metric_name = ?1 AND player = ?2 AND tag = ?3 \
               AND value_kind = 'count'",
        params![metric_name, player_val, tag_val],
        |row| {
            let sum: f64 = row.get(0)?;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "count metrics sum to well within i64 range for Phase 1 workloads"
            )]
            Ok(sum as i64)
        },
    )
}
