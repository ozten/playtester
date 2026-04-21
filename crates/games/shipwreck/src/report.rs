//! ShipWreck-specific report sections for the `playtest report` command.
//!
//! Reads the SQLite rows that `ingest_directory` wrote via
//! [`crate::metrics::ShipWreckMetrics`]; the report never re-reads JSONL
//! event logs. Parallel shape to `playtest_cribbage::report` — the
//! CLI's `report` command calls one of these per `--game`.

use playtest_metrics::{MarkdownBuilder, avg_numeric_metric, sum_count_metric};
use rusqlite::{Connection, Error as SqliteError};

use crate::metrics::{game_shape, player};

/// Write the **ShipWreck: game shape** section.
///
/// Headlines: average turn count, tie-breaker distribution, average
/// winner raft length & invention count, event-card play rate.
///
/// # Errors
/// Propagates any `rusqlite` error from the underlying queries.
pub fn write_game_shape_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("ShipWreck: game shape");

    let tb_breakdown = tie_breaker_breakdown(conn)?;
    if tb_breakdown.is_empty() {
        md.paragraph("*No ShipWreck games ingested.*");
        return Ok(());
    }

    md.h3("Tie-breaker distribution");
    let total: u64 = tb_breakdown.iter().map(|(_, n)| *n).sum();
    let mut rows = Vec::with_capacity(tb_breakdown.len());
    for (tag, n) in &tb_breakdown {
        rows.push(vec![tag.clone(), n.to_string(), format_share(*n, total)]);
    }
    md.table(&["tie_breaker", "games", "share"], &rows);

    if let Some(avg_turns) = avg_numeric_metric(conn, game_shape::GAME_LENGTH_TURNS, None, None)? {
        md.bullet(&format!("Average game length (turns): **{avg_turns:.1}**"));
    }
    if let Some(avg_raft) = avg_numeric_metric(conn, game_shape::WINNER_RAFT_LENGTH, None, None)? {
        md.bullet(&format!(
            "Average winner raft length: **{avg_raft:.2}** (missing on tied games)"
        ));
    }
    if let Some(avg_inv) = avg_numeric_metric(conn, game_shape::WINNER_EQUIPMENT_COUNT, None, None)?
    {
        md.bullet(&format!("Average winner equipment count: **{avg_inv:.2}**"));
    }
    if let Some(avg_cards) = avg_numeric_metric(conn, game_shape::EVENT_CARDS_PLAYED, None, None)? {
        md.bullet(&format!("Average event cards played: **{avg_cards:.2}**"));
    }
    md.end_block();
    Ok(())
}

/// Write the **ShipWreck: per-player** section: average rescue points,
/// raft length, invention count, and starvation events.
///
/// # Errors
/// Propagates any `rusqlite` error.
pub fn write_per_player_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("ShipWreck: per-player");

    let headers = &[
        "player",
        "avg rescue pts",
        "avg raft len",
        "avg inventions",
        "total starvations",
    ];
    let players = observed_players(conn)?;
    if players.is_empty() {
        md.paragraph("*No per-player data.*");
        return Ok(());
    }

    let mut rows = Vec::with_capacity(players.len());
    for p in players {
        let rescue = avg_numeric_metric(conn, player::PLAYER_RESCUE_POINTS, Some(p), None)?;
        let raft = avg_numeric_metric(conn, player::PLAYER_RAFT_LENGTH, Some(p), None)?;
        let inv = avg_numeric_metric(conn, player::PLAYER_INVENTION_COUNT, Some(p), None)?;
        let starve =
            sum_count_metric(conn, player::PLAYER_FOOD_STARVATION_EVENTS, Some(p), None)?;
        rows.push(vec![
            format!("player_{p}"),
            fmt_opt(rescue, 2),
            fmt_opt(raft, 2),
            fmt_opt(inv, 2),
            starve.to_string(),
        ]);
    }
    md.table(headers, &rows);
    Ok(())
}

/// Group `tie_breaker_used` tag values across all ingested games.
fn tie_breaker_breakdown(conn: &Connection) -> Result<Vec<(String, u64)>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT value_text, COUNT(*) FROM game_metrics \
         WHERE metric_name = ?1 AND player = -1 AND value_kind = 'tag' \
         GROUP BY value_text ORDER BY value_text",
    )?;
    let rows = stmt.query_map([game_shape::TIE_BREAKER_USED], |row| {
        let tag: String = row.get(0)?;
        let n: i64 = row.get(1)?;
        Ok((tag, u64::try_from(n).unwrap_or(0)))
    })?;
    rows.collect()
}

/// Which seat indices appeared in at least one per-player row.
fn observed_players(conn: &Connection) -> Result<Vec<u8>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT player FROM game_metrics \
         WHERE player >= 0 AND metric_name = ?1 ORDER BY player",
    )?;
    let rows = stmt.query_map([player::PLAYER_RESCUE_POINTS], |row| {
        let p: i64 = row.get(0)?;
        Ok(u8::try_from(p).unwrap_or(u8::MAX))
    })?;
    rows.collect()
}

fn format_share(games: u64, total: u64) -> String {
    if total == 0 {
        "-".into()
    } else {
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let num = games as f64;
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let den = total as f64;
        format!("{:.1}%", (num / den) * 100.0)
    }
}

fn fmt_opt(v: Option<f64>, precision: usize) -> String {
    v.map_or_else(|| "-".into(), |x| format!("{x:.precision$}"))
}
